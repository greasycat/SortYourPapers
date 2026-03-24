from __future__ import annotations

import hashlib
import json
import os
import shutil
import subprocess
import time
from dataclasses import dataclass
from pathlib import Path

import httpx

from .manifest import save_test_set
from .models import CuratedPaper, CuratedTestSet


@dataclass(slots=True)
class MaterializedPaper:
    paper_id: str
    arxiv_id: str
    source_url: str
    path: Path
    sha256: str
    byte_size: int
    downloaded: bool


@dataclass(slots=True)
class MaterializeReport:
    set_id: str
    cache_dir: Path
    papers: list[MaterializedPaper]


def materialize_test_set(
    test_set: CuratedTestSet,
    *,
    cache_root: Path | None = None,
    force_download: bool = False,
) -> MaterializeReport:
    cache_root = cache_root or default_cache_root()
    set_dir = cache_root / test_set.set_id
    files_dir = set_dir / "files"
    files_dir.mkdir(parents=True, exist_ok=True)

    state: dict[str, dict[str, object]] = {}
    state_path = set_dir / "state.json"
    if state_path.exists():
        state = json.loads(state_path.read_text(encoding="utf-8"))

    materialized: list[MaterializedPaper] = []
    manifest_copy = CuratedTestSet(
        set_id=test_set.set_id,
        description=test_set.description,
        source_dataset=test_set.source_dataset,
        selection_policy=test_set.selection_policy,
        generated_at_ms=test_set.generated_at_ms,
        papers=[
            CuratedPaper(
                paper_id=paper.paper_id,
                arxiv_id=paper.arxiv_id,
                title=paper.title,
                category=paper.category,
                subcategory=paper.subcategory,
                citations=paper.citations,
                date=paper.date,
                abstract_excerpt=paper.abstract_excerpt,
                selection_bucket=paper.selection_bucket,
                paper_url=paper.paper_url,
                pdf_url=paper.pdf_url,
                source_splits=list(paper.source_splits),
                sha256=paper.sha256,
                byte_size=paper.byte_size,
            )
            for paper in test_set.papers
        ],
    )

    with httpx.Client(timeout=60.0, follow_redirects=True) as client:
        for paper in manifest_copy.papers:
            target = files_dir / f"{paper.paper_id}.pdf"
            entry = _materialize_one(client, paper, target, force_download=force_download)
            materialized.append(entry)
            paper.sha256 = entry.sha256
            paper.byte_size = entry.byte_size
            state[paper.paper_id] = {
                "arxiv_id": paper.arxiv_id,
                "source_url": paper.pdf_url,
                "sha256": entry.sha256,
                "byte_size": entry.byte_size,
                "verified_at_ms": int(time.time() * 1000),
            }

    state_path.write_text(json.dumps(state, indent=2, sort_keys=True), encoding="utf-8")
    save_test_set(set_dir / "manifest.toml", manifest_copy)
    save_test_set(set_dir / "manifest.json", manifest_copy)

    return MaterializeReport(set_id=test_set.set_id, cache_dir=set_dir, papers=materialized)


def export_test_set(report: MaterializeReport, output_dir: Path) -> list[Path]:
    output_dir.mkdir(parents=True, exist_ok=True)
    exported: list[Path] = []
    for paper in report.papers:
        destination = output_dir / f"{paper.paper_id}.pdf"
        shutil.copy2(paper.path, destination)
        exported.append(destination)
    return exported


def default_cache_root() -> Path:
    xdg_cache_home = os.environ.get("XDG_CACHE_HOME")
    if xdg_cache_home:
        return Path(xdg_cache_home) / "sortyourpapers" / "testsets"
    return Path.home() / ".cache" / "sortyourpapers" / "testsets"


def _materialize_one(
    client: httpx.Client,
    paper: CuratedPaper,
    target: Path,
    *,
    force_download: bool,
) -> MaterializedPaper:
    if target.exists() and not force_download:
        current_hash = sha256_file(target)
        if paper.sha256 in (None, current_hash):
            return MaterializedPaper(
                paper_id=paper.paper_id,
                arxiv_id=paper.arxiv_id,
                source_url=paper.pdf_url,
                path=target,
                sha256=current_hash,
                byte_size=target.stat().st_size,
                downloaded=False,
            )

    response = client.get(paper.pdf_url)
    if response.status_code >= 400 or not _looks_like_pdf(response):
        if _render_paper_page_to_pdf(paper, target):
            current_hash = sha256_file(target)
            if paper.sha256 is not None and paper.sha256 != current_hash:
                raise ValueError(f"checksum mismatch for {paper.paper_id}")
            return MaterializedPaper(
                paper_id=paper.paper_id,
                arxiv_id=paper.arxiv_id,
                source_url=paper.pdf_url,
                path=target,
                sha256=current_hash,
                byte_size=target.stat().st_size,
                downloaded=True,
            )
        response.raise_for_status()

    target.write_bytes(response.content)
    current_hash = sha256_file(target)
    if paper.sha256 is not None and paper.sha256 != current_hash:
        raise ValueError(f"checksum mismatch for {paper.paper_id}")
    return MaterializedPaper(
        paper_id=paper.paper_id,
        arxiv_id=paper.arxiv_id,
        source_url=paper.pdf_url,
        path=target,
        sha256=current_hash,
        byte_size=target.stat().st_size,
        downloaded=True,
    )


def sha256_file(path: Path) -> str:
    hasher = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            hasher.update(chunk)
    return hasher.hexdigest()


def _looks_like_pdf(response: httpx.Response) -> bool:
    content_type = response.headers.get("content-type", "").lower()
    if "application/pdf" in content_type:
        return True
    return response.content.startswith(b"%PDF")


def _render_paper_page_to_pdf(paper: CuratedPaper, target: Path) -> bool:
    browser = _find_pdf_browser()
    if browser is None:
        return False

    html_url = _paper_html_url(paper.paper_url)
    completed = subprocess.run(
        [
            browser,
            "--headless",
            "--disable-gpu",
            "--no-pdf-header-footer",
            f"--print-to-pdf={target}",
            html_url,
        ],
        check=False,
        capture_output=True,
        text=True,
    )
    return completed.returncode == 0 and target.exists() and target.stat().st_size > 0


def _find_pdf_browser() -> str | None:
    for candidate in ("chromium", "chromium-browser", "google-chrome"):
        browser = shutil.which(candidate)
        if browser is not None:
            return browser
    return None


def _paper_html_url(paper_url: str) -> str:
    if "/abs/" in paper_url:
        html_url = paper_url.replace("/abs/", "/html/", 1)
        if not html_url.endswith("/"):
            html_url = f"{html_url}/"
        return html_url
    return paper_url
