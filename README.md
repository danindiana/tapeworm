<img src="https://github.com/user-attachments/assets/f56a6a11-b529-41c8-b95f-0223bb2af47c" width="30%" alt="gemini-svg (3)">

# tapeworm

**Terminal Activity & Process Execution Workflow Observer/Recorder**

A fast, structured shell history recorder written in Rust. Every command you run is persisted to a local SQLite database with full metadata, parsed into pipeline steps, and made available for security analysis, workflow visualization, and behavioral classification.

```
recording → redaction → pipeline parsing → analysis
```

Unlike `~/.zsh_history`, tapeworm gives you a queryable, non-lossy, security-aware record of your terminal workflows—with credential scrubbing before storage, forward taint analysis, tool-transition graphs, and session archetype classification.

---

## Three-pillar architecture

| Pillar | What it does |
|--------|-------------|
| **Security** | Redact credentials at record time; trace their flow through pipelines |
| **Observability** | Parse pipelines into steps; visualize tool-to-tool transition topology |
| **Intelligence** | Classify sessions by behavioral signature; search history semantically |

---

## Features

**Recording**
- Every command: ISO timestamp, unix epoch, CWD, exit code, duration (ms), gap (ms), shell, user, hostname, session UUID
- Non-blocking: disowned background subprocess (`&!`) — zero perceptible prompt latency
- Survives concurrent terminal windows (SQLite WAL mode)
- JSON and CSV export

**Security**
- Credential redaction before storage: `--token`, `--password`, `-p`, `API_KEY=`, and 15+ other patterns
- Tokenizer handles `$()` subshells, backtick substitution, quoted spans, and concatenated short flags (`-ps3cr3t`)
- Forward taint analysis: traces credential flow through `|` pipelines — detects when secrets reach network, file, or process sinks
- ResponseSink: structural detection of authenticated responses written to disk (without taint propagation)
- MCP server enforces read-only access at SQLite driver level (two-layer enforcement)

**Observability**
- Every command parsed into pipeline steps at record time
- Tool-to-tool transition graph with edge weights and connector types (`|`, `&&`, `||`, `;`)
- Graphviz DOT export for visualization
- Tool frequency analysis, pipeline pattern bigrams

**Intelligence**
- Session archetype classification: Burst / Debugging / Focused / Exploratory
- Inter-command gap timing: idle + think time between commands
- Ollama semantic search over command history
- Failure chain analysis: what did you run immediately after something broke?

---

## Installation

Requires Rust (stable, 2021 edition or later).

```bash
git clone https://github.com/danindiana/tapeworm
cd tapeworm
cargo build --release
sudo cp target/release/tapeworm /usr/local/bin/tapeworm
```

> **Note:** The compiler requires 64 MB stack during build due to `clap_builder`'s
> MIR inliner depth. `.cargo/config.toml` sets `RUST_MIN_STACK=67108864`
> automatically — no manual steps needed.

### Shell integration

**zsh** — add to `~/.zshrc`:
```zsh
eval "$(tapeworm init --shell zsh)"
```

**bash** — add to `~/.bashrc`:
```bash
eval "$(tapeworm init --shell bash)"
```

Start a new shell session. Recording begins immediately.

---

## Command reference

| Command | Description |
|---------|-------------|
| `init [--shell zsh\|bash] [--auto-embed]` | Print shell hook snippet for eval |
| `session-id` | Generate a new session UUID (used internally) |
| `record --cmd CMD --cwd DIR --exit N --duration N --gap N --session S [--embed]` | Write one record (called by hooks) |
| `log [-l N] [--since DURATION] [--today] [--session ID] [--failures]` | Recent command history |
| `search PATTERN [-l N] [--since DURATION] [--today]` | Substring search |
| `export [--format json\|csv]` | Dump all records to stdout |
| `stats` | Top commands + hourly activity chart |
| `tools [-l N]` | Top tools by pipeline-step frequency |
| `pipes [-l N]` | Top pipeline patterns and pipe bigrams |
| `graph [--dot] [--min-weight N] [--edge-type all\|pipe\|seq] [-l N]` | Tool transition graph |
| `taint [--all]` | Forward taint analysis: credential flow through pipelines |
| `session list [-l N]` | Recent sessions with summary stats |
| `session show SESSION_ID` | Full timeline for one session (with gap column) |
| `session failures [-l N]` | Failure chains: failed command → next command pairs |
| `session archetype [-l N] [--explain SESSION_ID]` | Classify sessions by behavioral archetype; `--explain` shows decision path for one session |
| `embed [--model MODEL] [--url URL] [-l N]` | Generate Ollama embeddings |
| `semantic QUERY [-l N] [--model MODEL] [--url URL]` | Natural language similarity search |
| `config` | Show config path and values |
| `config validate` | Validate config file and show migration ledger |
| `db-path` | Print path to the SQLite database |

