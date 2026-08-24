"""The command layer: how a running service is set up to be read afterwards."""

from __future__ import annotations

import logging
from logging.handlers import RotatingFileHandler
from pathlib import Path

import pytest

from conftest import FailingLlmClient, FakeLlmClient
from sypy.cli import LOG_BACKUP_COUNT, LOG_MAX_BYTES, _configure


@pytest.fixture(autouse=True)
def _restore_logging():
    """Put the root logger back, so configuring it here cannot leak into other tests."""
    root = logging.getLogger()
    handlers, level = list(root.handlers), root.level
    yield
    root.handlers[:] = handlers
    root.setLevel(level)


def test_every_line_carries_a_timestamp(monkeypatch, capsys) -> None:
    """The watcher's log is read hours later, and often after a restart.

    An untimed line cannot say whether it is from this run or the one that
    crashed — which is exactly the question a restart loop raises.
    """
    monkeypatch.delenv("SYPY_LOG_FILE", raising=False)
    _configure(verbose=False)

    logging.getLogger("sypy.test").info("something happened")

    line = capsys.readouterr().err.strip()
    assert line.endswith("INFO something happened")
    assert line[:4].isdigit(), f"no timestamp: {line!r}"


def test_the_service_log_is_rotated(monkeypatch, tmp_path: Path) -> None:
    """A watcher left running for months must not be able to fill the disk."""
    log_file = tmp_path / "logs" / "sypy.log"
    monkeypatch.setenv("SYPY_LOG_FILE", str(log_file))

    _configure(verbose=False)

    handlers = logging.getLogger().handlers
    rotating = [h for h in handlers if isinstance(h, RotatingFileHandler)]
    assert len(rotating) == 1
    assert rotating[0].maxBytes == LOG_MAX_BYTES
    assert rotating[0].backupCount == LOG_BACKUP_COUNT


def test_the_log_is_not_also_written_somewhere_nothing_trims(
    monkeypatch, tmp_path: Path
) -> None:
    """Under launchd, stderr goes to a file that nothing rotates.

    Writing every line to both would leave that unbounded copy growing exactly
    as it did before the rotation was added, so the rotating handler replaces
    the stream one rather than joining it.
    """
    monkeypatch.setenv("SYPY_LOG_FILE", str(tmp_path / "sypy.log"))

    _configure(verbose=False)

    streams = [
        handler
        for handler in logging.getLogger().handlers
        if type(handler) is logging.StreamHandler
    ]
    assert streams == []


def test_the_log_file_is_written_where_it_was_asked_for(
    monkeypatch, tmp_path: Path
) -> None:
    log_file = tmp_path / "nested" / "sypy.log"
    monkeypatch.setenv("SYPY_LOG_FILE", str(log_file))
    _configure(verbose=False)

    logging.getLogger("test").warning("into the file")
    for handler in logging.getLogger().handlers:
        handler.flush()

    assert "into the file" in log_file.read_text(encoding="utf-8")


def test_a_record_carries_the_path_to_the_document(library) -> None:
    """A search result whose file cannot be opened is only half an answer."""
    from sypy.cli import _describe
    from sypy.db import Paper

    paper = Paper(
        file_id="aaa111aaa111",
        content_hash="sha-1",
        store_name="aaa111aaa111__AI__Transformers",
        document_name="vaswani_2017_attention.pdf",
        title="Attention Is All You Need",
        tags=["AI", "Transformers"],
    )

    record = _describe(library, paper)

    assert Path(record["document"]).is_absolute()
    assert record["document"].endswith(
        "store/aaa111aaa111__AI__Transformers/vaswani_2017_attention.pdf"
    )
    assert record["notes"].endswith("aaa111aaa111__AI__Transformers/notes.md")
    assert record["category"] == "AI/Transformers"


def test_note_path_never_opens_an_editor(library, monkeypatch, capsys) -> None:
    """A caller writing the notes itself would be stuck holding an editor open."""
    from sypy import cli
    from sypy.db import Paper

    paper = Paper(
        file_id="aaa111aaa111",
        content_hash="sha-1",
        store_name="aaa111aaa111__AI",
        document_name="paper.pdf",
        title="A Paper",
        tags=["AI"],
    )
    library.db.upsert(paper)
    library.document_dir(paper).mkdir(parents=True)
    library.close()

    monkeypatch.setenv("EDITOR", "vim")
    monkeypatch.setattr(
        cli.subprocess, "call", lambda *a, **k: pytest.fail("opened an editor")
    )

    cli.note("aaa111aaa111", library.root, True)

    assert capsys.readouterr().out.strip().endswith("notes.md")


