"""The one model call the prototype makes: text in, keywords and a category out.

Kept behind a Protocol so the ingest pipeline can be exercised end to end
without a network or an API key.
"""

from __future__ import annotations

import json
from dataclasses import dataclass
from typing import Protocol, Sequence

from .config import MAX_TEXT_CHARS_PER_FILE, MAX_TOTAL_BATCH_TEXT_CHARS
from .extract import PaperText


class LlmError(RuntimeError):
    """Raised when the model call fails or its response cannot be used."""


@dataclass(frozen=True)
class KeywordPair:
    """What the model returns for a single paper."""

    file_id: str
    keywords: list[str]
    preliminary_category: str


class LlmClient(Protocol):
    """Extracts keywords and a preliminary category for a batch of papers."""

    async def extract_keywords(
        self, batch: Sequence[PaperText]
    ) -> list[KeywordPair]: ...


_SYSTEM_PROMPT = (
    "You read academic papers and label them. Return strict JSON only."
)

_RESPONSE_SCHEMA = {
    "type": "object",
    "properties": {
        "pairs": {
            "type": "array",
            "items": {
                "type": "object",
                "properties": {
                    "file_id": {"type": "string"},
                    "keywords": {"type": "array", "items": {"type": "string"}},
                    "preliminary_categories_k_depth": {"type": "string"},
                },
                "required": [
                    "file_id",
                    "keywords",
                    "preliminary_categories_k_depth",
                ],
                "additionalProperties": False,
            },
        }
    },
    "required": ["pairs"],
    "additionalProperties": False,
}


def build_prompt(batch: Sequence[PaperText]) -> str:
    """Build the user prompt for one batch.

    Each paper's text is truncated so a large batch cannot blow the context
    window; the per-file share shrinks as the batch grows, exactly as the Rust
    pipeline does it.
    """
    per_file_limit = min(
        MAX_TEXT_CHARS_PER_FILE, max(400, MAX_TOTAL_BATCH_TEXT_CHARS // max(len(batch), 1))
    )
    files = [
        {"file_id": paper.file_id, "text": paper.text[:per_file_limit]}
        for paper in batch
    ]
    return (
        "Return JSON with this exact schema:\n"
        '{"pairs":[{"file_id":"...","keywords":["..."],'
        '"preliminary_categories_k_depth":"..."}]}\n'
        "Rules:\n"
        f"- Return exactly {len(batch)} pairs\n"
        "- Include every file_id exactly once\n"
        "- Keep 5 to 12 keywords for each file\n"
        "- Keywords must be specific nouns or short noun phrases\n"
        "- `preliminary_categories_k_depth` is a plain-text category suggestion "
        "such as `Machine Learning/Transformers`; approximate is fine\n"
        "- No markdown\n\n"
        f"files:\n{json.dumps(files)}"
    )


def parse_response(content: str, batch: Sequence[PaperText]) -> list[KeywordPair]:
    """Parse and validate a model response against the batch it answered.

    Raises:
        LlmError: if the JSON is malformed, or the pairs do not cover exactly
            the requested files.
    """
    try:
        payload = json.loads(content)
    except json.JSONDecodeError as err:
        raise LlmError(f"model returned invalid JSON: {err}") from err

    pairs = payload.get("pairs")
    if not isinstance(pairs, list):
        raise LlmError("model response has no `pairs` array")

    expected = {paper.file_id for paper in batch}
    parsed: dict[str, KeywordPair] = {}
    for pair in pairs:
        if not isinstance(pair, dict):
            raise LlmError("each pair must be an object")
        file_id = pair.get("file_id")
        if file_id not in expected:
            raise LlmError(f"unexpected file_id {file_id!r} in response")
        if file_id in parsed:
            raise LlmError(f"duplicate file_id {file_id!r} in response")
        keywords = pair.get("keywords") or []
        if not isinstance(keywords, list):
            raise LlmError(f"keywords for {file_id!r} must be a list")
        parsed[file_id] = KeywordPair(
            file_id=file_id,
            keywords=[str(keyword) for keyword in keywords],
            preliminary_category=str(
                pair.get("preliminary_categories_k_depth") or ""
            ).strip(),
        )

    missing = expected - parsed.keys()
    if missing:
        raise LlmError(f"model response is missing file_id(s): {sorted(missing)}")

    # Answer in the batch's order so downstream records are stable.
    return [parsed[paper.file_id] for paper in batch]


class OpenAiClient:
    """`LlmClient` backed by the OpenAI chat completions API."""

    def __init__(self, api_key: str, model: str) -> None:
        from openai import AsyncOpenAI

        self._client = AsyncOpenAI(api_key=api_key)
        self._model = model

    async def extract_keywords(self, batch: Sequence[PaperText]) -> list[KeywordPair]:
        if not batch:
            return []
        try:
            response = await self._client.chat.completions.create(
                model=self._model,
                messages=[
                    {"role": "system", "content": _SYSTEM_PROMPT},
                    {"role": "user", "content": build_prompt(batch)},
                ],
                response_format={
                    "type": "json_schema",
                    "json_schema": {
                        "name": "keyword_batch_response",
                        "schema": _RESPONSE_SCHEMA,
                        "strict": True,
                    },
                },
            )
        except Exception as err:  # surface SDK/transport failures uniformly
            raise LlmError(f"model request failed: {err}") from err

        content = response.choices[0].message.content or ""
        return parse_response(content, batch)
