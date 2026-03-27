#!/usr/bin/env python3
"""
Read-only MCP server for the tapeworm history database.

Opens the SQLite DB in read-only URI mode and enforces SELECT-only queries,
so no MCP client (including Claude Code) can mutate or delete history records.

Usage (after `pip install mcp`):
    python tapeworm_mcp.py [--db PATH]

Add to Claude Code:
    claude mcp add tapeworm-history --transport stdio -- \\
        python /path/to/scripts/tapeworm_mcp.py
"""

import asyncio
import re
import sqlite3
import sys
from pathlib import Path

import mcp.server.stdio
import mcp.types as types
from mcp.server import Server

# ---------------------------------------------------------------------------
# Config
# ---------------------------------------------------------------------------

DEFAULT_DB = Path.home() / ".local" / "share" / "tapeworm" / "history.db"
MAX_ROWS = 500  # hard cap — prevent accidental full-table dumps

# ---------------------------------------------------------------------------
# DB helpers
# ---------------------------------------------------------------------------

def _db_path() -> Path:
    for i, arg in enumerate(sys.argv[1:], 1):
        if arg == "--db" and i < len(sys.argv):
            return Path(sys.argv[i + 1])
    return DEFAULT_DB


def _connect():
    path = _db_path()
    if not path.exists():
        raise FileNotFoundError(f"tapeworm DB not found: {path}")
    uri = f"file:{path}?mode=ro"
    return sqlite3.connect(uri, uri=True, check_same_thread=False)


_SAFE_IDENT = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")
_FORBIDDEN = re.compile(
    r"\b(INSERT|UPDATE|DELETE|DROP|CREATE|ALTER|ATTACH|DETACH|PRAGMA|VACUUM|REINDEX|ANALYZE)\b",
    re.IGNORECASE,
)


def _validate_select(sql: str) -> str | None:
    """Return an error string if the query is not a safe SELECT, else None."""
    stripped = sql.strip()
    upper = stripped.upper()
    if not (upper.startswith("SELECT") or upper.startswith("WITH")):
        return "Only SELECT (and CTEs starting with WITH ... SELECT) are permitted."
    m = _FORBIDDEN.search(stripped)
    if m:
        return f"Forbidden keyword '{m.group()}' detected — only read queries are allowed."
    return None


def _fmt_rows(description, rows) -> str:
    if not rows:
        return "(no rows)"
    cols = [d[0] for d in description]
    col_widths = [max(len(c), max((len(str(r[i])) for r in rows), default=0)) for i, c in enumerate(cols)]
    sep = "-+-".join("-" * w for w in col_widths)
    header = " | ".join(c.ljust(col_widths[i]) for i, c in enumerate(cols))
    body = "\n".join(
        " | ".join(str(r[i]).ljust(col_widths[i]) for i, _ in enumerate(cols))
        for r in rows
    )
    note = f"\n({len(rows)} rows)" if len(rows) >= MAX_ROWS else f"\n({len(rows)} rows)"
    return f"{header}\n{sep}\n{body}{note}"


# ---------------------------------------------------------------------------
# MCP server
# ---------------------------------------------------------------------------

app = Server("tapeworm-history")


@app.list_tools()
async def list_tools() -> list[types.Tool]:
    return [
        types.Tool(
            name="query",
            description=(
                "Run a read-only SQL SELECT query against the tapeworm shell history database. "
                f"Results are capped at {MAX_ROWS} rows. Only SELECT statements are permitted — "
                "no INSERT, UPDATE, DELETE, DROP, or other mutations."
            ),
            inputSchema={
                "type": "object",
                "properties": {
                    "sql": {
                        "type": "string",
                        "description": "A SQL SELECT statement (CTEs allowed).",
                    },
                    "params": {
                        "type": "array",
                        "items": {},
                        "description": "Optional positional bind parameters (?1, ?2, ...).",
                        "default": [],
                    },
                },
                "required": ["sql"],
            },
        ),
        types.Tool(
            name="list_tables",
            description="List all tables in the tapeworm database with their row counts.",
            inputSchema={"type": "object", "properties": {}},
        ),
        types.Tool(
            name="describe_table",
            description="Show the column schema for a specific table.",
            inputSchema={
                "type": "object",
                "properties": {
                    "table": {
                        "type": "string",
                        "description": "Table name (alphanumeric + underscores only).",
                    }
                },
                "required": ["table"],
            },
        ),
        types.Tool(
            name="recent_commands",
            description="Return the N most recent commands with timestamp, exit code, and cwd.",
            inputSchema={
                "type": "object",
                "properties": {
                    "limit": {
                        "type": "integer",
                        "description": "Number of commands to return (default 20, max 200).",
                        "default": 20,
                    }
                },
            },
        ),
        types.Tool(
            name="failed_commands",
            description="Return commands that exited non-zero, most recent first.",
            inputSchema={
                "type": "object",
                "properties": {
                    "limit": {
                        "type": "integer",
                        "description": "Number of commands to return (default 20, max 200).",
                        "default": 20,
                    }
                },
            },
        ),
    ]


