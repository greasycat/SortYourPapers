from syp_paperfetch.curate import build_curated_test_set
from syp_paperfetch.models import Candidate, SamplingPolicy, SelectionBucket


def test_sampling_builds_top_bottom_and_random() -> None:
    candidates = [
        Candidate("1001.0001", "A", "abstract", "CS", "cs.AI", 100, "2024-01-01", ["train"]),
        Candidate("1001.0002", "B", "abstract", "CS", "cs.AI", 50, "2024-01-01", ["train"]),
        Candidate("1001.0003", "C", "abstract", "CS", "cs.LG", 20, "2024-01-01", ["train"]),
        Candidate("1001.0004", "D", "abstract", "CS", "cs.CL", 10, "2024-01-01", ["train"]),
        Candidate("1001.0005", "E", "abstract", "CS", "cs.IR", 1, "2024-01-01", ["train"]),
    ]

    test_set = build_curated_test_set(
        candidates,
        SamplingPolicy(
            top_n_per_category=1,
            bottom_n_per_category=1,
            random_n_per_category=1,
            random_seed=7,
            per_subcategory_cap=1,
        ),
    )

    assert [paper.selection_bucket for paper in test_set.papers] == [
        SelectionBucket.TOP,
        SelectionBucket.BOTTOM,
        SelectionBucket.RANDOM,
    ]
    assert all(paper.paper_url.startswith("https://arxiv.org/abs/") for paper in test_set.papers)
    assert all(paper.pdf_url.startswith("https://arxiv.org/pdf/") for paper in test_set.papers)
    assert all(paper.source_splits == ["train"] for paper in test_set.papers)


def test_random_sampling_is_deterministic_and_unique() -> None:
    candidates = [
        Candidate(
            f"1001.{index:04d}",
            f"Paper {index}",
            "abstract",
            "CS",
            f"cs.{index % 5}",
            100 - index,
            "2024-01-01",
            ["train"],
        )
        for index in range(12)
    ]

    policy = SamplingPolicy(
        top_n_per_category=2,
        bottom_n_per_category=2,
        random_n_per_category=3,
        random_seed=11,
        per_subcategory_cap=1,
    )

    left = build_curated_test_set(candidates, policy)
    right = build_curated_test_set(candidates, policy)

    left_random = [paper.arxiv_id for paper in left.papers if paper.selection_bucket is SelectionBucket.RANDOM]
    right_random = [paper.arxiv_id for paper in right.papers if paper.selection_bucket is SelectionBucket.RANDOM]

    assert left_random == right_random
    assert len({paper.arxiv_id for paper in left.papers}) == len(left.papers)
