"""DuckDB store holding everything known about each paper.

The database is the source of truth. The flat store's filenames and the symlink
tree are both projections of it and can be rebuilt from it at any time, which is
what makes re-tagging safe: change the rows, then rebuild the names.

Column names follow the Rust `paper-db` crate (`file_id`, `content_hash`,
`created_at_ms`) so the two can be read side by side.

Expanding it means adding a column: `_MIGRATIONS` is an ordered list applied
once each and recorded in `schema_version`. Anything not worth a column yet goes
in `paper_attributes` as a key/value pair.
"""

from __future__ import annotations

import time
from contextlib import contextmanager
from dataclasses import dataclass, field
from pathlib import Path
from typing import Iterator, Sequence

import duckdb

# DuckDB allows one writing process at a time, so a command run while the
# watcher is mid-pass finds the file locked. Waiting briefly turns that from an
# error into a pause; past this the lock is not contention but a stuck process,
# and saying so is more use than waiting longer.
LOCK_WAIT_SECONDS = 30.0
_LOCK_POLL_SECONDS = 0.25

# Applied in order, each exactly once. Append to expand the schema; never edit
# or reorder an entry that has shipped.
_MIGRATIONS: list[tuple[str, str]] = [
    (
        "0001_initial",
        """
        CREATE TABLE IF NOT EXISTS papers (
            file_id        TEXT PRIMARY KEY,
            content_hash   TEXT NOT NULL,
            store_name     TEXT NOT NULL,
            original_name  TEXT,
            source_path    TEXT,
            size_bytes     BIGINT,
            pages_read     INTEGER,
            title          TEXT,
            year           INTEGER,
            created_at_ms  BIGINT NOT NULL,
            updated_at_ms  BIGINT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS paper_tags (
            file_id  TEXT NOT NULL,
            position INTEGER NOT NULL,
            tag      TEXT NOT NULL,
            PRIMARY KEY (file_id, position)
        );

        CREATE TABLE IF NOT EXISTS paper_authors (
            file_id  TEXT NOT NULL,
            position INTEGER NOT NULL,
            name     TEXT NOT NULL,
            PRIMARY KEY (file_id, position)
        );

        CREATE TABLE IF NOT EXISTS paper_keywords (
            file_id TEXT NOT NULL,
            keyword TEXT NOT NULL,
            PRIMARY KEY (file_id, keyword)
        );

        CREATE TABLE IF NOT EXISTS paper_attributes (
            file_id TEXT NOT NULL,
            key     TEXT NOT NULL,
            value   TEXT,
            PRIMARY KEY (file_id, key)
        );
        """,
    ),
    (
        "0002_stored_file_state",
        """
        ALTER TABLE papers ADD COLUMN IF NOT EXISTS stored_mtime_ms BIGINT;
        """,
    ),
    (
        "0003_page_image_provenance",
        """
        ALTER TABLE papers ADD COLUMN IF NOT EXISTS from_page_images BOOLEAN;
        """,
    ),
    (
        "0004_document_folder",
        """
        ALTER TABLE papers ADD COLUMN IF NOT EXISTS document_name TEXT;
        """,
    ),
]


@dataclass
class Paper:
    """Everything the library knows about one PDF."""

    file_id: str
    content_hash: str
    # The document's folder in the store, `<id>__<Tag>__<Tag>`. It holds the
    # document and anything its owner keeps beside it.
    store_name: str
    # The document file inside that folder.
    document_name: str = ""
    original_name: str | None = None
    source_path: str | None = None
    # Size and mtime of the file in the store, as last observed. Together they
    # are a cheap gate: a file whose stat has not moved does not need rehashing.
    size_bytes: int | None = None
    stored_mtime_ms: int | None = None
    pages_read: int | None = None
    # True when the text behind this document's labels was read off rendered
    # page images rather than a text layer.
    from_page_images: bool = False
    title: str | None = None
    year: int | None = None
    tags: list[str] = field(default_factory=list)
    authors: list[str] = field(default_factory=list)
    keywords: list[str] = field(default_factory=list)


