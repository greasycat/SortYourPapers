"""Reading a folder of documents into the library.

A pass runs in phases, and the split is deliberate: DuckDB allows one writing
process, so any stretch where this holds the lock is a stretch where no other
``sypy`` command can run. Everything slow is therefore done with no connection
open, and the database is visited in three short bursts between:

1. hash every candidate                     no database
2. **read**: which are already held, and how the library is organised
3. parse only the new ones                  no database
4. call the model                           no database
5. copy the files                           no database
6. **write**: record the whole batch at once
7. link them into the tree                  no database

Measured on twelve 4MB documents, that took the share of a pass with the lock
unavailable from 66% to 3%, and unlike the old shape it does not grow with file
size — the bursts are proportional to the library, not to the bytes read.

Model batches run concurrently, capped the way the Rust pipeline caps them,
because that stage is bound by round-trips rather than tokens. Nothing is
written unless the filing mode says so: rearranging a person's files is not
undoable by guessing.
"""

from __future__ import annotations

import asyncio
import logging

import duckdb
from dataclasses import dataclass, field, replace
from pathlib import Path
from typing import Sequence

from .config import MAX_CONCURRENT_REQUESTS, MAX_STEERING_CATEGORIES, Settings
from .db import Paper
from .discovery import discover_pdfs, file_id as content_hash_of
from .extract import ExtractionError, PaperText, extract_paper_text
from .library import FilingMode, Library, LibraryError, PlannedFiling, RescanReport
from .llm import KeywordPair, LlmClient, LlmError
from .render import RenderError, render_pages
from .naming import link_name, new_paper_id, split_category, store_name

log = logging.getLogger(__name__)


@dataclass
class IngestReport:
    """What one ingest pass did, or would have done."""

    filed: list[PlannedFiling] = field(default_factory=list)
    planned: list[PlannedFiling] = field(default_factory=list)
    skipped_already_known: int = 0
    skipped_oversized: list[Path] = field(default_factory=list)
    failed: list[tuple[Path, str]] = field(default_factory=list)
    # What reconciling the store against the database found on the way in.
    rescan: RescanReport | None = None

    @property
    def processed(self) -> int:
        return len(self.filed) or len(self.planned)


async def ingest_folder(
    settings: Settings,
    client: LlmClient,
    library: Library,
    *,
    mode: FilingMode = FilingMode.PREVIEW,
) -> IngestReport:
    """Ingest every not-yet-known document in the input folder.

    Runs in phases so the database is only open while it is being used. Reading
    files, parsing PDFs, calling the model, and copying files all happen with no
    connection held; the database is visited in three short bursts between them.
    DuckDB allows one writing process, so a pass that held its lock across the
    file work would shut every other `sypy` command out for the length of it.
    """
    report = IngestReport()
    report.rescan = _reconcile_store(library)

    # Phase 1 — hash every candidate. No database.
    candidates = _hash_candidates(settings, library, report)
    if not candidates:
        return report

    # Phase 2 — the read burst: what is already held, and how the library is
    # currently organised.
    known = library.db.known_content_hashes([digest for _, digest in candidates])
    existing = library.existing_categories(limit=MAX_STEERING_CATEGORIES)
    library.release()

    # Phase 3 — read only what is new. No database, and no wasted parsing of
    # documents the library already has. Scanned pages are read here too, so
    # by phase 4 every document has text regardless of where it came from.
    prepared = await _extract_new(candidates, known, settings, report, client)
    if not prepared:
        return report

    papers = [paper for paper, _ in prepared]
    hashes = {paper.file_id: digest for paper, digest in prepared}

    # Phase 4 — the model calls. No database.
    batches = [
        papers[start : start + settings.keyword_batch_size]
        for start in range(0, len(papers), settings.keyword_batch_size)
    ]
    semaphore = asyncio.Semaphore(MAX_CONCURRENT_REQUESTS)

    async def run_batch(batch: Sequence[PaperText]):
        async with semaphore:
            return await client.extract_keywords(batch, existing)

    results = await asyncio.gather(
        *(run_batch(batch) for batch in batches), return_exceptions=True
    )

    labelled: list[tuple[PaperText, KeywordPair]] = []
    for batch, result in zip(batches, results):
        if isinstance(result, BaseException):
            # One failed batch must not discard the batches that succeeded.
            reason = _describe(result)
            log.warning("batch of %d document(s) failed: %s", len(batch), reason)
            report.failed.extend((paper.path, reason) for paper in batch)
            continue
        labelled.extend(zip(batch, result))

    if not labelled:
        return report

    if not mode.writes:
        for paper_text, pair in labelled:
            paper = _describe_paper(paper_text, pair, hashes[paper_text.file_id])
            report.planned.append(library.plan_filing(paper, paper_text.path))
        return report

    # Phase 5 — copy the files. No database.
    placed: list[tuple[Paper, Path, Path]] = []
    for paper_text, pair in labelled:
        paper = _describe_paper(paper_text, pair, hashes[paper_text.file_id])
        try:
            target = library.place_file(paper, paper_text.path, mode)
        except (LibraryError, OSError) as err:
            report.failed.append((paper_text.path, str(err)))
            continue
        placed.append((paper, paper_text.path, target))

    # Phase 6 — the write burst: every document of the pass in one visit.
    library.record([paper for paper, _, _ in placed])
    library.release()

    # Phase 7 — link them into the tree. No database.
    taken: set[Path] = set()
    for paper, source, target in placed:
        report.filed.append(
            PlannedFiling(
                file_id=paper.file_id,
                source=source,
                store_path=target,
                link_path=library.link_paper(paper, taken),
            )
        )

    return report


