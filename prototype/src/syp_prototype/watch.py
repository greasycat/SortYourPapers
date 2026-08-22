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
from .library import FilingMode, Library
from .llm import LlmClient
from .watchlock import claim as claim_folders

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
    library: Library,
    *,
    mode: FilingMode = FilingMode.PREVIEW,
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

    # Claimed before anything else happens. Two watchers sharing either folder
    # file the same document twice, because each decides what is new before
    # either writes.
    claim = claim_folders(settings.input_dir, library.root)

    wake = asyncio.Event()
    observer = _start_observer(settings, wake)
    log.info(
        "watching %s -> %s (%d already in the library, mode=%s)",
        settings.input_dir,
        library.root,
        library.db.count(),
        mode.value,
    )

    reports: list[IngestReport] = []
    last_run: Snapshot | None = None
    try:
        while max_passes is None or len(reports) < max_passes:
            pending = _pending(settings, library)
            if not pending or pending == last_run:
                # Nothing to do, so hold no database lock while waiting or the
                # other `sypy` commands could never run against this library.
                library.release()
                await _wait_for_change(wake)
                continue

            library.release()
            await asyncio.sleep(SETTLE_SECONDS)
            if _pending(settings, library) != pending:
                # Still arriving. Re-measure rather than ingesting a partial copy.
                continue

            # Drop events this pass is about to cause.
            wake.clear()
            log.info("ingesting %d pending PDF(s)", len(pending))
            last_run = pending

            try:
                report = await ingest_folder(settings, client, library, mode=mode)
            except Exception as err:  # a bad pass must not kill the watcher
                log.error("ingest failed: %s; waiting for the next change", err)
                continue

            reports.append(report)
            if report.rescan and report.rescan.changed:
                log.info(
                    "%d stored file(s) changed on disk; hashes refreshed",
                    len(report.rescan.changed),
                )
            if report.rescan and report.rescan.missing:
                log.warning(
                    "%d document(s) missing from the store",
                    len(report.rescan.missing),
                )
            log.info(
                "%s %d document(s), %d already known, %d failed",
                "filed" if mode.writes else "would file",
                report.processed,
                report.skipped_already_known,
                len(report.failed),
            )
    finally:
        observer.stop()
        observer.join(timeout=5)
        claim.release()

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


def _pending(settings: Settings, library: Library) -> Snapshot:
    return snapshot_input(
        settings.input_dir,
        library.root,
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