def now_ms() -> int:
    return int(time.time() * 1000)


class PaperDb:
    """Connection to the library database."""

    def __init__(self, path: Path) -> None:
        self.path = path
        path.parent.mkdir(parents=True, exist_ok=True)
        self._connection: duckdb.DuckDBPyConnection | None = None

    @property
    def _conn(self) -> duckdb.DuckDBPyConnection:
        """The connection, opened on first use.

        DuckDB takes an exclusive lock on the file, so a long-running watcher
        that held one open would block every other `sypy` command for as long as
        it ran. Connecting lazily lets an idle watcher drop the lock — see
        `release` — and pick it up again for the next pass.
        """
        if self._connection is None:
            self._connection = self._connect()
            self._migrate()
        return self._connection

    def _connect(self) -> duckdb.DuckDBPyConnection:
        """Open the database, waiting out a lock another process is holding.

        Only lock contention is waited on. Any other IO failure — a missing
        directory, a permission problem, a corrupt file — is raised at once,
        because no amount of waiting fixes it.
        """
        deadline = time.monotonic() + LOCK_WAIT_SECONDS
        while True:
            try:
                return duckdb.connect(str(self.path))
            except duckdb.IOException as err:
                if "lock" not in str(err).lower() or time.monotonic() >= deadline:
                    raise
                time.sleep(_LOCK_POLL_SECONDS)

    def release(self) -> None:
        """Drop the connection, and the file lock with it.

        Safe to call at any time: the next read or write reconnects.
        """
        if self._connection is not None:
            self._connection.close()
            self._connection = None

    def close(self) -> None:
        self.release()

    def __enter__(self) -> "PaperDb":
        return self

    def __exit__(self, *_exc: object) -> None:
        self.close()

    def _migrate(self) -> None:
        conn = self._connection
        assert conn is not None  # only called from the connection property
        conn.execute(
            """
            CREATE TABLE IF NOT EXISTS schema_version (
                name        TEXT PRIMARY KEY,
                applied_at_ms BIGINT NOT NULL
            )
            """
        )
        applied = {
            row[0] for row in conn.execute("SELECT name FROM schema_version").fetchall()
        }
        for name, sql in _MIGRATIONS:
            if name in applied:
                continue
            conn.execute("BEGIN")
            try:
                conn.execute(sql)
                conn.execute(
                    "INSERT INTO schema_version VALUES (?, ?)", [name, now_ms()]
                )
                conn.execute("COMMIT")
            except Exception:
                conn.execute("ROLLBACK")
                raise

    def backup_to(self, destination: Path) -> Path:
        """Write a standalone copy of the database to `destination`.

        Copied through DuckDB rather than by copying the file: a database being
        written has changes in a write-ahead log beside it, so a file copy taken
        at the wrong moment restores to a database missing its most recent rows
        — or to one that will not open at all. `COPY FROM DATABASE` reads a
        committed view and writes a complete one.

        Raises:
            duckdb.Error: if the copy cannot be made. A backup that failed has
                to say so; a quiet one is worse than none, because it is
                believed.
        """
        if destination.exists():
            raise duckdb.IOException(f"{destination} already exists")
        destination.parent.mkdir(parents=True, exist_ok=True)

        source = self._conn.execute("SELECT current_database()").fetchone()[0]
        # ATTACH takes a literal, not a parameter, so the path is quoted by
        # hand. Doubling the quote is what keeps a folder name containing one
        # from ending the string early.
        literal = str(destination).replace("'", "''")
        self._conn.execute(f"ATTACH '{literal}' AS sypy_backup")
        try:
            self._conn.execute(f'COPY FROM DATABASE "{source}" TO sypy_backup')
        finally:
            self._conn.execute("DETACH sypy_backup")
        return destination

    @contextmanager
    def _transaction(self) -> Iterator[None]:
        self._conn.execute("BEGIN")
        try:
            yield
            self._conn.execute("COMMIT")
        except Exception:
            self._conn.execute("ROLLBACK")
            raise

    # ---- reads -------------------------------------------------------------

    def find_by_content_hash(self, content_hash: str) -> str | None:
        """The id of the paper with this content, if the library already has it."""
        row = self._conn.execute(
            "SELECT file_id FROM papers WHERE content_hash = ?", [content_hash]
        ).fetchone()
        return row[0] if row else None

    def get(self, file_id: str) -> Paper | None:
        row = self._conn.execute(
            """
            SELECT file_id, content_hash, store_name, original_name, source_path,
                   size_bytes, pages_read, title, year, stored_mtime_ms,
                   from_page_images, document_name
            FROM papers WHERE file_id = ?
            """,
            [file_id],
        ).fetchone()
        if row is None:
            return None
        return self._hydrate(row)

    def all_papers(self) -> list[Paper]:
        rows = self._conn.execute(
            """
            SELECT file_id, content_hash, store_name, original_name, source_path,
                   size_bytes, pages_read, title, year, stored_mtime_ms,
                   from_page_images, document_name
            FROM papers ORDER BY file_id
            """
        ).fetchall()
        return [self._hydrate(row) for row in rows]

    def count(self) -> int:
        return self._conn.execute("SELECT count(*) FROM papers").fetchone()[0]

    def tag_paths(self, limit: int | None = None) -> list[str]:
        """Every distinct category path the library already uses, as `A/B/C`.

        This is what a new document is shown so it can join an existing branch
        instead of inventing a parallel one.
        """
        rows = self._conn.execute(
            """
            SELECT string_agg(tag, '/' ORDER BY position) AS path
            FROM paper_tags
            GROUP BY file_id
            """
        ).fetchall()
        paths = sorted({path for (path,) in rows if path})
        return paths[:limit] if limit else paths

    def known_content_hashes(self, digests: Sequence[str]) -> set[str]:
        """Which of these contents the library already holds.

        One query rather than one per candidate, so the whole already-known
        check is a single short visit to the database.
        """
        if not digests:
            return set()
        unique = list(dict.fromkeys(digests))
        placeholders = ", ".join("?" for _ in unique)
        rows = self._conn.execute(
            f"SELECT content_hash FROM papers WHERE content_hash IN ({placeholders})",  # noqa: S608
            unique,
        ).fetchall()
        return {row[0] for row in rows}

    def delete(self, file_id: str) -> None:
        """Forget a document entirely."""
        with self._transaction():
            for table in (
                "paper_tags",
                "paper_authors",
                "paper_keywords",
                "paper_attributes",
                "papers",
            ):
                self._conn.execute(
                    f"DELETE FROM {table} WHERE file_id = ?", [file_id]  # noqa: S608
                )

    def attributes(self, file_id: str) -> dict[str, str | None]:
        rows = self._conn.execute(
            "SELECT key, value FROM paper_attributes WHERE file_id = ? ORDER BY key",
            [file_id],
        ).fetchall()
        return {key: value for key, value in rows}

    def _hydrate(self, row: tuple) -> Paper:
        file_id = row[0]
        return Paper(
            file_id=file_id,
            content_hash=row[1],
            store_name=row[2],
            original_name=row[3],
            source_path=row[4],
            size_bytes=row[5],
            pages_read=row[6],
            title=row[7],
            year=row[8],
            stored_mtime_ms=row[9],
            from_page_images=bool(row[10]),
            document_name=row[11] or "",
            tags=self._ordered(file_id, "paper_tags", "tag"),
            authors=self._ordered(file_id, "paper_authors", "name"),
            keywords=[
                value
                for (value,) in self._conn.execute(
                    "SELECT keyword FROM paper_keywords WHERE file_id = ? ORDER BY keyword",
                    [file_id],
                ).fetchall()
            ],
        )

    def _ordered(self, file_id: str, table: str, column: str) -> list[str]:
        rows = self._conn.execute(
            f"SELECT {column} FROM {table} WHERE file_id = ? ORDER BY position",  # noqa: S608
            [file_id],
        ).fetchall()
        return [value for (value,) in rows]

    # ---- writes ------------------------------------------------------------

    def upsert(self, paper: Paper) -> None:
        """Write a paper and its multi-valued fields as one atomic change."""
        with self._transaction():
            self._write(paper)

    def known_content_hashes(self, digests: Sequence[str]) -> set[str]:
        """Which of these contents the library already holds.

        One query rather than one per candidate, so the whole already-known
        check is a single short visit to the database.
        """
        if not digests:
            return set()
        unique = list(dict.fromkeys(digests))
        placeholders = ", ".join("?" for _ in unique)
        rows = self._conn.execute(
            f"SELECT content_hash FROM papers WHERE content_hash IN ({placeholders})",  # noqa: S608
            unique,
        ).fetchall()
        return {row[0] for row in rows}

    def delete(self, file_id: str) -> None:
        """Forget a document entirely."""
        with self._transaction():
            for table in (
                "paper_tags",
                "paper_authors",
                "paper_keywords",
                "paper_attributes",
                "papers",
            ):
                self._conn.execute(
                    f"DELETE FROM {table} WHERE file_id = ?", [file_id]  # noqa: S608
                )

    def attributes(self, file_id: str) -> dict[str, str | None]:
        rows = self._conn.execute(
            "SELECT key, value FROM paper_attributes WHERE file_id = ? ORDER BY key",
            [file_id],
        ).fetchall()
        return {key: value for key, value in rows}

    def _hydrate(self, row: tuple) -> Paper:
        file_id = row[0]
        return Paper(
            file_id=file_id,
            content_hash=row[1],
            store_name=row[2],
            original_name=row[3],
            source_path=row[4],
            size_bytes=row[5],
            pages_read=row[6],
            title=row[7],
            year=row[8],
            stored_mtime_ms=row[9],
            from_page_images=bool(row[10]),
            document_name=row[11] or "",
            tags=self._ordered(file_id, "paper_tags", "tag"),
            authors=self._ordered(file_id, "paper_authors", "name"),
            keywords=[
                value
                for (value,) in self._conn.execute(
                    "SELECT keyword FROM paper_keywords WHERE file_id = ? ORDER BY keyword",
                    [file_id],
                ).fetchall()
            ],
        )

    def _ordered(self, file_id: str, table: str, column: str) -> list[str]:
        rows = self._conn.execute(
            f"SELECT {column} FROM {table} WHERE file_id = ? ORDER BY position",  # noqa: S608
            [file_id],
        ).fetchall()
        return [value for (value,) in rows]

    # ---- writes ------------------------------------------------------------

    def upsert(self, paper: Paper) -> None:
        """Write a paper and its multi-valued fields as one atomic change."""
        with self._transaction():
            self._write(paper)

    def _write(self, paper: Paper) -> None:
        """Write one paper. Caller owns the transaction."""
        timestamp = now_ms()
        existing = self._conn.execute(
            "SELECT created_at_ms FROM papers WHERE file_id = ?", [paper.file_id]
        ).fetchone()
        created_at = existing[0] if existing else timestamp
        self._conn.execute("DELETE FROM papers WHERE file_id = ?", [paper.file_id])
        self._conn.execute(
            """
            INSERT INTO papers (
                file_id, content_hash, store_name, document_name,
                original_name, source_path, size_bytes, stored_mtime_ms,
                pages_read, title, year, from_page_images,
                created_at_ms, updated_at_ms
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            [
                paper.file_id,
                paper.content_hash,
                paper.store_name,
                paper.document_name,
                paper.original_name,
                paper.source_path,
                paper.size_bytes,
                paper.stored_mtime_ms,
                paper.pages_read,
                paper.title,
                paper.year,
                paper.from_page_images,
                created_at,
                timestamp,
            ],
        )
        self._replace_ordered(paper.file_id, "paper_tags", "tag", paper.tags)
        self._replace_ordered(paper.file_id, "paper_authors", "name", paper.authors)
        self._conn.execute(
            "DELETE FROM paper_keywords WHERE file_id = ?", [paper.file_id]
        )
        for keyword in dict.fromkeys(paper.keywords):
            self._conn.execute(
                "INSERT INTO paper_keywords VALUES (?, ?)", [paper.file_id, keyword]
            )

    def upsert_many(self, papers: Sequence[Paper]) -> None:
        """Write several documents in one transaction.

        Filing writes every document of a pass at once so the lock is taken
        once, after the copying is finished, rather than once per document
        while it is still going on — and so a failure part-way leaves none of
        them recorded rather than an arbitrary prefix.
        """
        if not papers:
            return
        with self._transaction():
            for paper in papers:
                self._write(paper)

    def set_stored_file_states(
        self, states: Sequence[tuple[str, str, int, int]]
    ) -> None:
        """Record what several stored files look like now, in one transaction.

        Each entry is ``(file_id, content_hash, size_bytes, mtime_ms)``.
        """
        if not states:
            return
        timestamp = now_ms()
        with self._transaction():
            for file_id, content_hash, size_bytes, mtime_ms in states:
                self._conn.execute(
                    """
                    UPDATE papers
                    SET content_hash = ?, size_bytes = ?, stored_mtime_ms = ?,
                        updated_at_ms = ?
                    WHERE file_id = ?
                    """,
                    [content_hash, size_bytes, mtime_ms, timestamp, file_id],
                )

    def set_tags(self, file_id: str, tags: list[str], store_name: str) -> None:
        """Re-tag a document and record the new name of its store folder."""
        with self._transaction():
            self._replace_ordered(file_id, "paper_tags", "tag", tags)
            self._conn.execute(
                "UPDATE papers SET store_name = ?, updated_at_ms = ? WHERE file_id = ?",
                [store_name, now_ms(), file_id],
            )

    def set_stored_file_state(
        self, file_id: str, content_hash: str, size_bytes: int, mtime_ms: int
    ) -> None:
        """Record what the stored file looks like now.

        Called after filing, and again whenever a rescan finds the file on disk
        no longer matches what was recorded.
        """
        with self._transaction():
            self._conn.execute(
                """
                UPDATE papers
                SET content_hash = ?, size_bytes = ?, stored_mtime_ms = ?,
                    updated_at_ms = ?
                WHERE file_id = ?
                """,
                [content_hash, size_bytes, mtime_ms, now_ms(), file_id],
            )

    def set_store_layout(
        self, file_id: str, store_name: str, document_name: str
    ) -> None:
        """Record where a document's folder and file now are."""
        with self._transaction():
            self._conn.execute(
                """
                UPDATE papers
                SET store_name = ?, document_name = ?, updated_at_ms = ?
                WHERE file_id = ?
                """,
                [store_name, document_name, now_ms(), file_id],
            )

    def set_attribute(self, file_id: str, key: str, value: str | None) -> None:
        """Record a field that does not have a column yet."""
        with self._transaction():
            self._conn.execute(
                "DELETE FROM paper_attributes WHERE file_id = ? AND key = ?",
                [file_id, key],
            )
            self._conn.execute(
                "INSERT INTO paper_attributes VALUES (?, ?, ?)", [file_id, key, value]
            )

    def _replace_ordered(
        self, file_id: str, table: str, column: str, values: list[str]
    ) -> None:
        self._conn.execute(f"DELETE FROM {table} WHERE file_id = ?", [file_id])  # noqa: S608
        for position, value in enumerate(values):
            self._conn.execute(
                f"INSERT INTO {table} (file_id, position, {column}) VALUES (?, ?, ?)",  # noqa: S608
                [file_id, position, value],
            )
