from __future__ import annotations

import asyncio
from pathlib import Path

import pytest
from conftest import FailingLlmClient, FakeLlmClient, write_pdf

import syp_prototype.watch as watch_module
from syp_prototype.config import Settings
from syp_prototype.watch import watch


@pytest.fixture(autouse=True)
def _fast_watcher(monkeypatch: pytest.MonkeyPatch) -> None:
    """Keep the real settle/rescan logic, just not the real wall-clock waits."""
    monkeypatch.setattr(watch_module, "SETTLE_SECONDS", 0.05)
    monkeypatch.setattr(watch_module, "IDLE_RESCAN_SECONDS", 0.05)


async def test_ingests_what_is_already_waiting(settings: Settings) -> None:
    write_pdf(settings.input_dir / "a.pdf", "attention")
    client = FakeLlmClient()

    reports = await asyncio.wait_for(
        watch(settings, client, max_passes=1), timeout=10
    )

    assert reports[0].processed == 1


async def test_a_pdf_arriving_later_triggers_a_pass(settings: Settings) -> None:
    client = FakeLlmClient()

    async def drop_a_paper_in() -> None:
        await asyncio.sleep(0.2)
        write_pdf(settings.input_dir / "late.pdf", "arrived late")

    reports, _ = await asyncio.wait_for(
        asyncio.gather(
            watch(settings, client, max_passes=1), drop_a_paper_in()
        ),
        timeout=10,
    )

    assert [Path(r.path).name for r in reports[0].ingested] == ["late.pdf"]


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
    settings: Settings,
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
        asyncio.gather(watch(settings, client, max_passes=1), keep_writing()),
        timeout=10,
    )

    assert client.size_when_called == path.stat().st_size, (
        "ingest ran while the file was still growing; the settle re-check did "
        "not hold the pass back"
    )


async def test_unchanged_input_is_not_retried_after_a_failure(
    settings: Settings,
) -> None:
    write_pdf(settings.input_dir / "a.pdf", "a")
    client = FailingLlmClient()

    # Two passes are requested but the input never changes, so the watcher must
    # idle after the first rather than burning calls on the same failure.
    with pytest.raises(asyncio.TimeoutError):
        await asyncio.wait_for(watch(settings, client, max_passes=2), timeout=1.0)

    assert client.calls == 1


async def test_a_failed_pass_does_not_stop_the_watcher(settings: Settings) -> None:
    write_pdf(settings.input_dir / "a.pdf", "a")
    client = FailingLlmClient()

    async def drop_a_second_paper_in() -> None:
        await asyncio.sleep(0.4)
        write_pdf(settings.input_dir / "b.pdf", "b")

    _, _ = await asyncio.wait_for(
        asyncio.gather(
            watch(settings, client, max_passes=2), drop_a_second_paper_in()
        ),
        timeout=10,
    )

    # The watcher survived the first failure and tried again once input changed.
    assert client.calls == 2


async def test_a_missing_input_folder_is_refused_up_front(tmp_path: Path) -> None:
    settings = Settings(
        input_dir=tmp_path / "nope", output_dir=tmp_path / "sorted"
    )

    with pytest.raises(NotADirectoryError):
        await watch(settings, FakeLlmClient(), max_passes=1)
