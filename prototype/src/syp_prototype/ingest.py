"""Reading a folder of PDFs into labelled ingest records.

Discovery, extraction, and one batched model call per group of papers. Batches
run concurrently, capped the same way the Rust pipeline caps them, because the
stage is bound by round-trips rather than by tokens.
"""

from __future__ import annotations

import asyncio
import logging
from dataclasses import dataclass, field
from pathlib import Path
from typing import Sequence

from .config import MAX_CONCURRENT_REQUESTS, Settings
from .discovery import discover_pdfs, file_id
from .extract import ExtractionError, PaperText, extract_paper_text
from .llm import LlmClient, LlmError
from .store import IngestIndex, IngestRecord

log = logging.getLogger(__name__)


@dataclass
class IngestReport:
    """What one ingest pass did."""

    ingested: list[IngestRecord] = field(default_factory=list)
    skipped_already_ingested: int = 0
    skipped_oversized: list[Path] = field(default_factory=list)
    failed: list[tuple[Path, str]] = field(default_factory=list)

    @property
    def processed(self) -> int:
        return len(self.ingested)


async def ingest_folder(
    settings: Settings, client: LlmClient, index: IngestIndex
) -> IngestReport:
    """Ingest every not-yet-seen PDF in the input folder."""
    report = IngestReport()
    papers = _prepare_papers(settings, index, report)
    if not papers:
        return report

    batches = [
        papers[start : start + settings.keyword_batch_size]
        for start in range(0, len(papers), settings.keyword_batch_size)
    ]
    semaphore = asyncio.Semaphore(MAX_CONCURRENT_REQUESTS)

    async def run_batch(batch: Sequence[PaperText]):
        async with semaphore:
            return await client.extract_keywords(batch)

    results = await asyncio.gather(
        *(run_batch(batch) for batch in batches), return_exceptions=True
    )

    for batch, result in zip(batches, results):
        if isinstance(result, BaseException):
            # One failed batch must not discard the batches that succeeded, so
            # the failure is recorded per paper and the pass continues.
            reason = _describe(result)
            log.warning("batch of %d paper(s) failed: %s", len(batch), reason)
            report.failed.extend((paper.path, reason) for paper in batch)
            continue
        for paper, pair in zip(batch, result):
            record = IngestRecord(
                file_id=paper.file_id,
                path=str(paper.path),
                keywords=pair.keywords,
                preliminary_category=pair.preliminary_category,
                pages_read=paper.pages_read,
                chars_extracted=len(paper.text),
            )
            index.add(record)
            report.ingested.append(record)

    return report


def _prepare_papers(
    settings: Settings, index: IngestIndex, report: IngestReport
) -> list[PaperText]:
    """Discover, filter, and extract text for the papers worth a model call."""
    max_bytes = settings.max_file_size_mb * 1024 * 1024
    papers: list[PaperText] = []

    for candidate in discover_pdfs(settings.input_dir, recursive=settings.recursive):
        if _is_within(candidate.path, settings.output_dir):
            continue
        if candidate.size_bytes > max_bytes:
            report.skipped_oversized.append(candidate.path)
            continue

        try:
            identifier = file_id(candidate.path)
        except OSError as err:
            report.failed.append((candidate.path, f"could not read file: {err}"))
            continue

        if identifier in index:
            report.skipped_already_ingested += 1
            continue

        try:
            paper = extract_paper_text(
                candidate.path, identifier, settings.page_cutoff
            )
        except ExtractionError as err:
            report.failed.append((candidate.path, str(err)))
            continue

        if not paper.has_text_layer:
            # Scanned PDFs need an image-reading path the prototype does not
            # have; report them rather than sending an empty prompt.
            report.failed.append((candidate.path, "no text layer (scanned PDF?)"))
            continue

        papers.append(paper)

    return papers


def _describe(err: BaseException) -> str:
    return str(err) if isinstance(err, LlmError) else f"{type(err).__name__}: {err}"


def _is_within(path: Path, parent: Path) -> bool:
    return path == parent or parent in path.parents
