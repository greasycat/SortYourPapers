"""The command layer: how a running service is set up to be read afterwards."""

from __future__ import annotations

import logging
from logging.handlers import RotatingFileHandler
from pathlib import Path

import pytest

from syp_prototype.cli import LOG_BACKUP_COUNT, LOG_MAX_BYTES, _configure


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

    logging.getLogger("test").info("something happened")

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
