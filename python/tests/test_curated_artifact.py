from pathlib import Path

from syp_paperfetch.manifest import load_test_set
from syp_paperfetch.models import SelectionBucket


def test_committed_curated_artifact_is_populated_and_matches_json() -> None:
    base = Path(__file__).resolve().parents[2] / "assets" / "testsets" / "scijudgebench-diverse"
    toml_set = load_test_set(base.with_suffix(".toml"))
    json_set = load_test_set(base.with_suffix(".json"))

    assert toml_set.as_dict() == json_set.as_dict()
    assert toml_set.papers
    assert any(paper.selection_bucket is SelectionBucket.RANDOM for paper in toml_set.papers)
    assert all(paper.paper_url.startswith("https://arxiv.org/abs/") for paper in toml_set.papers)
    assert all(paper.pdf_url.startswith("https://arxiv.org/pdf/") for paper in toml_set.papers)
