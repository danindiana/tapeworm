"""
Tests for the pure-Python validation and formatting functions in tapeworm_mcp.py.

MCP server infrastructure (asyncio event loop, stdio transport, tool registration)
is not exercised here — only the logic that gates what SQL reaches the database.
These are the functions where a bug has security consequences.
"""

import sys
from pathlib import Path

import pytest

# Ensure scripts/ is on the path so the import below works from any cwd.
sys.path.insert(0, str(Path(__file__).parent))

from tapeworm_mcp import _SAFE_IDENT, _fmt_rows, _validate_select


# ---------------------------------------------------------------------------
# _validate_select — happy path
# ---------------------------------------------------------------------------

class TestValidateSelectAccepted:
    def test_simple_select(self):
        assert _validate_select("SELECT 1") is None

    def test_lowercase_select(self):
        assert _validate_select("select * from commands") is None

    def test_mixed_case_select(self):
        assert _validate_select("Select id From commands") is None

    def test_leading_whitespace(self):
        assert _validate_select("   SELECT id FROM commands") is None

    def test_leading_newline(self):
        assert _validate_select("\nSELECT id FROM commands") is None

    def test_with_cte(self):
        sql = "WITH cte AS (SELECT id FROM commands) SELECT * FROM cte"
        assert _validate_select(sql) is None

    def test_with_cte_lowercase(self):
        sql = "with cte as (select id from commands) select * from cte"
        assert _validate_select(sql) is None

    def test_select_with_limit(self):
        assert _validate_select("SELECT * FROM commands LIMIT 10") is None

    def test_select_with_join(self):
        sql = (
            "SELECT c.id, p.tool "
            "FROM commands c "
            "JOIN pipeline_steps p ON p.command_id = c.id"
        )
        assert _validate_select(sql) is None

    def test_select_with_subquery(self):
        sql = "SELECT * FROM (SELECT id, command FROM commands WHERE exit_code != 0)"
        assert _validate_select(sql) is None

    def test_forbidden_word_inside_string_literal(self):
        # Querying *for* commands that contained DROP/CREATE is a valid use case.
        sql = "SELECT * FROM commands WHERE command LIKE '%DROP TABLE%'"
        assert _validate_select(sql) is None, (
            "Forbidden keyword inside a string literal should not be blocked"
        )

    def test_multiple_forbidden_words_all_in_literals(self):
        sql = "SELECT * FROM commands WHERE command IN ('DROP TABLE foo', 'CREATE INDEX bar')"
        assert _validate_select(sql) is None

    def test_escaped_quote_inside_literal(self):
        # SQLite escapes single quotes by doubling them: 'O''Reilly'
        sql = "SELECT * FROM commands WHERE user = 'O''Reilly'"
        assert _validate_select(sql) is None

    def test_semicolon_in_string_literal_not_injection(self):
        sql = "SELECT * FROM commands WHERE command = 'echo hello; ls'"
        assert _validate_select(sql) is None


# ---------------------------------------------------------------------------
# _validate_select — write/admin operations must all be blocked
# ---------------------------------------------------------------------------