def _crowded(library, count: int = 25):
    """A library with more documents than a default search will show."""
    from sypy.db import Paper

    library.db.upsert_many(
        [
            Paper(
                file_id=f"{i:012d}",
                content_hash=f"sha-{i}",
                store_name=f"{i:012d}__AI",
                document_name="paper.pdf",
                title=f"Paper {i}",
                tags=["AI"],
                keywords=["shared"],
            )
            for i in range(count)
        ]
    )
    root = library.root
    library.close()
    return root


def _invoke(root, *args):
    from typer.testing import CliRunner

    from sypy.cli import app

    return CliRunner().invoke(app, [*args, "--library", str(root)])


def test_a_capped_search_says_it_was_capped(library) -> None:
    """A cap that says nothing reads as the whole answer.

    The reader stops looking, and the document that was cut off is one they
    now believe the library does not hold.
    """
    root = _crowded(library)

    result = _invoke(root, "find", "shared", "--limit", "3")

    assert result.exit_code == 0, result.output
    assert len([line for line in result.stdout.splitlines() if line[:12].isdigit()]) == 3
    assert "more than 3 matched" in result.stderr
    assert "--limit 0" in result.stderr


def test_the_cap_notice_leaves_the_records_parseable(library) -> None:
    """It goes to stderr, so `--json` stdout is still only JSON."""
    import json

    root = _crowded(library)

    result = _invoke(root, "find", "shared", "--limit", "3", "--json")

    assert len(json.loads(result.stdout)) == 3
    assert "more than 3 matched" in result.stderr


def test_a_search_that_fits_says_nothing_about_a_cap(library) -> None:
    root = _crowded(library, count=3)

    result = _invoke(root, "find", "shared", "--limit", "3")

    assert result.exit_code == 0, result.output
    assert "more than" not in result.stderr


def test_no_limit_shows_everything(library) -> None:
    root = _crowded(library)

    result = _invoke(root, "find", "shared", "--limit", "0")

    assert "25 document(s)" in result.stdout
    assert "more than" not in result.stderr


def test_a_negative_limit_is_a_usage_error(library) -> None:
    """Not a DuckDB binder error surfacing from three layers down."""
    root = _crowded(library, count=1)

    result = _invoke(root, "find", "shared", "--limit", "-1")

    assert result.exit_code == 2
    assert "--limit" in result.stderr


# ---- re-asking for a category ----------------------------------------------


def _misfiled(library, tags=("Psychology", "Research Methods")):
    """A document filed where steering put it, with a real folder in the store."""
    from sypy.db import Paper

    paper = Paper(
        file_id="78c64b3b8ef6",
        content_hash="sha-kahn",
        store_name="78c64b3b8ef6__" + "__".join(tags),
        document_name="kahn_2025_successor-representations.pdf",
        title="Trial-by-trial learning of successor representations",
        authors=["Ari E. Kahn", "Nathaniel D. Daw"],
        year=2025,
        tags=list(tags),
        keywords=["successor representations", "temporal difference learning"],
    )
    library.db.upsert(paper)
    folder = library.document_dir(paper)
    folder.mkdir(parents=True)
    (folder / paper.document_name).write_bytes(b"%PDF-1.4 not a real pdf")
    (folder / "notes.md").write_text("kept beside it", encoding="utf-8")
    library.rebuild_tree()
    root = library.root
    library.close()
    return root


def _retag(root, fake, *args, **kwargs):
    """Invoke `retag` with the fake client standing in for the real one."""
    from typer.testing import CliRunner

    from sypy import cli

    original = cli._build
    cli._build = lambda *a, **k: (original(*a, **k)[0], fake)
    try:
        return CliRunner().invoke(
            cli.app, ["retag", *args, "--library", str(root)], **kwargs
        )
    finally:
        cli._build = original


def test_accepting_a_suggestion_moves_the_document(library, settings) -> None:
    """Tags, folder, link, and keywords all move together."""
    from sypy.library import Library

    root = _misfiled(library)
    fake = FakeLlmClient()

    result = _retag(root, fake, "78c64b3b8ef6", input="a\n")

    assert result.exit_code == 0, result.output
    with Library(root) as after:
        paper = after.db.get("78c64b3b8ef6")
        assert paper.tags == ["Cognitive Science", "Computational Modelling"]
        assert after.document_dir(paper).is_dir()
        assert (after.document_dir(paper) / "notes.md").exists(), "notes travel"
        assert "shared" in paper.keywords, "keywords were refreshed too"


def test_regenerating_never_offers_the_same_category_twice(library) -> None:
    """The whole point of asking again.

    Each round must carry what came before, or the model has no reason to
    answer differently and the loop cannot end except by cancelling.
    """
    root = _misfiled(library)
    fake = FakeLlmClient()

    result = _retag(root, fake, "78c64b3b8ef6", input="r\nr\na\n")

    assert result.exit_code == 0, result.output
    assert len(fake.suggestions) == 3
    assert fake.suggestions[0].rejected == []
    assert fake.suggestions[1].rejected == ["Cognitive Science/Computational Modelling"]
    assert fake.suggestions[2].rejected == [
        "Cognitive Science/Computational Modelling",
        "Neuroscience/Learning",
    ]


