use crate::record::CommandRecord;
use anyhow::Result;
use rusqlite::{params, Connection};
use std::path::PathBuf;

pub fn db_path() -> PathBuf {
    let mut p = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from(format!(
            "{}/.local/share",
            std::env::var("HOME").unwrap_or_default()
        )));
    p.push("tapeworm");
    p.push("history.db");
    p
}

pub fn open() -> Result<Connection> {
    let path = db_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let conn = Connection::open(&path)?;
    migrate(&conn)?;
    Ok(conn)
}

fn migrate(conn: &Connection) -> Result<()> {
    // Migration ledger: each applied version is recorded with a timestamp.
    // Must be created before any migration check.
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA foreign_keys=ON;
         CREATE TABLE IF NOT EXISTS schema_versions (
             version    INTEGER PRIMARY KEY,
             applied_at TEXT    NOT NULL
         );",
    )?;

    // v1: base schema
    apply_migration(conn, 1, || {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS commands (
                 id             INTEGER PRIMARY KEY AUTOINCREMENT,
                 timestamp_unix INTEGER NOT NULL,
                 timestamp_iso  TEXT    NOT NULL,
                 command        TEXT    NOT NULL,
                 cwd            TEXT    NOT NULL,
                 exit_code      INTEGER NOT NULL DEFAULT 0,
                 duration_ms    INTEGER NOT NULL DEFAULT 0,
                 shell          TEXT    NOT NULL DEFAULT 'unknown',
                 user           TEXT    NOT NULL DEFAULT '',
                 hostname       TEXT    NOT NULL DEFAULT '',
                 session_id     TEXT    NOT NULL DEFAULT ''
             );
             CREATE INDEX IF NOT EXISTS idx_commands_ts
                 ON commands(timestamp_unix DESC);
             CREATE INDEX IF NOT EXISTS idx_commands_cmd
                 ON commands(command);
             CREATE INDEX IF NOT EXISTS idx_commands_session
                 ON commands(session_id);

             CREATE TABLE IF NOT EXISTS pipeline_steps (
                 id           INTEGER PRIMARY KEY AUTOINCREMENT,
                 command_id   INTEGER NOT NULL REFERENCES commands(id) ON DELETE CASCADE,
                 step_index   INTEGER NOT NULL,
                 tool         TEXT    NOT NULL,
                 raw          TEXT    NOT NULL,
                 connector    TEXT    NOT NULL DEFAULT ''
             );
             CREATE INDEX IF NOT EXISTS idx_ps_command_id ON pipeline_steps(command_id);
             CREATE INDEX IF NOT EXISTS idx_ps_tool       ON pipeline_steps(tool);

             CREATE TABLE IF NOT EXISTS command_embeddings (
                 command_id  INTEGER PRIMARY KEY REFERENCES commands(id) ON DELETE CASCADE,
                 model       TEXT    NOT NULL,
                 embedding   BLOB    NOT NULL
             );",
        )?;
        Ok(())
    })?;

    // v2: gap_ms column on commands.
    // Bootstrap guard: existing DBs that already have gap_ms (added before the ledger)
    // skip the ALTER rather than failing with "duplicate column".
    apply_migration(conn, 2, || {
        let has_col: bool = conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('commands') WHERE name = 'gap_ms'",
            [],
            |r| r.get::<_, i64>(0),
        ).map(|n| n > 0).unwrap_or(false);
        if !has_col {
            conn.execute(
                "ALTER TABLE commands ADD COLUMN gap_ms INTEGER NOT NULL DEFAULT 0",
                [],
            )?;
        }
        Ok(())
    })?;

    Ok(())
}

/// Run a migration if it has not been recorded in schema_versions.
/// On success, inserts the version into the ledger.
/// If the migration closure fails, the error propagates and the version is NOT recorded.
fn apply_migration<F>(conn: &Connection, version: i64, migration: F) -> Result<()>
where
    F: FnOnce() -> Result<()>,
{
    let already_applied: bool = conn.query_row(
        "SELECT COUNT(*) FROM schema_versions WHERE version = ?1",
        params![version],
        |r| r.get::<_, i64>(0),
    ).map(|n| n > 0).unwrap_or(false);

    if already_applied {
        return Ok(());
    }

    migration()?;

    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT OR IGNORE INTO schema_versions (version, applied_at) VALUES (?1, ?2)",
        params![version, now],
    )?;
    Ok(())
}

