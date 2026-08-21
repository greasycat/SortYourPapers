from collections import Counter

from syp_paperfetch.curate import build_clustering_eval_set
from syp_paperfetch.models import Candidate, ClusteringEvalPolicy


def _candidates(subcategory: str, count: int) -> list[Candidate]:
    return [
        Candidate(
            f"{subcategory}.{index:04d}",
            f"Paper {index}",
            "abstract",
            "Computer Science",
            subcategory,
            citations=index,
            date="2024-01-01",
            source_splits=["train"],
        )
        for index in range(count)
    ]


def test_every_reference_label_carries_the_same_number_of_papers() -> None:
    candidates = _candidates("cs.LG", 12) + _candidates("cs.CV", 15) + _candidates("cs.CL", 11)

    test_set = build_clustering_eval_set(
        candidates,
        ClusteringEvalPolicy(subcategories=3, papers_per_subcategory=10),
        generated_at_ms=0,
    )

    counts = Counter(paper.subcategory for paper in test_set.papers)
    assert counts == {"cs.LG": 10, "cs.CV": 10, "cs.CL": 10}


def test_subcategories_that_cannot_fill_the_quota_are_left_out() -> None:
    candidates = _candidates("cs.LG", 10) + _candidates("cs.NI", 3)

    test_set = build_clustering_eval_set(
        candidates,
        ClusteringEvalPolicy(subcategories=5, papers_per_subcategory=10),
        generated_at_ms=0,
    )

    assert {paper.subcategory for paper in test_set.papers} == {"cs.LG"}


def test_the_largest_subcategories_win_the_available_slots() -> None:
    candidates = _candidates("cs.LG", 30) + _candidates("cs.CV", 20) + _candidates("cs.CL", 10)

    test_set = build_clustering_eval_set(
        candidates,
        ClusteringEvalPolicy(subcategories=2, papers_per_subcategory=10),
        generated_at_ms=0,
    )

    assert {paper.subcategory for paper in test_set.papers} == {"cs.LG", "cs.CV"}


def test_selection_is_deterministic_for_a_seed() -> None:
    candidates = _candidates("cs.LG", 40)
    policy = ClusteringEvalPolicy(subcategories=1, papers_per_subcategory=10, random_seed=99)

    first = build_clustering_eval_set(candidates, policy, generated_at_ms=0)
    second = build_clustering_eval_set(candidates, policy, generated_at_ms=0)

    assert [paper.paper_id for paper in first.papers] == [paper.paper_id for paper in second.papers]


def test_sampling_is_not_a_citation_ranking() -> None:
    candidates = _candidates("cs.LG", 40)

    test_set = build_clustering_eval_set(
        candidates,
        ClusteringEvalPolicy(subcategories=1, papers_per_subcategory=10),
        generated_at_ms=0,
    )

    chosen = {paper.arxiv_id for paper in test_set.papers}
    most_cited = {candidate.arxiv_id for candidate in sorted(candidates, key=lambda c: -c.citations)[:10]}
    assert chosen != most_cited, "a clustering set should hold typical papers, not only landmarks"