def _describe_paper(
    paper_text: PaperText, pair: KeywordPair, content_hash: str
) -> Paper:
    """Build the database row for one labelled document. Pure computation."""
    tags = split_category(pair.preliminary_category)
    paper = Paper(
        file_id=new_paper_id(),
        content_hash=content_hash,
        store_name="",  # set below, once the id and tags are both known
        original_name=paper_text.path.name,
        source_path=str(paper_text.path),
        pages_read=paper_text.pages_read,
        from_page_images=paper_text.from_page_images,
        title=pair.title or None,
        year=pair.year,
        tags=tags,
        authors=pair.authors,
        keywords=pair.keywords,
    )
    # The folder carries the id and tags; the file inside carries the readable
    # name, so opening the folder shows something a person recognises.
    suffix = paper_text.path.suffix or ".pdf"
    paper.store_name = store_name(paper.file_id, tags, suffix="")
    paper.document_name = link_name(
        fallback=f"{paper.store_name}{suffix}",
        authors=paper.authors,
        year=paper.year,
        title=paper.title,
        suffix=suffix,
    )
    return paper


def _reconcile_store(library: Library) -> RescanReport | None:
    """Refresh recorded hashes before deciding what the library already holds.

    Whether a document is already known is read from those hashes, and a file
    edited in the store leaves its own stale — so without this an annotated copy
    arriving later is ingested a second time. Size and mtime gate the work, so a
    library nothing has touched costs one stat per document.

    A store that cannot be read is reported and stepped over rather than
    stopping the pass: failing to reconcile risks a duplicate, while refusing to
    run means nothing is filed at all.

    That includes the database being busy. `duckdb.IOException` is not an
    `OSError` — it derives from `OperationalError` — so catching `OSError`
    alone let the most likely failure of all, a lock held past the wait, kill
    the pass this docstring promises to survive.
    """
    try:
        return library.rescan()
    except (OSError, duckdb.Error) as err:
        log.warning("could not reconcile the store: %s; continuing", err)
        return None


def _hash_candidates(
    settings: Settings, library: Library, report: IngestReport
) -> list[tuple[Path, str]]:
    """Find the candidate documents and hash them. No database.

    Hashing is what identifies a document, so it has to happen before the
    library can say whether it already holds one. Parsing does not, and is far
    slower, so it waits until phase 3 when the known ones have been dropped.
    """
    max_bytes = settings.max_file_size_mb * 1024 * 1024
    candidates: list[tuple[Path, str]] = []

    for candidate in discover_pdfs(settings.input_dir, recursive=settings.recursive):
        if _is_within(candidate.path, library.root):
            continue
        if candidate.size_bytes > max_bytes:
            report.skipped_oversized.append(candidate.path)
            continue
        try:
            candidates.append((candidate.path, content_hash_of(candidate.path)))
        except OSError as err:
            report.failed.append((candidate.path, f"could not read file: {err}"))

    return candidates


async def _extract_new(
    candidates: list[tuple[Path, str]],
    known: set[str],
    settings: Settings,
    report: IngestReport,
    client: LlmClient,
) -> list[tuple[PaperText, str]]:
    """Read the candidates the library does not already hold. No database.

    A PDF with no text layer is a scan: it opens fine and yields nothing. Rather
    than reporting it as a failure, its pages are rendered and read, and the
    text that comes back stands in for the text it does not carry. From here on
    it is an ordinary document — labelled by the same batched call, steered by
    the same categories.
    """
    prepared: list[tuple[PaperText, str]] = []
    scanned: list[tuple[PaperText, str]] = []

    for path, digest in candidates:
        if digest in known:
            report.skipped_already_known += 1
            continue

        try:
            paper = extract_paper_text(path, digest, settings.page_cutoff)
        except ExtractionError as err:
            report.failed.append((path, str(err)))
            continue

        (prepared if paper.has_text_layer else scanned).append((paper, digest))

    if scanned:
        prepared.extend(await _read_scanned(scanned, settings, report, client))
    return prepared


async def _read_scanned(
    scanned: list[tuple[PaperText, str]],
    settings: Settings,
    report: IngestReport,
    client: LlmClient,
) -> list[tuple[PaperText, str]]:
    """Render and read documents that carry no text layer.

    One extra request per scanned document, capped like every other stage. A
    document whose pages will not render, or that the model cannot read, is
    reported and skipped rather than sent on with nothing to label.
    """
    semaphore = asyncio.Semaphore(MAX_CONCURRENT_REQUESTS)

    async def read(paper: PaperText) -> PaperText:
        images = await asyncio.to_thread(
            render_pages, paper.path, settings.page_cutoff
        )
        async with semaphore:
            text = await client.describe_pages(images)
        return replace(paper, text=text, from_page_images=True)

    log.info("reading %d scanned document(s) from rendered pages", len(scanned))
    results = await asyncio.gather(
        *(read(paper) for paper, _ in scanned), return_exceptions=True
    )

    read_papers: list[tuple[PaperText, str]] = []
    for (paper, digest), result in zip(scanned, results):
        if isinstance(result, (RenderError, LlmError)):
            report.failed.append((paper.path, f"scanned PDF: {result}"))
            continue
        if isinstance(result, BaseException):
            report.failed.append((paper.path, f"scanned PDF: {_describe(result)}"))
            continue
        read_papers.append((result, digest))
    return read_papers


def _describe(err: BaseException) -> str:
    return str(err) if isinstance(err, LlmError) else f"{type(err).__name__}: {err}"


def _is_within(path: Path, parent: Path) -> bool:
    return path == parent or parent in path.parents
