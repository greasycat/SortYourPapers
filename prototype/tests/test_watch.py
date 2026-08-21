from __future__ import annotations

import asyncio
import contextlib
from pathlib import Path

import pytest
from conftest import FailingLlmClient, FakeLlmClient, write_pdf

import syp_prototype.watch as watch_module
from syp_prototype.config import Settings
from syp_prototype.library import FilingMode, Library
from syp_prototype.watch import watch


@pytest.fixture(autouse=True)
def _fast_watcher(monkeypatch: pytest.MonkeyPatch) -> None:
    """Keep the real settle/rescan logic, just not the real wall-clock waits."""
    monkeypatch.setattr(watch_module, "SETTLE_SECONDS", 0.05)
    monkeypatch.setattr(watch_module, "IDLE_RESCAN_SECONDS", 0.05)


async def test_ingests_what_is_already_waiting(settings: Settings, library: Library) -> None:
    write_pdf(settings.input_dir / "a.pdf", "attention")
    client = FakeLlmClient()

    reports = await asyncio.wait_for(
        watch(settings, client, library, mode=FilingMode.MOVE, max_passes=1), timeout=10
    )

    assert reports[0].processed == 1


async def test_a_pdf_arriving_later_triggers_a_pass(settings: Settings, library: Library) -> None:
    client = FakeLlmClient()

    async def drop_a_paper_in() -> None:
        await asyncio.sleep(0.2)
        write_pdf(settings.input_dir / "late.pdf", "arrived late")

    reports, _ = await asyncio.wait_for(
        asyncio.gather(
            watch(settings, client, library, mode=FilingMode.MOVE, max_passes=1), drop_a_paper_in()
        ),
        timeout=10,
    )

    assert [f.source.name for f in reports[0].filed] == ["late.pdf"]


class _SizeRecordingClient(FakeLlmClient):
    """Records how big the watched file was at the moment ingest actually ran."""

    def __init__(self, path: Path) -> None:
        super().__init__()
        self._path = path
        self.size_when_called: int | None = None

    async def extract_keywords(self, batch):
        self.size_when_called = self._path.stat().st_size
        return await super().extract_keywords(batch)


async def test_a_still_growing_file_is_not_ingested_half_written(
    settings: Settings, library: Library
) -> None:
    path = write_pdf(settings.input_dir / "growing.pdf", "first chunk")
    client = _SizeRecordingClient(path)

    async def keep_writing() -> None:
        # Grow well past one settle window, so a watcher that skipped the
        # re-check would reach the model while the file was still arriving.
        for _ in range(5):
            await asyncio.sleep(0.03)
            path.write_bytes(path.read_bytes() + b"\x00" * 512)

    await asyncio.wait_for(
        asyncio.gather(
            watch(settings, client, library, mode=FilingMode.PREVIEW, max_passes=1), keep_writing()
        ),
        timeout=10,
    )

    assert client.size_when_called == path.stat().st_size, (
        "ingest ran while the file was still growing; the settle re-check did "
        "not hold the pass back"
    )


async def test_unchanged_input_is_not_retried_after_a_failure(
    settings: Settings, library: Library
) -> None:
    write_pdf(settings.input_dir / "a.pdf", "a")
    client = FailingLlmClient()

    # Two passes are requested but the input never changes, so the watcher must
    # idle after the first rather than burning calls on the same failure.
    with pytest.raises(asyncio.TimeoutError):
        await asyncio.wait_for(watch(settings, client, library, mode=FilingMode.MOVE, max_passes=2), timeout=1.0)

    assert client.calls == 1


async def test_a_failed_pass_does_not_stop_the_watcher(settings: Settings, library: Library) -> None:
    write_pdf(settings.input_dir / "a.pdf", "a")
    client = FailingLlmClient()

    async def drop_a_second_paper_in() -> None:
        await asyncio.sleep(0.4)
        write_pdf(settings.input_dir / "b.pdf", "b")

    _, _ = await asyncio.wait_for(
        asyncio.gather(
            watch(settings, client, library, mode=FilingMode.MOVE, max_passes=2), drop_a_second_paper_in()
        ),
        timeout=10,
    )

    # The watcher survived the first failure and tried again once input changed.
    assert client.calls == 2


async def test_a_missing_input_folder_is_refused_up_front(
    tmp_path: Path, library: Library
) -> None:
    settings = Settings(
        input_dir=tmp_path / "nope", output_dir=tmp_path / "sorted"
    )

    with pytest.raises(NotADirectoryError):
        await watch(settings, FakeLlmClient(), library, max_passes=1)


async def test_an_idle_watcher_does_not_hold_the_database_lock(
    settings: Settings, library: Library
) -> None:
    """A background service must not lock every other command out.

    DuckDB's lock is held per process, so this probes from a real subprocess: a
    second connection inside this one would succeed either way and prove
    nothing. The watcher is left running rather than bounded by max_passes,
    because the lock is only dropped on the idle path.
    """
    import subprocess
    import sys

    write_pdf(settings.input_dir / "a.pdf", "attention")
    watcher = asyncio.create_task(
        watch(settings, FakeLlmClient(), library, mode=FilingMode.COPY)
    )
    try:
        # Long enough for the first pass to finish and the watcher to settle
        # into its wait, which is where the lock should be dropped.
        await asyncio.sleep(0.6)
        assert not watcher.done(), "the watcher should still be running"

        probe = await asyncio.to_thread(
            subprocess.run,
            [
                sys.executable,
                "-c",
                "import duckdb;"
                f"c=duckdb.connect({str(library.db.path)!r});"
                "print(c.execute('SELECT count(*) FROM papers').fetchone()[0]);"
                "c.close()",
            ],
            capture_output=True,
            text=True,
            timeout=30,
        )
    finally:
        watcher.cancel()
        with contextlib.suppress(asyncio.CancelledError):
            await watcher

    assert probe.returncode == 0, (
        "another process could not open the library while the watcher idled:\n"
        f"{probe.stderr}"
    )
    assert probe.stdout.strip() == "1", probe.stdout
