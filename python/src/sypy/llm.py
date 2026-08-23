"""The one model call the tool makes: text in, keywords and a category out.

Kept behind a Protocol so the ingest pipeline can be exercised end to end
without a network or an API key.
"""

from __future__ import annotations

import base64
import json
from dataclasses import dataclass, field
from typing import Protocol, Sequence

from .budget import Budget
from .config import (
    DEFAULT_LLM_MAX_RETRIES,
    DEFAULT_LLM_TIMEOUT_SECONDS,
    MAX_TEXT_CHARS_PER_FILE,
    MAX_TOTAL_BATCH_TEXT_CHARS,
)
from .extract import PaperText
from .render import PageImage


class LlmError(RuntimeError):
    """Raised when the model call fails or its response cannot be used."""


@dataclass(frozen=True)
class KeywordPair:
    """What the model returns for a single paper.

    Title, authors, and year are what the symlink tree names links by, so they
    are asked for in the same request rather than a second one.
    """

    file_id: str
    keywords: list[str]
    preliminary_category: str
    title: str = ""
    authors: list[str] = field(default_factory=list)
    year: int | None = None


class LlmClient(Protocol):
    """Extracts keywords and a preliminary category for a batch of documents."""

    async def extract_keywords(
        self,
        batch: Sequence[PaperText],
        existing_categories: Sequence[str] = (),
    ) -> list[KeywordPair]: ...

    async def describe_pages(self, images: Sequence[PageImage]) -> str: ...


_SYSTEM_PROMPT = (
    "You read documents of any kind and label them. Return strict JSON only."
)

_VISION_SYSTEM_PROMPT = "You transcribe what a document says. Return plain text only."

# The result is not shown to anyone: it stands in for the text a scanned page
# does not carry, and is then labelled exactly like any other document's text.
# So it asks for the document's own content, not a description of the picture.
_VISION_PROMPT = (
    "These are the first pages of a document that carries no text layer.\n"
    "Write plain text that stands in for what the document says, so it can be "
    "catalogued.\n"
    "Include, when they are legible:\n"
    "- the title, on its own first line\n"
    "- the authors or the issuing organisation\n"
    "- the date or year\n"
    "- then one short paragraph saying what the document is and what it covers\n"
    "Rules:\n"
    "- Write the document's own content; do not describe the image or say what "
    "you can see\n"
    "- Leave out anything not legible rather than guessing at it\n"
    "- No markdown, no headings, no preamble"
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
                    "title": {"type": "string"},
                    "authors": {"type": "array", "items": {"type": "string"}},
                    "year": {"type": ["integer", "null"]},
                },
                "required": [
                    "file_id",
                    "keywords",
                    "preliminary_categories_k_depth",
                    "title",
                    "authors",
                    "year",
                ],
                "additionalProperties": False,
            },
        }
    },
    "required": ["pairs"],
    "additionalProperties": False,
}


def build_prompt(
    batch: Sequence[PaperText], existing_categories: Sequence[str] = ()
) -> str:
    """Build the user prompt for one batch.

    Each document's text is truncated so a large batch cannot blow the context
    window; the per-file share shrinks as the batch grows, exactly as the Rust
    pipeline does it.
    """
    existing_section = ""
    existing_rule = ""
    if existing_categories:
        # Without this every document invents its own taxonomy, so two utility
        # bills can land under unrelated top-level branches.
        existing_rule = (
            "- Prefer a path from existing_categories when the document "
            "plausibly belongs under one, matching it exactly; invent a new "
            "path only when none fits\n"
        )
        existing_section = (
            f"\n\nexisting_categories:\n{json.dumps(list(existing_categories))}"
        )

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
        '"preliminary_categories_k_depth":"...","title":"...",'
        '"authors":["..."],"year":2017}]}\n'
        "Rules:\n"
        f"- Return exactly {len(batch)} pairs\n"
        "- Include every file_id exactly once\n"
        "- Keep 5 to 12 keywords for each file\n"
        "- Keywords must be specific nouns or short noun phrases\n"
        "- `preliminary_categories_k_depth` is a plain-text category suggestion "
        "such as `Machine Learning/Transformers` or `Finance/Utility Bills`; "
        "approximate is fine\n"
        "- Documents are not always academic papers. Bills, receipts, manuals, "
        "contracts, and notes are all expected; categorize each on its own terms "
        "rather than forcing it into an academic subject\n"
        "- `title` is the document\'s title, or \"\" if the text does not show one\n"
        "- `authors` are full names in the order printed, or [] if none are shown; "
        "bills, receipts, and manuals usually have none\n"
        "- `year` is the year the document is dated, as an integer, or null if not shown\n"
        "- Do not guess title, authors, or year; leave them empty when unsure\n"
        f"{existing_rule}"
        "- No markdown\n\n"
        f"files:\n{json.dumps(files)}"
        f"{existing_section}"
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
        authors = pair.get("authors") or []
        if not isinstance(authors, list):
            raise LlmError(f"authors for {file_id!r} must be a list")
        parsed[file_id] = KeywordPair(
            file_id=file_id,
            keywords=[str(keyword) for keyword in keywords],
            preliminary_category=str(
                pair.get("preliminary_categories_k_depth") or ""
            ).strip(),
            title=str(pair.get("title") or "").strip(),
            authors=[str(author).strip() for author in authors if str(author).strip()],
            year=_coerce_year(pair.get("year")),
        )

    missing = expected - parsed.keys()
    if missing:
        raise LlmError(f"model response is missing file_id(s): {sorted(missing)}")

    # Answer in the batch's order so downstream records are stable.
    return [parsed[paper.file_id] for paper in batch]


