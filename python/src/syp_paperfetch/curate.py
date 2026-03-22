from __future__ import annotations

import hashlib
import time
from collections import defaultdict
from collections.abc import Iterable

from .models import Candidate, CuratedPaper, CuratedTestSet, SamplingPolicy, SelectionBucket


def build_curated_test_set(
    candidates: list[Candidate],
    policy: SamplingPolicy,
) -> CuratedTestSet:
    grouped: dict[str, list[Candidate]] = defaultdict(list)
    for candidate in candidates:
        grouped[candidate.category or "uncategorized"].append(candidate)

    selected: list[CuratedPaper] = []
    seen_arxiv_ids: set[str] = set()

    for category in sorted(grouped):
        ranked = sorted(
            grouped[category],
            key=lambda item: (-item.citations, item.arxiv_id),
        )
        selected.extend(
            _pick_ranked(
                ranked,
                SelectionBucket.TOP,
                policy.top_n_per_category,
                policy.per_subcategory_cap,
                seen_arxiv_ids,
            )
        )
        selected.extend(
            _pick_ranked(
                list(reversed(ranked)),
                SelectionBucket.BOTTOM,
                policy.bottom_n_per_category,
                policy.per_subcategory_cap,
                seen_arxiv_ids,
            )
        )
        selected.extend(
            _pick_random(
                ranked,
                policy.random_n_per_category,
                policy.per_subcategory_cap,
                policy.random_seed,
                seen_arxiv_ids,
            )
        )

    selected.sort(
        key=lambda item: (
            item.category,
            _bucket_order(item.selection_bucket),
            -item.citations,
            item.paper_id,
        )
    )

    return CuratedTestSet(
        set_id="scijudgebench-diverse",
        description=(
            "Curated SciJudgeBench arXiv test set with top, bottom, and deterministic "
            "random citation samples per category."
        ),
        source_dataset="OpenMOSS-Team/SciJudgeBench",
        selection_policy=policy,
        generated_at_ms=int(time.time() * 1000),
        papers=selected,
    )


def _pick_ranked(
    ordered: Iterable[Candidate],
    bucket: SelectionBucket,
    limit: int,
    per_subcategory_cap: int,
    seen_arxiv_ids: set[str],
) -> list[CuratedPaper]:
    return _pick_candidates(ordered, bucket, limit, per_subcategory_cap, seen_arxiv_ids)


def _pick_random(
    ordered: list[Candidate],
    limit: int,
    per_subcategory_cap: int,
    seed: int,
    seen_arxiv_ids: set[str],
) -> list[CuratedPaper]:
    shuffled = sorted(
        ordered,
        key=lambda item: (
            _stable_random_key(item.category, item.arxiv_id, seed),
            item.arxiv_id,
        ),
    )
    return _pick_candidates(
        shuffled,
        SelectionBucket.RANDOM,
        limit,
        per_subcategory_cap,
        seen_arxiv_ids,
    )


def _pick_candidates(
    ordered: Iterable[Candidate],
    bucket: SelectionBucket,
    limit: int,
    per_subcategory_cap: int,
    seen_arxiv_ids: set[str],
) -> list[CuratedPaper]:
    if limit <= 0:
        return []

    picked: list[CuratedPaper] = []
    overflow: list[Candidate] = []
    subcategory_counts: dict[str, int] = defaultdict(int)

    for candidate in ordered:
        if candidate.arxiv_id in seen_arxiv_ids:
            continue
        subcategory = candidate.subcategory or "uncategorized"
        if subcategory_counts[subcategory] >= per_subcategory_cap:
            overflow.append(candidate)
            continue
        subcategory_counts[subcategory] += 1
        seen_arxiv_ids.add(candidate.arxiv_id)
        picked.append(_to_curated_paper(candidate, bucket))
        if len(picked) == limit:
            return picked

    for candidate in overflow:
        if candidate.arxiv_id in seen_arxiv_ids:
            continue
        seen_arxiv_ids.add(candidate.arxiv_id)
        picked.append(_to_curated_paper(candidate, bucket))
        if len(picked) == limit:
            break

    return picked


def _to_curated_paper(candidate: Candidate, bucket: SelectionBucket) -> CuratedPaper:
    return CuratedPaper(
        paper_id=f"arxiv-{candidate.arxiv_id.replace('/', '-')}",
        arxiv_id=candidate.arxiv_id,
        canonical_pdf_url=f"https://arxiv.org/pdf/{candidate.arxiv_id}.pdf",
        title=candidate.title,
        category=candidate.category or "uncategorized",
        subcategory=candidate.subcategory or "uncategorized",
        citations=candidate.citations,
        date=candidate.date,
        abstract_excerpt=_excerpt(candidate.abstract_text, 320),
        selection_bucket=bucket,
    )


def _excerpt(value: str, limit: int) -> str:
    text = value.strip()
    if len(text) <= limit:
        return text
    return f"{text[: limit - 1]}…"


def _stable_random_key(category: str, arxiv_id: str, seed: int) -> int:
    payload = f"{seed}:{category}:{arxiv_id}".encode("utf-8")
    return int(hashlib.sha256(payload).hexdigest(), 16)


def _bucket_order(bucket: SelectionBucket) -> int:
    return {
        SelectionBucket.TOP: 0,
        SelectionBucket.BOTTOM: 1,
        SelectionBucket.RANDOM: 2,
    }[bucket]
