from __future__ import annotations

from pathlib import Path

import pytest

from syp_prototype.db import Paper, PaperDb


def _paper(**overrides) -> Paper:
    fields = {
        "file_id": "abc123abc123",
        "content_hash": "sha-1",
        "store_name": "abc123abc123__AI.pdf",
        "title": "A Paper",
        "year": 2017,
        "tags": ["AI", "Transformers"],
        "authors": ["Ashish Vaswani", "Noam Shazeer"],
        "keywords": ["attention", "sequence"],
    }
    fields.update(overrides)
    return Paper(**fields)


def test_round_trips_a_paper_with_its_ordered_multi_valued_fields(
    tmp_path: Path,
) -> None:
    with PaperDb(tmp_path / "papers.duckdb") as db:
        db.upsert(_paper())

        loaded = db.get("abc123abc123")

    assert loaded is not None
    assert loaded.tags == ["AI", "Transformers"], "tag order is the folder order"
    assert loaded.authors == ["Ashish Vaswani", "Noam Shazeer"]
    assert sorted(loaded.keywords) == ["attention", "sequence"]
    assert loaded.year == 2017


def test_content_hash_lookup_is_what_makes_reingest_free(tmp_path: Path) -> None:
    with PaperDb(tmp_path / "papers.duckdb") as db:
        db.upsert(_paper())

        assert db.find_by_content_hash("sha-1") == "abc123abc123"
        assert db.find_by_content_hash("sha-missing") is None


def test_upsert_preserves_created_at_but_moves_updated_at(tmp_path: Path) -> None:
    path = tmp_path / "papers.duckdb"
    with PaperDb(path) as db:
        db.upsert(_paper())
        created, updated = db._conn.execute(
            "SELECT created_at_ms, updated_at_ms FROM papers"
        ).fetchone()

        db.upsert(_paper(title="Renamed"))
        created_after, updated_after = db._conn.execute(
            "SELECT created_at_ms, updated_at_ms FROM papers"
        ).fetchone()

    assert created_after == created
    assert updated_after >= updated
    assert created == updated


def test_retagging_replaces_tags_rather_than_appending(tmp_path: Path) -> None:
    with PaperDb(tmp_path / "papers.duckdb") as db:
        db.upsert(_paper())

        db.set_tags("abc123abc123", ["Systems"], "abc123abc123__Systems.pdf")
        loaded = db.get("abc123abc123")

    assert loaded is not None
    assert loaded.tags == ["Systems"]
    assert loaded.store_name == "abc123abc123__Systems.pdf"


def test_attributes_hold_fields_that_have_no_column_yet(tmp_path: Path) -> None:
    with PaperDb(tmp_path / "papers.duckdb") as db:
        db.upsert(_paper())

        db.set_attribute("abc123abc123", "doi", "10.1000/xyz")
        db.set_attribute("abc123abc123", "doi", "10.1000/corrected")
        db.set_attribute("abc123abc123", "venue", "NeurIPS")

        assert db.attributes("abc123abc123") == {
            "doi": "10.1000/corrected",
            "venue": "NeurIPS",
        }


def test_migrations_are_applied_once_and_reopening_is_safe(tmp_path: Path) -> None:
    path = tmp_path / "papers.duckdb"
    with PaperDb(path) as db:
        db.upsert(_paper())
        first = db._conn.execute("SELECT count(*) FROM schema_version").fetchone()[0]

    with PaperDb(path) as db:
        second = db._conn.execute("SELECT count(*) FROM schema_version").fetchone()[0]
        assert db.count() == 1, "reopening must not wipe anything"

    assert first == second


def test_a_lock_held_by_another_process_is_waited_out(tmp_path: Path) -> None:
    """A command run while the watcher is mid-pass should pause, not fail."""
    import subprocess
    import sys
    import time

    path = tmp_path / "papers.duckdb"
    with PaperDb(path) as db:
        db.upsert(_paper())

    # A stand-in for a watcher holding the write lock for a moment.
    holder = subprocess.Popen(
        [
            sys.executable,
            "-c",
            f"import duckdb,time;c=duckdb.connect({str(path)!r});"
            "c.execute('SELECT count(*) FROM papers').fetchone();"
            "print('held',flush=True);time.sleep(1.5);c.close()",
        ],
        stdout=subprocess.PIPE,
        text=True,
    )
    try:
        assert holder.stdout.readline().strip() == "held"
        started = time.monotonic()
        with PaperDb(path) as db:
            assert db.count() == 1
        waited = time.monotonic() - started
    finally:
        holder.wait(timeout=10)

    assert waited >= 0.5, "it should have waited for the holder, not raced past"


def test_a_failure_that_is_not_contention_is_raised_at_once(tmp_path: Path) -> None:
    # Waiting 30s on a permission error would help nobody.
    import time

    import duckdb

    db = PaperDb(tmp_path / "papers.duckdb")
    original = duckdb.connect

    def refuse(*args, **kwargs):
        raise duckdb.IOException("Permission denied")

    duckdb.connect = refuse
    try:
        started = time.monotonic()
        with pytest.raises(duckdb.IOException, match="Permission denied"):
            db.count()
        assert time.monotonic() - started < 1.0, "should not have waited"
    finally:
        duckdb.connect = original


def test_a_failed_batch_write_records_none_of_it(tmp_path: Path) -> None:
    # One transaction for the batch, so a failure part-way leaves the library
    # unchanged rather than holding an arbitrary prefix of the pass.
    db = PaperDb(tmp_path / "papers.duckdb")
    good = _paper(file_id="aaaaaaaaaaaa")
    bad = _paper(file_id="bbbbbbbbbbbb", tags=None)  # tags=None breaks the write

    with pytest.raises(Exception):
        db.upsert_many([good, bad])

    assert db.count() == 0, "the first document must not survive the batch failing"
    db.close()