@app.call_tool()
async def call_tool(name: str, arguments: dict) -> list[types.TextContent]:
    def text(s: str) -> list[types.TextContent]:
        return [types.TextContent(type="text", text=s)]

    try:
        conn = _connect()
    except FileNotFoundError as e:
        return text(f"ERROR: {e}")

    try:
        if name == "list_tables":
            cur = conn.execute(
                "SELECT m.name, (SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=m.name) "
                "FROM sqlite_master WHERE type='table' ORDER BY m.name"
            )
            # row counts via individual queries (can't parameterise table names in COUNT)
            tables = [row[0] for row in cur.fetchall()]
            lines = []
            for t in tables:
                if not _SAFE_IDENT.match(t):
                    continue
                cnt = conn.execute(f"SELECT COUNT(*) FROM {t}").fetchone()[0]
                lines.append(f"{t:<30} {cnt} rows")
            return text("\n".join(lines) if lines else "(no tables)")

        elif name == "describe_table":
            table = arguments.get("table", "")
            if not _SAFE_IDENT.match(table):
                return text("ERROR: Invalid table name — only alphanumeric + underscore allowed.")
            cur = conn.execute(f"PRAGMA table_info({table})")
            rows = cur.fetchall()
            if not rows:
                return text(f"ERROR: Table '{table}' not found.")
            lines = [f"{'col':<4} {'name':<25} {'type':<15} {'notnull':<8} {'pk'}"]
            lines.append("-" * 60)
            for r in rows:
                lines.append(f"{r[0]:<4} {r[1]:<25} {r[2]:<15} {r[3]:<8} {r[5]}")
            return text("\n".join(lines))

        elif name == "query":
            sql = arguments.get("sql", "")
            params = arguments.get("params", [])
            err = _validate_select(sql)
            if err:
                return text(f"ERROR: {err}")
            # Inject LIMIT if not present
            upper = sql.strip().upper()
            if "LIMIT" not in upper:
                sql = sql.rstrip("; \n") + f" LIMIT {MAX_ROWS}"
            cur = conn.execute(sql, params)
            rows = cur.fetchmany(MAX_ROWS)
            return text(_fmt_rows(cur.description, rows))

        elif name == "recent_commands":
            limit = min(int(arguments.get("limit", 20)), 200)
            cur = conn.execute(
                "SELECT id, timestamp_iso, exit_code, duration_ms, cwd, command "
                "FROM commands ORDER BY id DESC LIMIT ?",
                (limit,),
            )
            return text(_fmt_rows(cur.description, cur.fetchall()))

        elif name == "failed_commands":
            limit = min(int(arguments.get("limit", 20)), 200)
            cur = conn.execute(
                "SELECT id, timestamp_iso, exit_code, cwd, command "
                "FROM commands WHERE exit_code != 0 ORDER BY id DESC LIMIT ?",
                (limit,),
            )
            return text(_fmt_rows(cur.description, cur.fetchall()))

        else:
            return text(f"ERROR: Unknown tool '{name}'")

    except sqlite3.OperationalError as e:
        return text(f"SQL ERROR: {e}")
    finally:
        conn.close()


# ---------------------------------------------------------------------------
# Entry point
# ---------------------------------------------------------------------------

async def main():
    async with mcp.server.stdio.stdio_server() as (read_stream, write_stream):
        await app.run(
            read_stream,
            write_stream,
            app.create_initialization_options(),
        )


if __name__ == "__main__":
    asyncio.run(main())
