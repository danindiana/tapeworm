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
| `db-path` | Print path to the SQLite database file |

### Examples

```bash
# View last 100 commands
tapeworm log -l 100

# Find all git commands
tapeworm search git

# Export everything to JSON
tapeworm export --format json > history.json

# Export to CSV
tapeworm export --format csv > history.csv

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