def _coerce_year(value: object) -> int | None:
    """A publication year, or None when the model could not find a usable one."""
    try:
        year = int(str(value))
    except (TypeError, ValueError):
        return None
    # Anything outside this range is a page number or a DOI fragment, not a year.
    return year if 1400 <= year <= 2200 else None


class OpenAiClient:
    """`LlmClient` backed by the OpenAI chat completions API.

    The same model is used for text and for images; the OpenAI default this
    tool ships with reads both.

    Every request is bounded twice over. A `Budget` is consulted before the
    request is sent and refuses it once the rolling day is spent, which is what
    stops an unattended watcher restarting into the same folder forever. And the
    SDK is given an explicit retry count and timeout, so a request that is
    failing or hanging gives up instead of holding a pass open indefinitely.
    """

    def __init__(
        self,
        api_key: str,
        model: str,
        *,
        budget: Budget | None = None,
        max_retries: int = DEFAULT_LLM_MAX_RETRIES,
        timeout_seconds: float = DEFAULT_LLM_TIMEOUT_SECONDS,
    ) -> None:
        from openai import AsyncOpenAI

        self._client = AsyncOpenAI(
            api_key=api_key,
            # Left to itself the SDK retries on its own schedule; saying it
            # here means the ceiling is one number, in one place, that the
            # ledger's request count can be reasoned about against.
            max_retries=max_retries,
            timeout=timeout_seconds,
        )
        self._model = model
        self._budget = budget if budget is not None else Budget()

    def _spend(self, what: str) -> None:
        """Take one request out of the day's allowance before making it.

        Reserved rather than counted afterwards: four batches run at once, and
        a ceiling each of them checks before any of them records would let all
        four through on the last unit of allowance.
        """
        self._budget.check(what)
        self._budget.record(requests=1)

    def _record_tokens(self, response: object) -> None:
        """Add what the request actually cost, once it is known.

        One record covers the SDK's retries of the same request, which is what
        the retry ceiling is for: without it a single failing request could
        cost several times what the ledger was told.
        """
        usage = getattr(response, "usage", None)
        total = getattr(usage, "total_tokens", None)
        if isinstance(total, int) and total > 0:
            self._budget.record(requests=0, tokens=total)

    async def extract_keywords(
        self,
        batch: Sequence[PaperText],
        existing_categories: Sequence[str] = (),
    ) -> list[KeywordPair]:
        if not batch:
            return []
        self._spend(f"request for {len(batch)} document(s)")
        try:
            response = await self._client.chat.completions.create(
                model=self._model,
                messages=[
                    {"role": "system", "content": _SYSTEM_PROMPT},
                    {
                        "role": "user",
                        "content": build_prompt(batch, existing_categories),
                    },
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

        self._record_tokens(response)
        content = response.choices[0].message.content or ""
        return parse_response(content, batch)

    async def describe_pages(self, images: Sequence[PageImage]) -> str:
        """Read rendered pages and return text standing in for the document's own.

        Raises:
            LlmError: if the request fails or the model returns nothing usable.
        """
        if not images:
            raise LlmError("no rendered pages to read")
        self._spend(f"page-reading request for {len(images)} page(s)")

        content: list[dict] = [{"type": "text", "text": _VISION_PROMPT}]
        for image in images:
            encoded = base64.b64encode(image.data).decode("ascii")
            content.append(
                {
                    "type": "image_url",
                    "image_url": {
                        "url": f"data:{image.media_type};base64,{encoded}"
                    },
                }
            )

        try:
            response = await self._client.chat.completions.create(
                model=self._model,
                messages=[
                    {"role": "system", "content": _VISION_SYSTEM_PROMPT},
                    {"role": "user", "content": content},
                ],
            )
        except Exception as err:  # surface SDK/transport failures uniformly
            raise LlmError(f"page-reading request failed: {err}") from err

        self._record_tokens(response)
        text = (response.choices[0].message.content or "").strip()
        if not text:
            raise LlmError("the model returned no text for the rendered pages")
        return text