### Common examples

```bash
# View the last 100 commands
tapeworm log -l 100

# Commands from the last 2 hours
tapeworm log --since 2h

# Only failed commands
tapeworm log --failures
tapeworm log --failures --since 1d

# Search within a time window
tapeworm search git --since 2h
tapeworm search "cargo build" --today

# Show a session's timeline (with gap timing)
tapeworm session list
tapeworm session show <8-char-prefix>

# What tools do I reach for most?
tapeworm tools

# Tool-to-tool transition graph in the terminal
tapeworm graph

# Same graph as a PNG (requires graphviz)
tapeworm graph --dot | dot -Tpng -o graph.png

# Credential flow analysis
tapeworm taint
tapeworm taint --all      # also show clean steps

# How did my sessions behave?
tapeworm session archetype

# Why was session abc12345 classified as Debugging?
tapeworm session archetype --explain abc12345

# Natural language history search
tapeworm semantic "debug memory leak"
tapeworm semantic "GPU memory status" -l 5
```

---

## Security

### Credential redaction

tapeworm scrubs credential-like values before storing any command. The tokenizer handles the full range of shell credential patterns:

| Pattern | Example | Stored as |
|---------|---------|-----------|
| Flag + space + value | `mysql -p s3cr3t` | `mysql -p <REDACTED>` |
| Flag=value | `curl --token=abc` | `curl --token=<REDACTED>` |
| Concatenated short flag | `mysql -ps3cr3t` | `mysql -p<REDACTED>` |
| Env-var assignment | `API_KEY=xyz ./deploy.sh` | `API_KEY=<REDACTED> ./deploy.sh` |
| `$()` subshell value | `curl --token $(cat /tmp/tok)` | `curl --token <REDACTED>` |
| Backtick substitution | `curl --token \`cat /tmp/tok\`` | `curl --token <REDACTED>` |

Covered flags: `--password`, `--passwd`, `-p`, `--token`, `--secret`, `--secret-key`, `--secret-access-key`, `--api-key`, `--apikey`, `--auth`, `--auth-token`, `--bearer`, `--private-key`, `--signing-key`, `--access-key`, `--client-secret`.

Env-var substrings: `PASSWORD`, `PASSWD`, `SECRET`, `TOKEN`, `API_KEY`, `AUTH`, `BEARER`, `PRIVATE_KEY`, `ACCESS_KEY`, `SIGNING_KEY`, `CLIENT_SECRET`.

**Known limitation:** `-p` is overloaded — `ssh -p 22 user@host` becomes `ssh -p <REDACTED> user@host`. tapeworm errs toward redaction.

**Shell-side mitigation:** prefix with a space and set `HISTIGNORE=" *"`, or use `read -rs TOKEN` to avoid secrets appearing on the command line at all.

---

### Forward taint analysis

`tapeworm taint` scans your recorded pipelines for commands containing `<REDACTED>` and traces where credentials can reach via `|` chains.

```
=== taint analysis: credential flow ===
3 pipelines  2 ⚠  sinks reached

[2026-03-26T18:00:00] curl --token <REDACTED> https://api.example.com | jq .result | tee out.json
   0  curl   [CREDENTIAL-USE  ]  secret sent to network; stdout = server response
   2  tee    [RESPONSE-SINK ⚠ ]  authenticated response written to disk

[2026-03-26T18:01:00] echo <REDACTED> | base64 | curl -d @- http://host
   0  echo   [TAINT-SOURCE    ]  stdout carries secret
   1  base64 [PROPAGATED      ]
   2  curl   [NETWORK-SINK ⚠  ]  tainted data sent to external endpoint
```

**Taint labels:**

| Label | Colour | Meaning |
|-------|--------|---------|
| `TAINT-SOURCE` | Yellow | Step has `<REDACTED>`; stdout may carry secret downstream |
| `CREDENTIAL-USE` | Yellow | Step has `<REDACTED>`; tool sends it as credential (curl, ssh, mysql…); stdout = response |
| `PROPAGATED` | Dim | Receives tainted stdin via `\|`; passes it through |
| `NETWORK-SINK` | Red | Receives tainted stdin; sends it to external network |
| `FILE-SINK` | Red | Receives tainted stdin; writes it to disk |
| `PROCESS-SINK` | Red | Receives tainted stdin; spawns subprocesses with it as args |
| `DISCARDED` | Green | Receives tainted stdin; output is metadata — taint terminates (wc, sha256sum…) |
| `RESPONSE-SINK` | Orange | Clean step that writes to file following a `CREDENTIAL-USE` via `\|`; authenticated response persisted |

