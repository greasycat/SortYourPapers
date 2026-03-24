from pathlib import Path

from syp_paperfetch.manifest import load_test_set, save_test_set
from syp_paperfetch.models import CuratedPaper, CuratedTestSet, SamplingPolicy, SelectionBucket


def test_manifest_round_trip(tmp_path: Path) -> None:
    test_set = CuratedTestSet(
        set_id="demo",
        description="Demo",
        source_dataset="OpenMOSS-Team/SciJudgeBench",
        selection_policy=SamplingPolicy(),
        generated_at_ms=123,
        papers=[
            CuratedPaper(
                paper_id="arxiv-1234.5678",
                arxiv_id="1234.5678",
                title="Title",
                category="CS",
                subcategory="cs.AI",
                citations=10,
                date="2024-01-01",
                abstract_excerpt="Excerpt",
                selection_bucket=SelectionBucket.TOP,
                paper_url="https://arxiv.org/abs/1234.5678",
                pdf_url="https://arxiv.org/pdf/1234.5678.pdf",
                source_splits=["test", "train"],
                sha256="abc",
                byte_size=42,
            )
        ],
    )

    for path in (tmp_path / "manifest.toml", tmp_path / "manifest.json"):
        save_test_set(path, test_set)
        loaded = load_test_set(path)

        assert loaded.as_dict() == test_set.as_dict()
