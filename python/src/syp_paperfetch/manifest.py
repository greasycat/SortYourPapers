from __future__ import annotations

import tomllib
from pathlib import Path

import tomli_w

from .models import CuratedPaper, CuratedTestSet, SamplingPolicy, SelectionBucket


def load_test_set(path: Path) -> CuratedTestSet:
    raw = tomllib.loads(path.read_text(encoding="utf-8"))
    return CuratedTestSet(
        set_id=str(raw["id"]),
        description=str(raw["description"]),
        source_dataset=str(raw["source_dataset"]),
        selection_policy=SamplingPolicy(**raw["selection_policy"]),
        generated_at_ms=int(raw["generated_at_ms"]),
        papers=[
            CuratedPaper(
                paper_id=str(paper["paper_id"]),
                arxiv_id=str(paper["arxiv_id"]),
                canonical_pdf_url=str(paper["canonical_pdf_url"]),
                title=str(paper["title"]),
                category=str(paper["category"]),
                subcategory=str(paper["subcategory"]),
                citations=int(paper["citations"]),
                date=str(paper["date"]) if paper.get("date") else None,
                abstract_excerpt=str(paper["abstract_excerpt"]),
                selection_bucket=SelectionBucket(str(paper["selection_bucket"])),
                sha256=str(paper["sha256"]) if paper.get("sha256") else None,
                byte_size=int(paper["byte_size"]) if paper.get("byte_size") else None,
            )
            for paper in raw.get("papers", [])
        ],
    )


def save_test_set(path: Path, test_set: CuratedTestSet) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    payload = test_set.as_dict()
    path.write_text(tomli_w.dumps(payload), encoding="utf-8")