**Propagation rules:**
- Only `|` carries taint (stdout → stdin). `&&`, `||`, `;` are control-flow — they do not propagate.
- `CredentialUse` tools (curl, wget, ssh, mysql…) consume the credential as an argument; their stdout is the server response, not the secret — taint does not propagate downstream.
- Unknown tools default to Passthrough (sound over-approximation: no missed paths).

**Tool taxonomy:** Passthrough (grep, awk, sed, jq, tee, base64…) / Discard (wc, sha256sum, diff…) / NetworkSink (curl, wget, nc, ssh, mysql…) / FileSink (dd, cp, rsync…) / ProcessSink (xargs, parallel).

---

### MCP read-only enforcement

The `tapeworm-history` MCP server enforces two layers:

1. **SQLite URI mode:** connection opened as `file:...?mode=ro` — OS-level flag that physically prevents writes.
2. **Query validator:** rejects anything that doesn't begin with `SELECT` or `WITH`; blocks `INSERT`, `UPDATE`, `DELETE`, `DROP`, `CREATE`, `ALTER`, `ATTACH`, `PRAGMA`, `VACUUM`.

Claude Code cannot modify your history through the MCP interface.

---

## Observability

### Tool transition graph

`tapeworm graph` queries the `pipeline_steps` corpus for weighted directed edges between consecutive tools — which tools flow into which, and how often.

```bash
# Terminal table (min-weight 2, all connector types)
tapeworm graph

# Pipe-only edges, higher noise threshold
tapeworm graph --edge-type pipe --min-weight 3

# Graphviz DOT — pipe to dot for rendering
tapeworm graph --dot | dot -Tpng -o graph.png
tapeworm graph --dot | dot -Tsvg -o graph.svg
```

The DOT output uses dark-theme styling with edge width proportional to weight and colour-coded by connector type (green = `|`, cyan = `&&`, orange = `||`, grey = `;`).

**Example output:**
```
=== tool transition graph ===
 #  Weight  From   Via  To
 1  4       cargo  |    grep
 2  4       grep   |    sort
 3  3       sort   |    uniq
 4  2       find   |    xargs
```

The terminal graph exposes the structural signature of your shell usage: `grep` as a hub means your workflows are filter-heavy; a strong `sort | uniq | head` chain indicates frequency analysis patterns.

---

### Pipeline composition analysis

Every recorded command is parsed into pipeline steps at record time and stored in `pipeline_steps`. The parser correctly handles:

- Single and double quotes (operators inside quotes are literal)
- Backslash escapes
- `$(...)` subshell expansions
- Bare `(...)` subshell groupings: `(cd /tmp && ls) | grep foo`

Tool names are extracted by stripping env-var assignments, wrapper commands (`sudo`, `env`, `time`, `nice`, `nohup`, `watch`), wrapper flags, and path prefixes.

```bash
# Most-used tools across all pipeline positions
tapeworm tools

# Recurring full pipeline patterns + most common pipe bigrams
tapeworm pipes
```

---

## Intelligence

### Session archetypes

`tapeworm session archetype` classifies each recorded session by behavioral signature using three signal families:

| Signal | Feature | What it captures |
|--------|---------|-----------------|
| Timing rhythm | `mean_gap_ms`, `gap_cv` | Cadence between commands |
| Error pattern | `failure_rate` | Fraction of non-zero exits |
| Tool variety | `tool_entropy` (normalised Shannon) | Breadth of tool usage |

**Archetypes:**

| Archetype | Colour | Trigger |
|-----------|--------|---------|
| `unknown` | Grey | < 3 commands; insufficient data |
| `burst` | Cyan | `mean_gap` < 2 s AND ≥ 5 commands — scripted, muscle memory |
| `debugging` | Red | `failure_rate` > 35% — error/fix/retry cycle |
| `focused` | Green | `tool_entropy` < 0.45 — narrow toolset, single deep task |
| `exploratory` | Yellow | `tool_entropy` ≥ 0.45 — varied tools, open-ended work |

Sessions are also flagged `⚠ interrupted` when any single gap exceeds 5 minutes (orthogonal to the primary archetype).

