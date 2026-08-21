from __future__ import annotations

import json
from pathlib import Path

import pytest

from syp_prototype.config import MAX_TEXT_CHARS_PER_FILE
from syp_prototype.extract import PaperText
from syp_prototype.llm import LlmError, build_prompt, parse_response


def _paper(file_id: str, text: str = "some text") -> PaperText:
    return PaperText(
        file_id=file_id, path=Path(f"/tmp/{file_id}.pdf"), text=text, pages_read=1
    )


def test_prompt_truncates_per_file_text_so_a_big_batch_still_fits() -> None:
    batch = [_paper(f"f{index}", "x" * 100_000) for index in range(4)]

    prompt = build_prompt(batch)

    files = json.loads(prompt.split("files:\n", 1)[1])
    assert all(len(entry["text"]) <= MAX_TEXT_CHARS_PER_FILE for entry in files)


def test_parses_pairs_back_into_batch_order() -> None:
    batch = [_paper("a"), _paper("b")]
    content = json.dumps(
        {
            "pairs": [
                {
                    "file_id": "b",
                    "keywords": ["k2"],
                    "preliminary_categories_k_depth": "Systems",
                },
                {
                    "file_id": "a",
                    "keywords": ["k1"],
                    "preliminary_categories_k_depth": "AI",
                },
            ]
        }
    )

    pairs = parse_response(content, batch)

    assert [pair.file_id for pair in pairs] == ["a", "b"]
    assert pairs[0].preliminary_category == "AI"


def test_a_missing_file_is_rejected_rather_than_silently_dropped() -> None:
    batch = [_paper("a"), _paper("b")]
    content = json.dumps(
        {
            "pairs": [
                {
                    "file_id": "a",
                    "keywords": [],
                    "preliminary_categories_k_depth": "AI",
                }
            ]
        }
    )

    with pytest.raises(LlmError, match="missing file_id"):
        parse_response(content, batch)


def test_an_unrequested_file_is_rejected() -> None:
    content = json.dumps(
        {
            "pairs": [
                {
                    "file_id": "zzz",
                    "keywords": [],
                    "preliminary_categories_k_depth": "AI",
                }
            ]
        }
    )

    with pytest.raises(LlmError, match="unexpected file_id"):
        parse_response(content, [_paper("a")])


def test_malformed_json_is_an_llm_error() -> None:
    with pytest.raises(LlmError, match="invalid JSON"):
        parse_response("not json", [_paper("a")])
