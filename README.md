# tapeworm

**Terminal Activity & Process Execution Workflow Observer/Recorder**

A fast, structured shell history recorder written in Rust. Every command you run gets persisted to a local SQLite database with full metadata — timestamp, working directory, exit code, duration, shell, session ID, and more.

Unlike `~/.zsh_history` or `~/.bash_history`, tapeworm gives you a queryable, exportable, non-lossy record of your terminal workflows.

---

## Features

- Records every command with: ISO timestamp, unix epoch, CWD, exit code, duration (ms), shell type, user, hostname, session UUID
- SQLite storage with WAL mode — survives concurrent terminal windows without corruption
- `~` home-dir collapsing and exit-code colorization in the log view
- Hourly activity bar chart in stats
- JSON and CSV export
- Non-blocking: recording fires as a disowned background subprocess (`&!`) — zero perceptible prompt latency
- Composes cleanly with oh-my-zsh, powerlevel10k, and other zsh frameworks via `add-zsh-hook`
- **Pipeline composition analysis**: parses every recorded command into steps, extracts tool names, and stores them in a `pipeline_steps` table for frequency and bigram analysis

---

## Installation

### From source

Requires Rust (stable, 2021 edition or later).

```bash
git clone https://github.com/danindiana/tapeworm
cd tapeworm
cargo build --release
sudo cp target/release/tapeworm /usr/local/bin/tapeworm
```

### Shell integration

**zsh** — add to `~/.zshrc`:
```zsh
eval "$(tapeworm init --shell zsh)"
```

**bash** — add to `~/.bashrc`:
```bash
eval "$(tapeworm init --shell bash)"
```

Then start a new shell session (or `source ~/.zshrc`). Recording begins immediately.

---

## Usage

```
tapeworm <COMMAND>
```

| Command | Description |
|---------|-------------|
| `init [--shell zsh\|bash]` | Print shell hook snippet for eval |
| `session-id` | Generate a new UUID4 (used internally by hooks) |
| `record --cmd CMD --cwd DIR --exit N --duration N --session S` | Write one record (called by hooks) |
| `log [-l LIMIT]` | Display recent command history (default: 50) |
| `search PATTERN [-l LIMIT]` | Substring search across command history |
| `export [--format json\|csv]` | Dump all records to stdout |
| `stats` | Top commands + hourly activity chart |
| `tools [-l LIMIT]` | Top tools ranked by frequency across all pipeline steps |
| `pipes [-l LIMIT]` | Top pipeline patterns and most common pipe bigrams |
| `db-path` | Print path to the SQLite database file |

### Examples

```bash
# View last 100 commands
tapeworm log -l 100

# Find all git commands
tapeworm search git

# What tools do I use most?
tapeworm tools

# What pipelines do I compose most often?
tapeworm pipes

# Export everything to JSON
tapeworm export --format json > history.json

# Usage statistics
tapeworm stats
```

---

## Database

Records are stored at:
```
~/.local/share/tapeworm/history.db
```

Schema:

```sql
CREATE TABLE commands (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp_unix INTEGER NOT NULL,
    timestamp_iso  TEXT    NOT NULL,   -- RFC 3339
    command        TEXT    NOT NULL,
    cwd            TEXT    NOT NULL,
    exit_code      INTEGER NOT NULL DEFAULT 0,
    duration_ms    INTEGER NOT NULL DEFAULT 0,
    shell          TEXT    NOT NULL DEFAULT 'unknown',
    user           TEXT    NOT NULL DEFAULT '',
    hostname       TEXT    NOT NULL DEFAULT '',
    session_id     TEXT    NOT NULL DEFAULT ''  -- UUID v4 per shell process
);

-- One row per pipeline step within a recorded command.
-- Enables tool frequency, bigram, and composition pattern analysis.
CREATE TABLE pipeline_steps (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    command_id   INTEGER NOT NULL REFERENCES commands(id) ON DELETE CASCADE,
    step_index   INTEGER NOT NULL,   -- 0-based position in pipeline
    tool         TEXT    NOT NULL,   -- extracted tool name (argv[0], basename, wrappers stripped)
    raw          TEXT    NOT NULL,   -- full text of this pipeline step
    connector    TEXT    NOT NULL DEFAULT ''  -- |, &&, ||, ; or "" for last step
);
```

You can query it directly with `sqlite3`:
```bash
sqlite3 ~/.local/share/tapeworm/history.db \
  "SELECT command, exit_code FROM commands ORDER BY timestamp_unix DESC LIMIT 20;"
```

---

## How the shell hooks work

### zsh

`preexec` captures the command text and a millisecond start time before execution. `precmd` fires after the command completes, computes duration, and calls `tapeworm record ... &!` (disowned background job).

```zsh
add-zsh-hook preexec _tapeworm_preexec
add-zsh-hook precmd  _tapeworm_precmd
```

### bash

Uses a `DEBUG` trap to capture `$BASH_COMMAND` before execution, and `PROMPT_COMMAND` to record after. A `_tw_in_prompt` guard prevents recursion.

---

## Semantic search (Ollama embeddings)

tapeworm can embed every recorded command using a local Ollama model and enable natural language retrieval over your shell history.

### Setup

Pull an embedding model (one-time):
```bash
ollama pull nomic-embed-text
```

