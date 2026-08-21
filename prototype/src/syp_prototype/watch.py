"""Long-running watcher that ingests the input folder as papers arrive.

Filesystem events are a wake-up hint only: the folder scan is the single source
of truth for what needs ingesting. That keeps the loop correct when events are
coalesced, missed, or caused by the pipeline's own writes.
"""

from __future__ import annotations

import asyncio
import logging
from pathlib import Path

from watchdog.events import FileSystemEventHandler
from watchdog.observers import Observer

from .config import Settings
from .discovery import Snapshot, snapshot_input
from .ingest import IngestReport, ingest_folder
from .llm import LlmClient
from .store import INGEST_INDEX_FILE, IngestIndex

log = logging.getLogger(__name__)

# How long the folder must stay unchanged before a run starts, so PDFs still
# being copied in are not ingested half-written.
SETTLE_SECONDS = 3.0

# Rescan interval when no event arrives, so a missed event delays a run rather
# than stalling the watcher.
IDLE_RESCAN_SECONDS = 60.0


class _WakeHandler(FileSystemEventHandler):
    """Turns any filesystem event into a single wake-up."""

    def __init__(self, wake: asyncio.Event, loop: asyncio.AbstractEventLoop) -> None:
        self._wake = wake
        self._loop = loop

    def on_any_event(self, event) -> None:
        # Watchdog calls this from its own thread; hop back to the loop thread.
        self._loop.call_soon_threadsafe(self._wake.set)


async def watch(
    settings: Settings,
    client: LlmClient,
    *,
    max_passes: int | None = None,
) -> list[IngestReport]:
    """Ingest the input folder whenever new PDFs settle in it.

    Runs until cancelled. A failed pass is reported and the watcher keeps going,
    but the same unchanged input is not retried: the files stay put until they
    change or a new PDF arrives.

    Args:
        max_passes: stop after this many ingest passes. For tests; ``None``
            runs forever.
    """
    if not settings.input_dir.is_dir():
        raise NotADirectoryError(
            f"watch input folder does not exist: {settings.input_dir}"
        )

    index = IngestIndex(settings.output_dir / INGEST_INDEX_FILE)
    wake = asyncio.Event()
    observer = _start_observer(settings, wake)
    log.info(
        "watching %s -> %s (%d already ingested)",
        settings.input_dir,
        settings.output_dir,
        len(index),
    )

    reports: list[IngestReport] = []
    last_run: Snapshot | None = None
    try:
        while max_passes is None or len(reports) < max_passes:
            pending = _pending(settings)
            if not pending or pending == last_run:
                await _wait_for_change(wake)
                continue

            await asyncio.sleep(SETTLE_SECONDS)
            if _pending(settings) != pending:
                # Still arriving. Re-measure rather than ingesting a partial copy.
                continue

            # Drop events this pass is about to cause.
            wake.clear()
            log.info("ingesting %d pending PDF(s)", len(pending))
            last_run = pending

            try:
                report = await ingest_folder(settings, client, index)
            except Exception as err:  # a bad pass must not kill the watcher
                log.error("ingest failed: %s; waiting for the next change", err)
                continue

            reports.append(report)
            log.info(
                "ingested %d paper(s), %d skipped, %d failed",
                report.processed,
                report.skipped_already_ingested,
                len(report.failed),
            )
    finally:
        observer.stop()
        observer.join(timeout=5)

    return reports


def _start_observer(settings: Settings, wake: asyncio.Event) -> Observer:
    observer = Observer()
    observer.schedule(
        _WakeHandler(wake, asyncio.get_running_loop()),
        str(settings.input_dir),
        recursive=settings.recursive,
    )
    observer.start()
    return observer


def _pending(settings: Settings) -> Snapshot:
    return snapshot_input(
        settings.input_dir,
        settings.output_dir,
        recursive=settings.recursive,
        max_file_size_mb=settings.max_file_size_mb,
    )


async def _wait_for_change(wake: asyncio.Event) -> None:
    """Wait for the next event, giving up after the idle interval so we rescan."""
    try:
        await asyncio.wait_for(wake.wait(), timeout=IDLE_RESCAN_SECONDS)
    except asyncio.TimeoutError:
        return
    finally:
        wake.clear()