```bash
tapeworm session archetype
tapeworm session archetype -l 50

# Show which features drove a specific session's classification
tapeworm session archetype --explain a3f8c2d1
```

When ≥ 5 sessions are present the table also computes a population baseline (mean ± σ) for each feature. Up to 500 sessions are sampled for the baseline calculation, keeping z-scores stable regardless of the display `--limit`. Sessions more than 2σ from the mean receive deviation indicators in the flags column: `↑fail` (high failure rate), `↓gap` (unusually fast), `↑ent` (high entropy), and so on. A legend line notes when baseline is active.

The `--explain` view shows each classification gate with its feature value, the threshold, and whether that gate fired:

```
=== archetype explain: a3f8c2d1

Features:
  cmd_count        20
  failure_rate     5.0%
  mean_gap         -  (max: 0ms)
  tool_entropy     0.868

Decision path:
  ✓ cmd_count ≥ 3?           20 ≥ 3  →  continue
  ✗ failure_rate > 35%?       5.0% > 35%  →  skip
  ✗ gap < 2s AND cmds ≥ 5?   no gap data ✗  AND  20 ≥ 5 ✓  →  skip
  ✗ entropy > 0 AND < 0.45?  0.868 < 0.45  →  skip
  ✓ entropy ≥ 0.45?           0.868 ≥ 0.45  →  EXPLORATORY  ◀

  Classification: EXPLORATORY
```

---

### Inter-command gap timing

The shell hook records `gap_ms` — time elapsed from when the previous command finished to when you submitted the next one. This is idle + think time: reading output, editing files, copy-pasting.

`tapeworm session show` displays a `gap` column colour-coded by magnitude:
- **Dim** (< 60 s): normal flow
- **Yellow** (60–300 s): long pause — reading docs or thinking
- **Red** (> 300 s): interrupted session — stepped away

`tapeworm session show` also appends a gap distribution histogram when any gap data is present — six buckets from `<1s` to `>5m` with a bar scaled to the largest bucket:

```
  gap distribution:
     <1s    0
    1-5s    3  ▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪
   5-30s    1  ▪▪▪▪▪▪▪▪
  30-60s    0
    1-5m    1  ▪▪▪▪▪▪▪▪
     >5m    0
```

Gap data accumulates starting from the first shell opened after installing the updated hook. Pre-existing records show `0`.

`tapeworm session show` also prints a compact archetype summary line at the bottom:

```
  archetype focused   fail 0%   gap̄ 4s   entropy 0.31
```

---

### Semantic search (Ollama)

tapeworm can embed every command with a local Ollama model and enable natural language retrieval.

```bash
# Pull an embedding model (one-time)
ollama pull nomic-embed-text

# Embed all recorded commands
tapeworm embed

# Natural language queries
tapeworm semantic "debug memory leak"
tapeworm semantic "rust compilation failed"
tapeworm semantic "GPU memory status" -l 5
```

Each command is embedded as `"shell command: {cmd} | directory: {cwd}"` — CWD inclusion makes the embedding project-aware. Embeddings are stored as packed `f32` BLOBs; cosine similarity is computed in memory at query time.

Results are colour-coded: **green** ≥ 80%, **yellow** 60–79%, **grey** < 60%.

With `auto_embed = true` in config (or `tapeworm init --auto-embed`), every recorded command is embedded inline at record time. Ollama unavailability is silently skipped — the hook never fails.

