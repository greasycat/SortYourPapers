from __future__ import annotations

from dataclasses import asdict, dataclass, field
from enum import StrEnum


@dataclass(slots=True)
class Candidate:
    arxiv_id: str
    title: str
    abstract_text: str
    category: str
    subcategory: str
    citations: int
    date: str | None
    source_splits: list[str] = field(default_factory=list)


class SelectionBucket(StrEnum):
    TOP = "top"
    BOTTOM = "bottom"
    RANDOM = "random"


@dataclass(slots=True)
class SamplingPolicy:
    top_n_per_category: int = 5
    bottom_n_per_category: int = 5
    random_n_per_category: int = 5
    random_seed: int = 1_511_510_650
    per_subcategory_cap: int = 2

    def as_dict(self) -> dict[str, int]:
        return asdict(self)


@dataclass(slots=True)
class CuratedPaper:
    paper_id: str
    arxiv_id: str
    canonical_pdf_url: str
    title: str
    category: str
    subcategory: str
    citations: int
    date: str | None
    abstract_excerpt: str
    selection_bucket: SelectionBucket
    sha256: str | None = None
    byte_size: int | None = None

    def as_dict(self) -> dict[str, object]:
        raw = asdict(self)
        raw["selection_bucket"] = self.selection_bucket.value
        return raw


@dataclass(slots=True)
class CuratedTestSet:
    set_id: str
    description: str
    source_dataset: str
    selection_policy: SamplingPolicy
    generated_at_ms: int
    papers: list[CuratedPaper]

    def as_dict(self) -> dict[str, object]:
        return {
            "id": self.set_id,
            "description": self.description,
            "source_dataset": self.source_dataset,
            "selection_policy": self.selection_policy.as_dict(),
            "generated_at_ms": self.generated_at_ms,
            "papers": [paper.as_dict() for paper in self.papers],
        }