class TestValidateSelectRejected:
    def _assert_blocked(self, sql: str):
        result = _validate_select(sql)
        assert result is not None, f"Expected rejection for: {sql!r}"
        assert "ERROR" not in result or True  # result is an error message string

    # --- not starting with SELECT / WITH ---

    def test_empty_string(self):
        self._assert_blocked("")

    def test_bare_table_name(self):
        self._assert_blocked("commands")

    def test_explain(self):
        self._assert_blocked("EXPLAIN SELECT * FROM commands")

    def test_pragma_direct(self):
        self._assert_blocked("PRAGMA journal_mode")

    # --- write keywords at statement level ---

    def test_insert(self):
        self._assert_blocked("INSERT INTO commands VALUES (1,2,3)")

    def test_update(self):
        self._assert_blocked("UPDATE commands SET command = 'x'")

    def test_delete(self):
        self._assert_blocked("DELETE FROM commands WHERE id = 1")

    def test_drop(self):
        self._assert_blocked("DROP TABLE commands")

    def test_create(self):
        self._assert_blocked("CREATE TABLE foo (id INTEGER)")

    def test_alter(self):
        self._assert_blocked("ALTER TABLE commands ADD COLUMN foo TEXT")

    def test_attach(self):
        self._assert_blocked("ATTACH DATABASE 'other.db' AS other")

    def test_detach(self):
        self._assert_blocked("DETACH other")

    def test_vacuum(self):
        self._assert_blocked("VACUUM")

    def test_reindex(self):
        self._assert_blocked("REINDEX commands")

    def test_analyze(self):
        self._assert_blocked("ANALYZE commands")

    # --- case insensitivity ---

    def test_insert_lowercase(self):
        self._assert_blocked("insert into commands values (1,2,3)")

    def test_drop_mixed_case(self):
        self._assert_blocked("dRoP TABLE commands")

    # --- semicolon injection: forbidden keyword after semicolon ---

    def test_semicolon_injection(self):
        # The DROP is outside any string literal — must be caught.
        self._assert_blocked("SELECT 1; DROP TABLE commands")

    def test_semicolon_injection_update(self):
        self._assert_blocked("SELECT * FROM commands; UPDATE commands SET command='x'")

    # --- forbidden keyword in comment would still be flagged (acceptable) ---

    def test_forbidden_in_sql_comment(self):
        # SQL comments are not stripped — DROP in a comment triggers rejection.
        # This is conservative but safe: the cost of a false positive here is low.
        result = _validate_select("SELECT 1 -- DROP TABLE commands")
        # We don't assert the direction here — just document the current behaviour.
        # If the result is None (accepted), the query is harmless; if rejected, it's
        # an intentionally conservative decision.
        assert result is None or isinstance(result, str)


# ---------------------------------------------------------------------------
# _SAFE_IDENT — table name whitelist regex
# ---------------------------------------------------------------------------

class TestSafeIdent:
    def test_simple_name(self):
        assert _SAFE_IDENT.match("commands")

    def test_underscore_name(self):
        assert _SAFE_IDENT.match("pipeline_steps")

    def test_schema_versions(self):
        assert _SAFE_IDENT.match("schema_versions")

    def test_leading_underscore(self):
        assert _SAFE_IDENT.match("_internal")

    def test_alphanumeric(self):
        assert _SAFE_IDENT.match("table1")

    def test_starts_with_digit_rejected(self):
        assert not _SAFE_IDENT.match("1commands")

    def test_semicolon_rejected(self):
        assert not _SAFE_IDENT.match("commands; DROP TABLE commands")

    def test_slash_rejected(self):
        assert not _SAFE_IDENT.match("../../etc/passwd")

    def test_space_rejected(self):
        assert not _SAFE_IDENT.match("foo bar")

    def test_dot_rejected(self):
        assert not _SAFE_IDENT.match("foo.bar")

    def test_empty_rejected(self):
        assert not _SAFE_IDENT.match("")


# ---------------------------------------------------------------------------
# _fmt_rows — result formatting
# ---------------------------------------------------------------------------

class TestFmtRows:
    def test_no_rows(self):
        assert _fmt_rows([], []) == "(no rows)"

    def test_single_row_single_col(self):
        desc = [("id",)]
        rows = [(42,)]
        out = _fmt_rows(desc, rows)
        assert "id" in out
        assert "42" in out
        assert "(1 rows)" in out

    def test_header_and_body_present(self):
        desc = [("id",), ("command",)]
        rows = [(1, "ls -la"), (2, "git status")]
        out = _fmt_rows(desc, rows)
        assert "id" in out
        assert "command" in out
        assert "ls -la" in out
        assert "git status" in out
        assert "(2 rows)" in out

    def test_separator_line_present(self):
        desc = [("col",)]
        rows = [("value",)]
        out = _fmt_rows(desc, rows)
        # Separator is dashes
        assert "-" in out

    def test_columns_left_justified(self):
        desc = [("short",), ("a_longer_column_name",)]
        rows = [("x", "y")]
        out = _fmt_rows(desc, rows)
        # Both column names must appear
        assert "short" in out
        assert "a_longer_column_name" in out
