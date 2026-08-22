"""Reading the first pages of a PDF into text the model can work with."""

from __future__ import annotations

import re
from dataclasses import dataclass
from pathlib import Path

from pypdf import PdfReader

_WHITESPACE = re.compile(r"\s+")


class ExtractionError(RuntimeError):
    """Raised when a PDF cannot be opened or read at all."""


@dataclass(frozen=True)
class PaperText:
    """Text pulled from one PDF, ready to hand to the model."""

    file_id: str
    path: Path
    text: str
    pages_read: int
    # Whether the text was read off rendered page images rather than a text
    # layer. Worth carrying: everything downstream is then a model's reading of
    # a picture, not the document's own words.
    from_page_images: bool = False

    @property
    def has_text_layer(self) -> bool:
        """Whether the PDF carried any extractable text.

        A scanned paper opens fine and yields nothing, which is a different
        problem from a corrupt file and is reported separately.
        """
        return bool(self.text)


def extract_paper_text(path: Path, file_id: str, page_cutoff: int) -> PaperText:
    """Extract text from the first ``page_cutoff`` pages of ``path``.

    Raises:
        ExtractionError: if the PDF cannot be opened or its pages cannot be read.
    """
    try:
        reader = PdfReader(str(path))
        pages = reader.pages[:page_cutoff]
        chunks = [page.extract_text() or "" for page in pages]
    except ExtractionError:
        raise
    except Exception as err:  # pypdf raises a wide range of parse errors
        raise ExtractionError(f"could not read {path.name}: {err}") from err

    return PaperText(
        file_id=file_id,
        path=path,
        text=_normalize(" ".join(chunks)),
        pages_read=len(chunks),
    )


def _normalize(text: str) -> str:
    """Collapse the whitespace PDF extraction leaves behind."""
    return _WHITESPACE.sub(" ", text).strip()