pub fn insert(conn: &Connection, r: &CommandRecord) -> Result<i64> {
    conn.execute(
        "INSERT INTO commands
         (timestamp_unix, timestamp_iso, command, cwd, exit_code,
          duration_ms, gap_ms, shell, user, hostname, session_id)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
        params![
            r.timestamp_unix,
            r.timestamp_iso,
            r.command,
            r.cwd,
            r.exit_code,
            r.duration_ms,
            r.gap_ms,
            r.shell,
            r.user,
            r.hostname,
            r.session_id,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn recent(conn: &Connection, limit: usize) -> Result<Vec<CommandRecord>> {
    let mut stmt = conn.prepare(
        "SELECT id, timestamp_unix, timestamp_iso, command, cwd,
                exit_code, duration_ms, gap_ms, shell, user, hostname, session_id
         FROM commands
         ORDER BY timestamp_unix DESC
         LIMIT ?1",
    )?;
    rows_to_records(&mut stmt, params![limit as i64])
}

pub fn recent_since(conn: &Connection, since_unix: i64, limit: usize) -> Result<Vec<CommandRecord>> {
    let mut stmt = conn.prepare(
        "SELECT id, timestamp_unix, timestamp_iso, command, cwd,
                exit_code, duration_ms, gap_ms, shell, user, hostname, session_id
         FROM commands
         WHERE timestamp_unix >= ?1
         ORDER BY timestamp_unix DESC
         LIMIT ?2",
    )?;
    rows_to_records(&mut stmt, params![since_unix, limit as i64])
}

pub fn recent_in_session(conn: &Connection, session_id: &str, limit: usize) -> Result<Vec<CommandRecord>> {
    // Support prefix matching so the user can pass the 8-char truncated ID from `session list`
    let pattern = format!("{}%", session_id);
    let mut stmt = conn.prepare(
        "SELECT id, timestamp_unix, timestamp_iso, command, cwd,
                exit_code, duration_ms, gap_ms, shell, user, hostname, session_id
         FROM commands
         WHERE session_id LIKE ?1
         ORDER BY timestamp_unix ASC
         LIMIT ?2",
    )?;
    rows_to_records(&mut stmt, params![pattern, limit as i64])
}

pub fn search(conn: &Connection, pattern: &str, limit: usize) -> Result<Vec<CommandRecord>> {
    let like = format!("%{}%", pattern);
    let mut stmt = conn.prepare(
        "SELECT id, timestamp_unix, timestamp_iso, command, cwd,
                exit_code, duration_ms, gap_ms, shell, user, hostname, session_id
         FROM commands
         WHERE command LIKE ?1
         ORDER BY timestamp_unix DESC
         LIMIT ?2",
    )?;
    rows_to_records(&mut stmt, params![like, limit as i64])
}

pub fn all(conn: &Connection) -> Result<Vec<CommandRecord>> {
    let mut stmt = conn.prepare(
        "SELECT id, timestamp_unix, timestamp_iso, command, cwd,
                exit_code, duration_ms, gap_ms, shell, user, hostname, session_id
         FROM commands
         ORDER BY timestamp_unix ASC",
    )?;
    rows_to_records(&mut stmt, params![])
}

// --- Session queries ---

pub struct SessionSummary {
    pub session_id: String,
    pub start_unix: i64,
    pub end_unix: i64,
    pub cmd_count: i64,
    pub failure_count: i64,
    pub shell: String,
}

pub fn list_sessions(conn: &Connection, limit: usize) -> Result<Vec<SessionSummary>> {
    let mut stmt = conn.prepare(
        "SELECT
             session_id,
             MIN(timestamp_unix) as start_unix,
             MAX(timestamp_unix) as end_unix,
             COUNT(*)            as cmd_count,
             SUM(CASE WHEN exit_code != 0 THEN 1 ELSE 0 END) as failure_count,
             MAX(shell)          as shell
         FROM commands
         WHERE session_id != ''
         GROUP BY session_id
         ORDER BY start_unix DESC
         LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit as i64], |row| {
        Ok(SessionSummary {
            session_id:    row.get(0)?,
            start_unix:    row.get(1)?,
            end_unix:      row.get(2)?,
            cmd_count:     row.get(3)?,
            failure_count: row.get(4)?,
            shell:         row.get(5)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Commands that ran immediately after a non-zero exit within the same session.
/// Returns (failed_cmd, recovery_cmd) pairs ordered by failure time descending.
pub fn failure_chains(conn: &Connection, limit: usize) -> Result<Vec<(CommandRecord, CommandRecord)>> {
    // Find the next command (by time, then id as tiebreaker) in the same session after a failure.
    // Using (timestamp_unix, id) order rather than bare id because the hook records asynchronously
    // (&! / &), so insertion order can diverge from issue order within the same second.
    let mut stmt = conn.prepare(
        "SELECT
             f.id, f.timestamp_unix, f.timestamp_iso, f.command, f.cwd,
             f.exit_code, f.duration_ms, f.gap_ms, f.shell, f.user, f.hostname, f.session_id,
             r.id, r.timestamp_unix, r.timestamp_iso, r.command, r.cwd,
             r.exit_code, r.duration_ms, r.gap_ms, r.shell, r.user, r.hostname, r.session_id
         FROM commands f
         JOIN commands r
           ON r.session_id = f.session_id
          AND r.id = (
              SELECT id FROM commands
              WHERE session_id = f.session_id
                AND (timestamp_unix > f.timestamp_unix
                     OR (timestamp_unix = f.timestamp_unix AND id > f.id))
              ORDER BY timestamp_unix ASC, id ASC
              LIMIT 1
          )
         WHERE f.exit_code != 0
           AND f.session_id != ''
         ORDER BY f.timestamp_unix DESC
         LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit as i64], |row| {
        let failed = CommandRecord {
            id:             Some(row.get(0)?),
            timestamp_unix: row.get(1)?,
            timestamp_iso:  row.get(2)?,
            command:        row.get(3)?,
            cwd:            row.get(4)?,
            exit_code:      row.get(5)?,
            duration_ms:    row.get(6)?,
            gap_ms:         row.get(7)?,
            shell:          row.get(8)?,
            user:           row.get(9)?,
            hostname:       row.get(10)?,
            session_id:     row.get(11)?,
        };
        let recovery = CommandRecord {
            id:             Some(row.get(12)?),
            timestamp_unix: row.get(13)?,
            timestamp_iso:  row.get(14)?,
            command:        row.get(15)?,
            cwd:            row.get(16)?,
            exit_code:      row.get(17)?,
            duration_ms:    row.get(18)?,
            gap_ms:         row.get(19)?,
            shell:          row.get(20)?,
            user:           row.get(21)?,
            hostname:       row.get(22)?,
            session_id:     row.get(23)?,
        };
        Ok((failed, recovery))
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

// --- Archetype feature queries ---

/// Raw per-session stats returned from SQL (no tool entropy yet).
pub struct SessionRawStats {
    pub session_id:   String,
    pub start_unix:   i64,
    pub shell:        String,
    pub cmd_count:    i64,
    pub failure_rate: f64,
    pub mean_gap_ms:  f64,   // 0.0 when all gap_ms are 0
    pub max_gap_ms:   i64,
    pub gap_variance: f64,   // E[X²] - E[X]²; 0 when no gap data
}

/// Fetch per-session summary stats for the N most recent sessions.
pub fn session_raw_stats(conn: &Connection, limit: usize) -> Result<Vec<SessionRawStats>> {
    let mut stmt = conn.prepare(
        "SELECT
             session_id,
             MIN(timestamp_unix) AS start_unix,
             MAX(shell)          AS shell,
             COUNT(*)            AS cmd_count,
             CAST(SUM(CASE WHEN exit_code != 0 THEN 1 ELSE 0 END) AS REAL)
                 / COUNT(*)                                       AS failure_rate,
             COALESCE(AVG(CASE WHEN gap_ms > 0
                               THEN CAST(gap_ms AS REAL) END), 0.0) AS mean_gap_ms,
             MAX(gap_ms)                                          AS max_gap_ms,
             COALESCE(
                 AVG(CAST(gap_ms AS REAL) * CAST(gap_ms AS REAL))
                 - AVG(CAST(gap_ms AS REAL)) * AVG(CAST(gap_ms AS REAL)),
                 0.0)                                             AS gap_variance
         FROM commands
         WHERE session_id != ''
         GROUP BY session_id
         ORDER BY start_unix DESC
         LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit as i64], |row| {
        Ok(SessionRawStats {
            session_id:   row.get(0)?,
            start_unix:   row.get(1)?,
            shell:        row.get(2)?,
            cmd_count:    row.get(3)?,
            failure_rate: row.get(4)?,
            mean_gap_ms:  row.get(5)?,
            max_gap_ms:   row.get(6)?,
            gap_variance: row.get(7)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Fetch (session_id, tool, frequency) triples for a given set of sessions.
/// Returns only sessions that have pipeline_steps; others will have no entry.
pub fn session_tool_freqs(
    conn: &Connection,
    session_ids: &[&str],
) -> Result<std::collections::HashMap<String, std::collections::HashMap<String, i64>>> {
    if session_ids.is_empty() {
        return Ok(std::collections::HashMap::new());
    }
    let placeholders = session_ids.iter().enumerate()
        .map(|(i, _)| format!("?{}", i + 1))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT c.session_id, ps.tool, COUNT(*) AS freq
         FROM commands c
         JOIN pipeline_steps ps ON ps.command_id = c.id
         WHERE c.session_id IN ({}) AND ps.tool != ''
         GROUP BY c.session_id, ps.tool",
        placeholders
    );
    let mut stmt = conn.prepare(&sql)?;
    let params_vec: Vec<&dyn rusqlite::ToSql> = session_ids.iter()
        .map(|s| s as &dyn rusqlite::ToSql)
        .collect();
    let rows = stmt.query_map(params_vec.as_slice(), |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })?;
    let mut out: std::collections::HashMap<String, std::collections::HashMap<String, i64>> =
        std::collections::HashMap::new();
    for row in rows {
        let (sid, tool, freq) = row?;
        out.entry(sid).or_default().insert(tool, freq);
    }
    Ok(out)
}

/// Fetch per-session stats for a single session identified by ID prefix.
/// Returns None if no matching session is found.
pub fn session_raw_stats_one(conn: &Connection, session_prefix: &str) -> Result<Option<SessionRawStats>> {
    let pattern = format!("{}%", session_prefix);
    let mut stmt = conn.prepare(
        "SELECT
             session_id,
             MIN(timestamp_unix) AS start_unix,
             MAX(shell)          AS shell,
             COUNT(*)            AS cmd_count,
             CAST(SUM(CASE WHEN exit_code != 0 THEN 1 ELSE 0 END) AS REAL)
                 / COUNT(*)                                       AS failure_rate,
             COALESCE(AVG(CASE WHEN gap_ms > 0
                               THEN CAST(gap_ms AS REAL) END), 0.0) AS mean_gap_ms,
             MAX(gap_ms)                                          AS max_gap_ms,
             COALESCE(
                 AVG(CAST(gap_ms AS REAL) * CAST(gap_ms AS REAL))
                 - AVG(CAST(gap_ms AS REAL)) * AVG(CAST(gap_ms AS REAL)),
                 0.0)                                             AS gap_variance
         FROM commands
         WHERE session_id LIKE ?1 AND session_id != ''
         GROUP BY session_id
         LIMIT 1",
    )?;
    let result = stmt.query_map(params![pattern], |row| {
        Ok(SessionRawStats {
            session_id:   row.get(0)?,
            start_unix:   row.get(1)?,
            shell:        row.get(2)?,
            cmd_count:    row.get(3)?,
            failure_rate: row.get(4)?,
            mean_gap_ms:  row.get(5)?,
            max_gap_ms:   row.get(6)?,
            gap_variance: row.get(7)?,
        })
    })?.next().transpose()?;
    Ok(result)
}

pub fn top_commands(conn: &Connection, limit: usize) -> Result<Vec<(String, i64)>> {
    let mut stmt = conn.prepare(
        "SELECT command, COUNT(*) as cnt
         FROM commands
         GROUP BY command
         ORDER BY cnt DESC
         LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit as i64], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn hourly_distribution(conn: &Connection) -> Result<Vec<(i64, i64)>> {
    let mut stmt = conn.prepare(
        "SELECT CAST(strftime('%H', datetime(timestamp_unix, 'unixepoch')) AS INTEGER) as hr,
                COUNT(*) as cnt
         FROM commands
         GROUP BY hr
         ORDER BY hr ASC",
    )?;
    let rows = stmt.query_map(params![], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Returns (version, applied_at) pairs from the migration ledger, ascending.
pub fn schema_versions(conn: &Connection) -> Result<Vec<(i64, String)>> {
    let mut stmt = conn.prepare(
        "SELECT version, applied_at FROM schema_versions ORDER BY version ASC",
    )?;
    let rows = stmt.query_map(params![], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn total_count(conn: &Connection) -> Result<i64> {
    Ok(conn.query_row("SELECT COUNT(*) FROM commands", params![], |r| r.get(0))?)
}

pub fn avg_duration(conn: &Connection) -> Result<f64> {
    Ok(conn.query_row(
        "SELECT AVG(duration_ms) FROM commands",
        params![],
        |r| r.get::<_, Option<f64>>(0).map(|v| v.unwrap_or(0.0)),
    )?)
}

pub fn insert_pipeline_steps(
    conn: &Connection,
    command_id: i64,
    steps: &[crate::parse::PipelineStep],
) -> Result<()> {
    let mut stmt = conn.prepare_cached(
        "INSERT INTO pipeline_steps (command_id, step_index, tool, raw, connector)
         VALUES (?1, ?2, ?3, ?4, ?5)",
    )?;
    for step in steps {
        stmt.execute(params![
            command_id,
            step.index as i64,
            step.tool,
            step.raw,
            step.connector,
        ])?;
    }
    Ok(())
}

pub fn top_tools(conn: &Connection, limit: usize) -> Result<Vec<(String, i64)>> {
    let mut stmt = conn.prepare(
        "SELECT tool, COUNT(*) as cnt
         FROM pipeline_steps
         WHERE tool != ''
         GROUP BY tool
         ORDER BY cnt DESC
         LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit as i64], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Top consecutive tool pairs connected by a bare `|`.
pub fn top_bigrams(conn: &Connection, limit: usize) -> Result<Vec<(String, String, i64)>> {
    let mut stmt = conn.prepare(
        "SELECT a.tool, b.tool, COUNT(*) as cnt
         FROM pipeline_steps a
         JOIN pipeline_steps b
           ON b.command_id = a.command_id
          AND b.step_index = a.step_index + 1
         WHERE a.connector = '|'
           AND a.tool != ''
           AND b.tool != ''
         GROUP BY a.tool, b.tool
         ORDER BY cnt DESC
         LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit as i64], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Top full pipeline patterns as `tool1 | tool2 | tool3` strings.
/// Only multi-step pipelines are included. Uses ordered subquery for deterministic
/// GROUP_CONCAT results.
pub fn top_pipelines(conn: &Connection, limit: usize) -> Result<Vec<(String, i64)>> {
    // Each step contributes "tool connector" (e.g. "grep |") or just "tool" for the last step,
    // so GROUP_CONCAT(..., ' ') produces the accurate pattern "grep | awk && sed" rather than
    // collapsing all connectors to ' | '.
    let mut stmt = conn.prepare(
        "SELECT pattern, COUNT(*) as cnt
         FROM (
             SELECT command_id,
                    GROUP_CONCAT(step_str, ' ') as pattern
             FROM (
                 SELECT command_id, step_index,
                        CASE WHEN connector != ''
                             THEN tool || ' ' || connector
                             ELSE tool
                        END as step_str
                 FROM pipeline_steps
                 WHERE tool != ''
                 ORDER BY command_id, step_index
             )
             GROUP BY command_id
             HAVING COUNT(*) > 1
         )
         GROUP BY pattern
         ORDER BY cnt DESC
         LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit as i64], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

// --- Embedding functions ---

pub fn insert_embedding(
    conn: &Connection,
    command_id: i64,
    model: &str,
    embedding: &[f32],
) -> Result<()> {
    let blob = crate::embed::vec_to_blob(embedding);
    conn.execute(
        "INSERT OR REPLACE INTO command_embeddings (command_id, model, embedding)
         VALUES (?1, ?2, ?3)",
        params![command_id, model, blob],
    )?;
    Ok(())
}

/// Returns (command_id, command_text, cwd) for commands without embeddings.
pub fn get_unembedded(conn: &Connection, limit: usize) -> Result<Vec<(i64, String, String)>> {
    let lim = if limit == 0 { i64::MAX } else { limit as i64 };
    let mut stmt = conn.prepare(
        "SELECT id, command, cwd FROM commands
         WHERE id NOT IN (SELECT command_id FROM command_embeddings)
         ORDER BY id ASC
         LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![lim], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Returns count of commands without embeddings.
#[allow(dead_code)]
pub fn unembedded_count(conn: &Connection) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM commands WHERE id NOT IN (SELECT command_id FROM command_embeddings)",
        params![],
        |r| r.get(0),
    )?)
}

/// Load all embeddings for semantic search.
pub fn get_all_embeddings(conn: &Connection) -> Result<Vec<crate::semantic::EmbeddingEntry>> {
    let mut stmt = conn.prepare(
        "SELECT command_id, embedding FROM command_embeddings ORDER BY command_id ASC",
    )?;
    let rows = stmt.query_map(params![], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?))
    })?;
    let entries = rows
        .collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .map(|(id, blob)| crate::semantic::EmbeddingEntry {
            command_id: id,
            embedding: crate::embed::blob_to_vec(&blob),
        })
        .collect();
    Ok(entries)
}

/// Fetch specific command records by their IDs (for displaying search results).
pub fn get_commands_by_ids(conn: &Connection, ids: &[i64]) -> Result<Vec<CommandRecord>> {
    if ids.is_empty() {
        return Ok(vec![]);
    }
    // Build a VALUES list for the IN clause — safe since ids are i64
    let placeholders: String = ids.iter().enumerate()
        .map(|(i, _)| format!("?{}", i + 1))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT id, timestamp_unix, timestamp_iso, command, cwd,
                exit_code, duration_ms, gap_ms, shell, user, hostname, session_id
         FROM commands WHERE id IN ({})",
        placeholders
    );
    let mut stmt = conn.prepare(&sql)?;
    // Build params dynamically
    let params_vec: Vec<&dyn rusqlite::ToSql> = ids.iter()
        .map(|id| id as &dyn rusqlite::ToSql)
        .collect();
    rows_to_records(&mut stmt, params_vec.as_slice())
}

/// One directed edge in the tool transition graph.
pub struct ToolEdge {
    pub from:      String,
    pub to:        String,
    pub connector: String,  // |  &&  ||  ;  or "" for the last step
    pub weight:    i64,
}

/// Returns weighted directed edges between consecutive pipeline tools.
///
/// `edge_filter` narrows by connector type:
///   "pipe" → only `|` edges
///   "seq"  → `&&`, `||`, `;` edges
///   "all"  → everything (default)
///
/// Edges with weight < `min_weight` are excluded.
pub fn tool_transitions(
    conn: &Connection,
    edge_filter: &str,
    min_weight: i64,
    limit: usize,
) -> Result<Vec<ToolEdge>> {
    let connector_clause = match edge_filter {
        "pipe" => "AND a.connector = '|'",
        "seq"  => "AND a.connector IN ('&&', '||', ';')",
        _      => "",
    };
    let sql = format!(
        "SELECT a.tool, b.tool, a.connector, COUNT(*) AS weight
         FROM pipeline_steps a
         JOIN pipeline_steps b
             ON b.command_id = a.command_id
            AND b.step_index = a.step_index + 1
         WHERE a.tool != '' AND b.tool != ''
         {connector_clause}
         GROUP BY a.tool, b.tool, a.connector
         HAVING weight >= ?1
         ORDER BY weight DESC
         LIMIT ?2"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![min_weight, limit as i64], |row| {
        Ok(ToolEdge {
            from:      row.get(0)?,
            to:        row.get(1)?,
            connector: row.get(2)?,
            weight:    row.get(3)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Fetch all pipeline steps for every command that contains at least one
/// `<REDACTED>` token.  Rows are ordered by `(command_id, step_index)` so the
/// caller can group them with a simple linear scan.
pub fn tainted_step_rows(conn: &Connection) -> Result<Vec<crate::taint::StepRow>> {
    let mut stmt = conn.prepare(
        "SELECT c.id, c.command, c.timestamp_iso,
                ps.step_index, ps.tool, ps.raw, ps.connector
         FROM commands c
         JOIN pipeline_steps ps ON ps.command_id = c.id
         WHERE c.id IN (
             SELECT DISTINCT command_id
             FROM pipeline_steps
             WHERE raw LIKE '%<REDACTED>%'
         )
         ORDER BY c.id, ps.step_index",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(crate::taint::StepRow {
            command_id:    row.get(0)?,
            command_text:  row.get(1)?,
            timestamp_iso: row.get(2)?,
            step_index:    row.get(3)?,
            tool:          row.get(4)?,
            raw:           row.get(5)?,
            connector:     row.get(6)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

fn rows_to_records(
    stmt: &mut rusqlite::Statement,
    params: impl rusqlite::Params,
) -> Result<Vec<CommandRecord>> {
    let rows = stmt.query_map(params, |row| {
        Ok(CommandRecord {
            id:             Some(row.get(0)?),
            timestamp_unix: row.get(1)?,
            timestamp_iso:  row.get(2)?,
            command:        row.get(3)?,
            cwd:            row.get(4)?,
            exit_code:      row.get(5)?,
            duration_ms:    row.get(6)?,
            gap_ms:         row.get(7)?,
            shell:          row.get(8)?,
            user:           row.get(9)?,
            hostname:       row.get(10)?,
            session_id:     row.get(11)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}
