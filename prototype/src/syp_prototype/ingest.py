"""Reading a folder of PDFs into the library.

Discovery, extraction, one batched model call per group of papers, then filing:
each paper is moved into the flat store under its permanent id and linked into
the tag tree. Batches run concurrently, capped the way the Rust pipeline caps
them, because the stage is bound by round-trips rather than tokens.

Nothing is moved unless ``apply`` is set. A preview run reports exactly where
each paper would land, because moving a person's files is not undoable by
guessing.
"""

from __future__ import annotations

import asyncio
import logging
from dataclasses import dataclass, field
from pathlib import Path
from typing import Sequence

from .config import MAX_CONCURRENT_REQUESTS, Settings
from .db import Paper
from .discovery import discover_pdfs, file_id as content_hash_of
from .extract import ExtractionError, PaperText, extract_paper_text
from .library import Library, LibraryError, PlannedFiling
from .llm import KeywordPair, LlmClient, LlmError
from .naming import new_paper_id, split_category, store_name

log = logging.getLogger(__name__)


@dataclass
class IngestReport:
    """What one ingest pass did, or would have done."""

    filed: list[PlannedFiling] = field(default_factory=list)
    planned: list[PlannedFiling] = field(default_factory=list)
    skipped_already_known: int = 0
    skipped_oversized: list[Path] = field(default_factory=list)
    failed: list[tuple[Path, str]] = field(default_factory=list)

    @property
    def processed(self) -> int:
        return len(self.filed) or len(self.planned)


async def ingest_folder(
    settings: Settings,
    client: LlmClient,
    library: Library,
    *,
    apply: bool = False,
) -> IngestReport:
    """Ingest every not-yet-known PDF in the input folder."""
    report = IngestReport()
    prepared = _prepare_papers(settings, library, report)
    if not prepared:
        return report

    papers = [paper for paper, _ in prepared]
    hashes = {paper.file_id: digest for paper, digest in prepared}
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
            # One failed batch must not discard the batches that succeeded.
            reason = _describe(result)
            log.warning("batch of %d paper(s) failed: %s", len(batch), reason)
            report.failed.extend((paper.path, reason) for paper in batch)
            continue
        for paper_text, pair in zip(batch, result):
            _record(
                library,
                report,
                paper_text,
                pair,
                hashes[paper_text.file_id],
                apply=apply,
            )

    return report


def _record(
    library: Library,
    report: IngestReport,
    paper_text: PaperText,
    pair: KeywordPair,
    content_hash: str,
    *,
    apply: bool,
) -> None:
    """Turn one model answer into a library row and a filed file."""
    tags = split_category(pair.preliminary_category)
    paper = Paper(
        file_id=new_paper_id(),
        content_hash=content_hash,
        store_name="",  # set below, once the id and tags are both known
        original_name=paper_text.path.name,
        source_path=str(paper_text.path),
        size_bytes=paper_text.path.stat().st_size if paper_text.path.exists() else None,
        pages_read=paper_text.pages_read,
        title=pair.title or None,
        year=pair.year,
        tags=tags,
        authors=pair.authors,
        keywords=pair.keywords,
    )
    paper.store_name = store_name(
        paper.file_id, tags, suffix=paper_text.path.suffix or ".pdf"
    )

    if not apply:
        report.planned.append(library.plan_filing(paper, paper_text.path))
        return

    try:
        report.filed.append(library.file_paper(paper, paper_text.path))
    except (LibraryError, OSError) as err:
        report.failed.append((paper_text.path, str(err)))


def _prepare_papers(
    settings: Settings, library: Library, report: IngestReport
) -> list[tuple[PaperText, str]]:
    """Discover, filter, and extract text for the papers worth a model call."""
    max_bytes = settings.max_file_size_mb * 1024 * 1024
    prepared: list[tuple[PaperText, str]] = []

    for candidate in discover_pdfs(settings.input_dir, recursive=settings.recursive):
        if _is_within(candidate.path, library.root):
            continue
        if candidate.size_bytes > max_bytes:
            report.skipped_oversized.append(candidate.path)
            continue

        try:
            digest = content_hash_of(candidate.path)
        except OSError as err:
            report.failed.append((candidate.path, f"could not read file: {err}"))
            continue

        if library.db.find_by_content_hash(digest) is not None:
            report.skipped_already_known += 1
            continue

        try:
            paper = extract_paper_text(candidate.path, digest, settings.page_cutoff)
        except ExtractionError as err:
            report.failed.append((candidate.path, str(err)))
            continue

        if not paper.has_text_layer:
            # Scanned PDFs need an image-reading path the prototype does not
            # have; report them rather than sending an empty prompt.
            report.failed.append((candidate.path, "no text layer (scanned PDF?)"))
            continue

        prepared.append((paper, digest))

    return prepared


def _describe(err: BaseException) -> str:
    return str(err) if isinstance(err, LlmError) else f"{type(err).__name__}: {err}"


def _is_within(path: Path, parent: Path) -> bool:
    return path == parent or parent in path.parents
