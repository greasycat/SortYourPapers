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
