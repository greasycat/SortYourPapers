"""The read-only guard on `sortyourpaperya sql`.

A guard against a mistake, not a sandbox: whoever can run this can already read
the database file. What it has to stop is a query written to count documents
turning out to delete them, and a second statement riding in behind the first.
"""

from __future__ import annotations

from pathlib import Path

import pytest

from sortyourpaperya.db import PaperDb, is_read_only


@pytest.mark.parametrize(
    "statement",
    [
        "SELECT title FROM papers",
        "select * from papers where year = 2017",
        "WITH t AS (SELECT 1) SELECT * FROM t",
        "FROM papers SELECT title",  # DuckDB's from-first form
        "DESCRIBE papers",
        "SUMMARIZE papers",
        "SELECT ';DROP TABLE papers'",  # a semicolon inside a literal
        "SELECT 1;",  # one statement, terminated
        "  \n SELECT 1",
    ],
)
def test_a_reading_statement_is_allowed(statement: str) -> None:
    assert is_read_only(statement) is True


@pytest.mark.parametrize(
    "statement",
    [
        "DELETE FROM papers",
        "UPDATE papers SET title = 'x'",
        "DROP TABLE papers",
        "INSERT INTO papers VALUES (1)",
        "ATTACH 'elsewhere.duckdb'",
        "COPY papers TO 'out.csv'",
        "PRAGMA database_list",
        "SELECT 1; DROP TABLE papers",  # a second statement riding in behind
        "SELECT 1;DELETE FROM papers;",
        "-- harmless\nDELETE FROM papers",  # hidden behind a comment
        "/* SELECT */ UPDATE papers SET title = 'x'",
        "",
        "   ",
    ],
)
def test_anything_that_could_write_is_refused(statement: str) -> None:
    assert is_read_only(statement) is False


def test_select_returns_columns_and_rows(tmp_path: Path) -> None:
    from sortyourpaperya.db import Paper

    with PaperDb(tmp_path / "papers.duckdb") as db:
        db.upsert(
            Paper(
                file_id="abc123abc123",
                content_hash="sha-1",
                store_name="abc123abc123__AI",
                title="A Paper",
                year=2017,
            )
        )

        columns, rows = db.select("SELECT title, year FROM papers")

    assert columns == ["title", "year"]
    assert rows == [("A Paper", 2017)]


def test_a_writing_statement_never_reaches_duckdb(tmp_path: Path) -> None:
    from sortyourpaperya.db import Paper

    with PaperDb(tmp_path / "papers.duckdb") as db:
        db.upsert(Paper(file_id="abc123abc123", content_hash="s", store_name="s"))

        with pytest.raises(ValueError, match="single reading statement"):
            db.select("DELETE FROM papers")

        assert db.count() == 1, "the document must still be there"