Embed all recorded commands:
```bash
tapeworm embed
# Embedding 1247 commands with nomic-embed-text …
#   Done. 1247 embedded, 0 errors.
```

Subsequent runs only embed new commands (idempotent — already-embedded commands are skipped).

### Semantic search

```bash
# Natural language queries work across command text + working directory
tapeworm semantic "debug memory leak"
tapeworm semantic "rust compilation failed"
tapeworm semantic "how much disk space am I using"
tapeworm semantic "GPU memory status" -l 5
```

Results are ranked by cosine similarity with color-coded scores:
- **Green (≥80%)** — high confidence match
- **Yellow (60–79%)** — likely relevant
- **Grey (<60%)** — weak match

### How it works

Each command is embedded as:
```
"shell command: {cmd} | directory: {cwd}"
```

Including CWD makes the embedding context-aware — `cargo build` in `~/tapeworm` and `~/other-project` produce slightly different vectors, enabling project-scoped retrieval.

Embeddings are stored as packed little-endian `f32` BLOBs in the `command_embeddings` table. At query time, all embeddings are loaded into memory and ranked by cosine similarity against the query embedding. This is fast enough for typical shell history sizes (tens of thousands of commands).

### Options

```bash
tapeworm embed [--model MODEL] [--url URL] [-l LIMIT]
tapeworm semantic QUERY [-l LIMIT] [--model MODEL] [--url URL]
```

Default model: `nomic-embed-text`. Default URL: `http://localhost:11434`.

### Upgrade path

For very large histories (100k+ commands), replace the in-memory cosine search with [`sqlite-vec`](https://github.com/asg017/sqlite-vec) — a SQLite extension with SIMD-accelerated ANN search. The `command_embeddings` BLOB schema is forward-compatible.

---

## Pipeline composition analysis

Every recorded command is parsed into pipeline steps at record time and stored in `pipeline_steps`. This makes the history corpus a structured execution trace, not just a string log.

### Parser

The parser (`src/parse.rs`) uses a state machine that splits on `|`, `&&`, `||`, `;` at the top level only. It correctly handles:

- Single and double quotes (operators inside quotes are literal)
- Backslash escapes
- `$(...)` subshell expansions (operators inside are not splits)
- Bare `(...)` subshell groupings, e.g. `(cd /tmp && ls) | grep foo`

For each step, the tool name is extracted by stripping:
- Leading env-var assignments (`FOO=bar cmd` → `cmd`)
- Wrapper commands: `sudo`, `env`, `time`, `nice`, `nohup`, `watch`
- Flags belonging to wrappers, including their arguments (`sudo -u root cmd` → `cmd`)
- Path prefixes (`/usr/bin/grep` → `grep`, `./target/release/tapeworm` → `tapeworm`)

### What you can learn

**`tapeworm tools`** — which tools you reach for most, across all pipeline steps (not just first-position commands):

```
grep   ████████████████████████████ 312
sort   ████████████████             189
awk    ████████                      94
```

**`tapeworm pipes`** — which full pipeline patterns recur, and which tool-pairs you compose most:

```
Top patterns:
  grep | sort | uniq | head    (47x)
  ps | grep | awk              (23x)

Top bigrams (A | B):
  grep  →  sort    (61x)
  sort  →  uniq    (47x)
  ps    →  grep    (31x)
```

### Direct SQL queries

```bash
sqlite3 ~/.local/share/tapeworm/history.db

-- Commands where step 0 is git but the pipeline failed
SELECT c.command, c.exit_code
FROM commands c
JOIN pipeline_steps p ON p.command_id = c.id AND p.step_index = 0
WHERE p.tool = 'git' AND c.exit_code != 0;

-- Most common 3-tool pipelines
SELECT GROUP_CONCAT(tool, ' | '), COUNT(*) as cnt
FROM (SELECT command_id, tool FROM pipeline_steps ORDER BY command_id, step_index)
GROUP BY command_id
HAVING COUNT(*) = 3
ORDER BY cnt DESC LIMIT 20;

-- Tools you use after grep (bigrams where grep is the source)
SELECT b.tool, COUNT(*) as cnt
FROM pipeline_steps a JOIN pipeline_steps b
  ON b.command_id = a.command_id AND b.step_index = a.step_index + 1
WHERE a.tool = 'grep' AND a.connector = '|'
GROUP BY b.tool ORDER BY cnt DESC;
```

---

## Dependencies

| Crate | Purpose |
|-------|---------|
| `clap` v4 | CLI argument parsing (derive API) |
| `rusqlite` v0.31 (bundled) | SQLite — compiled in, no system dep |
| `chrono` v0.4 | Timestamps and RFC 3339 formatting |
| `serde` / `serde_json` | JSON serialization |
| `uuid` v1 | Session UUID generation |
| `comfy-table` v7 | Terminal table rendering |
| `colored` v2 | Exit-code colorization |
| `csv` v1 | CSV export |
| `dirs` v5 | XDG data directory resolution |
| `anyhow` v1 | Ergonomic error propagation |
| `hostname` v0.4 | Hostname resolution |

`rusqlite` with `features = ["bundled"]` compiles SQLite statically — no system SQLite version dependency.

---

## License

MIT