def test_the_model_is_told_where_the_document_sits_now(library) -> None:
    root = _misfiled(library)
    fake = FakeLlmClient()

    _retag(root, fake, "78c64b3b8ef6", input="c\n")

    call = fake.suggestions[0]
    assert call.current == "Psychology/Research Methods"
    assert "successor representations" in call.text, "the library's own keywords"
    assert "Ari E. Kahn" in call.text


def test_cancelling_changes_nothing(library) -> None:
    from sypy.library import Library

    root = _misfiled(library)

    result = _retag(root, FakeLlmClient(), "78c64b3b8ef6", input="c\n")

    assert result.exit_code == 0, "cancelling is a decision, not an error"
    with Library(root) as after:
        assert after.db.get("78c64b3b8ef6").tags == ["Psychology", "Research Methods"]


def test_no_one_at_the_keyboard_cancels_rather_than_crashing(library) -> None:
    """From cron, or with stdin closed, this must stop cleanly."""
    from sypy.library import Library

    root = _misfiled(library)

    result = _retag(root, FakeLlmClient(), "78c64b3b8ef6", input="")

    assert result.exit_code == 0, result.output
    assert "cancelled" in result.output
    with Library(root) as after:
        assert after.db.get("78c64b3b8ef6").tags == ["Psychology", "Research Methods"]


def test_a_document_whose_file_is_gone_still_gets_a_suggestion(library) -> None:
    """Its record is what the model reads from anyway."""
    root = _misfiled(library)
    from sypy.library import Library

    with Library(root) as lib:
        paper = lib.db.get("78c64b3b8ef6")
        (lib.document_dir(paper) / paper.document_name).unlink()
    fake = FakeLlmClient()

    result = _retag(root, fake, "78c64b3b8ef6", input="c\n")

    assert result.exit_code == 0, result.output
    assert "successor representations" in fake.suggestions[0].text


def test_an_unknown_id_is_reported_before_anything_is_asked(library) -> None:
    root = _misfiled(library)
    fake = FakeLlmClient()

    result = _retag(root, fake, "nosuchdocument", input="a\n")

    assert result.exit_code == 1
    assert fake.suggestions == [], "no request is paid for"


def test_a_failing_model_does_not_leave_the_document_half_moved(library) -> None:
    from sypy.library import Library

    root = _misfiled(library)

    result = _retag(root, FailingLlmClient(), "78c64b3b8ef6", input="a\n")

    assert result.exit_code == 1
    with Library(root) as after:
        assert after.db.get("78c64b3b8ef6").tags == ["Psychology", "Research Methods"]


def test_retag_with_a_category_never_asks_the_model(library) -> None:
    """The manual path is unchanged, and free."""
    from sypy.library import Library

    root = _misfiled(library)
    fake = FakeLlmClient()

    result = _retag(root, fake, "78c64b3b8ef6", "Cognitive Science/Computation")

    assert result.exit_code == 0, result.output
    assert fake.suggestions == []
    with Library(root) as after:
        paper = after.db.get("78c64b3b8ef6")
        assert paper.tags == ["Cognitive Science", "Computation"]
        assert paper.keywords == [
            "successor representations",
            "temporal difference learning",
        ], "a hand re-tag leaves what the library knew alone"


def test_only_our_own_lines_are_turned_up(monkeypatch, capsys) -> None:
    """`sypy retag` waits at a prompt; a request log lands in the middle of it.

    Named by what is ours rather than by what is noisy: the OpenAI SDK vendors
    its HTTP client, so the logger doing the narrating is called `httpx2` today
    and something else after the next release.
    """
    monkeypatch.delenv("SYPY_LOG_FILE", raising=False)
    _configure(verbose=False)

    logging.getLogger("httpx2._client").info("HTTP Request: POST ...")
    logging.getLogger("sypy.ingest").info("filed 3 documents")
    logging.getLogger("httpx2._client").warning("connection retried")

    err = capsys.readouterr().err
    assert "HTTP Request" not in err
    assert "filed 3 documents" in err
    assert "connection retried" in err, "warnings still come through"


def test_verbose_brings_everything_back(monkeypatch, capsys) -> None:
    monkeypatch.delenv("SYPY_LOG_FILE", raising=False)
    _configure(verbose=True)

    logging.getLogger("httpx2._client").info("HTTP Request: POST ...")

    assert "HTTP Request" in capsys.readouterr().err
