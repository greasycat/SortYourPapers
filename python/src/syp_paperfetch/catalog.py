from __future__ import annotations

import json
from collections.abc import Iterable
from dataclasses import dataclass, field
from pathlib import Path

from huggingface_hub import hf_hub_download

from .models import Candidate

DEFAULT_REPO_ID = "OpenMOSS-Team/SciJudgeBench"
DEFAULT_REVISION = "main"
DEFAULT_SPLIT_FILES = (
    ("train", "train.jsonl"),
    ("test", "test.jsonl"),
    ("test_ood_iclr", "test_ood_iclr.jsonl"),
    ("test_ood_year", "test_ood_year.jsonl"),
)


@dataclass(slots=True)
class DatasetSource:
    repo_id: str = DEFAULT_REPO_ID
    revision: str = DEFAULT_REVISION
    split_files: tuple[tuple[str, str], ...] = field(default_factory=lambda: DEFAULT_SPLIT_FILES)


def load_candidates(source: DatasetSource) -> list[Candidate]:
    by_arxiv_id: dict[str, Candidate] = {}

    for split_name, filename in source.split_files:
        local_path = Path(
            hf_hub_download(
                repo_id=source.repo_id,
                repo_type="dataset",
                filename=filename,
                revision=source.revision,
            )
        )
        for row in _load_rows(local_path):
            for candidate in _flatten_row(row, split_name):
                existing = by_arxiv_id.get(candidate.arxiv_id)
                if existing is None:
                    by_arxiv_id[candidate.arxiv_id] = candidate
                    continue
                _merge_candidate(existing, candidate)

    return sorted(by_arxiv_id.values(), key=lambda item: item.arxiv_id)


def _load_rows(path: Path) -> list[dict[str, object]]:
    raw = path.read_text(encoding="utf-8").strip()
    if not raw:
        return []
    if raw.startswith("["):
        loaded = json.loads(raw)
        if not isinstance(loaded, list):
            raise ValueError(f"expected list payload in {path}")
        return loaded
    return [json.loads(line) for line in raw.splitlines() if line.strip()]


def _flatten_row(row: dict[str, object], split_name: str) -> Iterable[Candidate]:
    for prefix in ("paper_a", "paper_b"):
        arxiv_id = _normalize_arxiv_id(_string(row.get(f"{prefix}_arxiv_id")))
        if arxiv_id is None:
            continue
        yield Candidate(
            arxiv_id=arxiv_id,
            title=_string(row.get(f"{prefix}_title")),
            abstract_text=_string(row.get(f"{prefix}_abstract")),
            category=_string(row.get(f"{prefix}_category")) or "uncategorized",
            subcategory=_string(row.get(f"{prefix}_subcategory")) or "uncategorized",
            citations=_int(row.get(f"{prefix}_citations")),
            date=_optional_string(row.get(f"{prefix}_date")),
            source_splits=[split_name],
        )


def _merge_candidate(existing: Candidate, incoming: Candidate) -> None:
    if not existing.title:
        existing.title = incoming.title
    if not existing.abstract_text:
        existing.abstract_text = incoming.abstract_text
    if existing.category == "uncategorized" and incoming.category != "uncategorized":
        existing.category = incoming.category
    if existing.subcategory == "uncategorized" and incoming.subcategory != "uncategorized":
        existing.subcategory = incoming.subcategory
    if existing.date is None:
        existing.date = incoming.date
    existing.citations = max(existing.citations, incoming.citations)
    for split_name in incoming.source_splits:
        if split_name not in existing.source_splits:
            existing.source_splits.append(split_name)
    existing.source_splits.sort()


def _normalize_arxiv_id(value: str | None) -> str | None:
    if value is None:
        return None
    normalized = (
        value.strip()
        .removeprefix("https://arxiv.org/abs/")
        .removeprefix("http://arxiv.org/abs/")
        .removeprefix("https://arxiv.org/pdf/")
        .removeprefix("http://arxiv.org/pdf/")
        .removesuffix(".pdf")
        .strip("/")
    )
    return normalized or None


def _string(value: object) -> str:
    if value is None:
        return ""
    if isinstance(value, str):
        return value.strip()
    return str(value).strip()


def _optional_string(value: object) -> str | None:
    text = _string(value)
    return text or None


def _int(value: object) -> int:
    if value is None:
        return 0
    if isinstance(value, bool):
        return int(value)
    if isinstance(value, int):
        return value
    if isinstance(value, float):
        return int(value)
    text = str(value).strip()
    return int(text) if text else 0