**Upgrade path:** for 100k+ commands, swap the in-memory cosine search for [`sqlite-vec`](https://github.com/asg017/sqlite-vec). The BLOB schema is forward-compatible.

---

### Session intelligence

Every shell process generates a fresh UUID4 stored in `$TAPEWORM_SESSION`.

```bash
# List recent sessions
tapeworm session list

# Full timeline for one session (8-char prefix)
tapeworm session show a3f8c2d1

# What did you run immediately after something broke?
tapeworm session failures

# Behavioral classification
tapeworm session archetype
```

---

## Configuration

Config file: `~/.config/tapeworm/config.toml` (created on first `tapeworm config` run).

```toml
[ollama]
url        = "http://localhost:11434"
model      = "nomic-embed-text"
auto_embed = false

[display]
log_limit = 50
```

---

## Database

```
~/.local/share/tapeworm/history.db
```

Schema:

```sql
CREATE TABLE commands (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp_unix INTEGER NOT NULL,
    timestamp_iso  TEXT    NOT NULL,   -- RFC 3339
    command        TEXT    NOT NULL,   -- after credential redaction
    cwd            TEXT    NOT NULL,
    exit_code      INTEGER NOT NULL DEFAULT 0,
    duration_ms    INTEGER NOT NULL DEFAULT 0,
    gap_ms         INTEGER NOT NULL DEFAULT 0,  -- idle + think time since prev command
    shell          TEXT    NOT NULL DEFAULT 'unknown',
    user           TEXT    NOT NULL DEFAULT '',
    hostname       TEXT    NOT NULL DEFAULT '',
    session_id     TEXT    NOT NULL DEFAULT ''  -- UUID v4 per shell process
);

-- One row per pipeline step within a command.
CREATE TABLE pipeline_steps (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    command_id   INTEGER NOT NULL REFERENCES commands(id) ON DELETE CASCADE,
    step_index   INTEGER NOT NULL,   -- 0-based position in pipeline
    tool         TEXT    NOT NULL,   -- extracted tool name
    raw          TEXT    NOT NULL,   -- full text of this step (post-redaction)
    connector    TEXT    NOT NULL DEFAULT ''  -- |  &&  ||  ;  or "" for last step
);

-- Ollama embeddings for semantic search.
CREATE TABLE command_embeddings (
    command_id  INTEGER PRIMARY KEY REFERENCES commands(id) ON DELETE CASCADE,
    model       TEXT    NOT NULL,
    embedding   BLOB    NOT NULL   -- packed little-endian f32 array
);
```

Schema migrations are tracked in a `schema_versions` ledger table (version, applied_at timestamp). The migration runner checks this table before each migration and skips already-applied versions. Existing databases are bootstrapped on first open: the column-existence check prevents duplicate-column errors on the `gap_ms` migration.

```bash
# Show which schema versions are applied and when
tapeworm config validate
```

Current versions: v1 = base schema, v2 = gap_ms column.

**Direct SQL access:**
```bash
sqlite3 ~/.local/share/tapeworm/history.db

-- Commands where step 0 is git but the pipeline failed
SELECT c.command, c.exit_code
FROM commands c
JOIN pipeline_steps p ON p.command_id = c.id AND p.step_index = 0
WHERE p.tool = 'git' AND c.exit_code != 0;

-- Tools you use after grep
SELECT b.tool, COUNT(*) as cnt
FROM pipeline_steps a JOIN pipeline_steps b
  ON b.command_id = a.command_id AND b.step_index = a.step_index + 1
WHERE a.tool = 'grep' AND a.connector = '|'
GROUP BY b.tool ORDER BY cnt DESC;

-- Sessions with above-average failure rates
SELECT session_id,
       ROUND(100.0 * SUM(exit_code != 0) / COUNT(*), 1) || '%' as fail_rate,
       COUNT(*) as cmds
FROM commands WHERE session_id != ''
GROUP BY session_id
HAVING SUM(exit_code != 0) > 0
ORDER BY fail_rate DESC;
```

---

## MCP integration (Claude Code)

Two MCP servers ship with tapeworm, scoped to the project directory via `.claude.json`.

```bash
# Ollama MCP — list models, generate, embed, pull, etc.
claude mcp add ollama-rawveg \
  --transport stdio \
  --env "OLLAMA_HOST=http://localhost:11434" \
  -- npx -y ollama-mcp

# Read-only tapeworm history MCP
pip install mcp
claude mcp add tapeworm-history --transport stdio -- python scripts/tapeworm_mcp.py

claude mcp list
# ollama-rawveg:     ✓ Connected
# tapeworm-history:  ✓ Connected
```

With these active, Claude Code can query your shell history in natural language, cross-reference it with code you're editing, and run embedding operations — all through the MCP tool interface. The history server is physically read-only at the SQLite driver level.

---

## Dependencies

| Crate | Purpose |
|-------|---------|
| `clap` v4 | CLI (derive API) |
| `rusqlite` v0.31 (bundled) | SQLite — compiled in, no system dependency |
| `chrono` v0.4 | Timestamps and RFC 3339 |
| `serde` / `serde_json` | JSON serialization |
| `uuid` v1 | Session UUID generation |
| `comfy-table` v7 | Terminal tables |
| `colored` v2 | Terminal colour output |
| `csv` v1 | CSV export |
| `dirs` v5 | XDG data directory resolution |
| `anyhow` v1 | Error propagation |
| `hostname` v0.4 | Hostname resolution |
| `reqwest` v0.12 (blocking) | Ollama HTTP client |
| `toml` v0.8 | Config deserialization |

---

## License

MIT
