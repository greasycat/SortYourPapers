from pathlib import Path

from syp_paperfetch.catalog import DatasetSource, load_candidates


def test_load_candidates_flattens_pairs_and_dedupes(monkeypatch, tmp_path: Path) -> None:
    train = tmp_path / "train.jsonl"
    test = tmp_path / "test.jsonl"
    train.write_text(
        '{"paper_a_arxiv_id":"1234.5678","paper_a_title":"A","paper_a_category":"CS","paper_a_subcategory":"cs.AI","paper_a_citations":10}\n',
        encoding="utf-8",
    )
    test.write_text(
        '{"paper_b_arxiv_id":"1234.5678","paper_b_title":"A","paper_b_category":"CS","paper_b_subcategory":"cs.AI","paper_b_citations":25}\n',
        encoding="utf-8",
    )

    def fake_download(*, filename: str, **_: object) -> str:
        return str(tmp_path / filename)

    monkeypatch.setattr("syp_paperfetch.catalog.hf_hub_download", fake_download)

    candidates = load_candidates(
        DatasetSource(split_files=(("train", "train.jsonl"), ("test", "test.jsonl")))
    )

    assert len(candidates) == 1
    assert candidates[0].citations == 25
    assert candidates[0].source_splits == ["test", "train"]
