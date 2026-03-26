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
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA foreign_keys=ON;

         CREATE TABLE IF NOT EXISTS commands (
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
             ON commands(session_id);",
    )?;
    Ok(())
}

pub fn insert(conn: &Connection, r: &CommandRecord) -> Result<i64> {
    conn.execute(
        "INSERT INTO commands
         (timestamp_unix, timestamp_iso, command, cwd, exit_code,
          duration_ms, shell, user, hostname, session_id)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        params![
            r.timestamp_unix,
            r.timestamp_iso,
            r.command,
            r.cwd,
            r.exit_code,
            r.duration_ms,
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
                exit_code, duration_ms, shell, user, hostname, session_id
         FROM commands
         ORDER BY timestamp_unix DESC
         LIMIT ?1",
    )?;
    rows_to_records(&mut stmt, params![limit as i64])
}

pub fn search(conn: &Connection, pattern: &str, limit: usize) -> Result<Vec<CommandRecord>> {
    let like = format!("%{}%", pattern);
    let mut stmt = conn.prepare(
        "SELECT id, timestamp_unix, timestamp_iso, command, cwd,
                exit_code, duration_ms, shell, user, hostname, session_id
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
                exit_code, duration_ms, shell, user, hostname, session_id
         FROM commands
         ORDER BY timestamp_unix ASC",
    )?;
    rows_to_records(&mut stmt, params![])
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

fn rows_to_records(
    stmt: &mut rusqlite::Statement,
    params: impl rusqlite::Params,
) -> Result<Vec<CommandRecord>> {
    let rows = stmt.query_map(params, |row| {
        Ok(CommandRecord {
            id: Some(row.get(0)?),
            timestamp_unix: row.get(1)?,
            timestamp_iso: row.get(2)?,
            command: row.get(3)?,
            cwd: row.get(4)?,
            exit_code: row.get(5)?,
            duration_ms: row.get(6)?,
            shell: row.get(7)?,
            user: row.get(8)?,
            hostname: row.get(9)?,
            session_id: row.get(10)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}
