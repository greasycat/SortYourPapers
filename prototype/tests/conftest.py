from __future__ import annotations

from pathlib import Path
from typing import Sequence

import pytest
from pypdf import PdfWriter

from syp_prototype.config import Settings
from syp_prototype.extract import PaperText
from syp_prototype.llm import KeywordPair, LlmError


class FakeLlmClient:
    """Answers every batch, recording what it was asked."""

    def __init__(
        self,
        category: str = "AI/Transformers",
        *,
        title: str = "Attention Is All You Need",
        authors: list[str] | None = None,
        year: int | None = 2017,
    ) -> None:
        self.category = category
        self.title = title
        self.authors = ["Ashish Vaswani"] if authors is None else authors
        self.year = year
        self.batches: list[list[str]] = []

    async def extract_keywords(self, batch: Sequence[PaperText]) -> list[KeywordPair]:
        self.batches.append([paper.file_id for paper in batch])
        return [
            KeywordPair(
                file_id=paper.file_id,
                keywords=["alpha", "beta"],
                preliminary_category=self.category,
                title=self.title,
                authors=list(self.authors),
                year=self.year,
            )
            for paper in batch
        ]


class FailingLlmClient:
    """Fails every batch, to exercise the error path."""

    def __init__(self, message: str = "rate limited") -> None:
        self.message = message
        self.calls = 0

    async def extract_keywords(self, batch: Sequence[PaperText]) -> list[KeywordPair]:
        self.calls += 1
        raise LlmError(self.message)


def write_pdf(path: Path, text: str) -> Path:
    """Write a single-page PDF carrying `text` as a real text layer."""
    # A minimal hand-built PDF is the only way to get an extractable text layer
    # without pulling in a rendering dependency just for the tests.
    stream = f"BT /F1 12 Tf 72 720 Td ({text}) Tj ET".encode("latin-1")
    objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] "
        b"/Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>",
        b"<< /Length " + str(len(stream)).encode() + b" >>\nstream\n" + stream + b"\nendstream",
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
    ]

    out = bytearray(b"%PDF-1.4\n")
    offsets = []
    for number, body in enumerate(objects, start=1):
        offsets.append(len(out))
        out += f"{number} 0 obj\n".encode() + body + b"\nendobj\n"

    xref_at = len(out)
    out += f"xref\n0 {len(objects) + 1}\n".encode()
    out += b"0000000000 65535 f \n"
    for offset in offsets:
        out += f"{offset:010d} 00000 n \n".encode()
    out += (
        f"trailer\n<< /Size {len(objects) + 1} /Root 1 0 R >>\nstartxref\n{xref_at}\n"
        "%%EOF\n"
    ).encode()

    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(bytes(out))
    return path


def write_scanned_pdf(path: Path) -> Path:
    """A valid PDF with no text layer, standing in for a scan."""
    writer = PdfWriter()
    writer.add_blank_page(width=612, height=792)
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("wb") as handle:
        writer.write(handle)
    return path


@pytest.fixture
def settings(tmp_path: Path) -> Settings:
    inbox = tmp_path / "inbox"
    inbox.mkdir()
    return Settings(
        input_dir=inbox,
        output_dir=inbox / "library",
        keyword_batch_size=2,
    )


@pytest.fixture
def library(settings: Settings):
    from syp_prototype.library import Library

    lib = Library(settings.output_dir)
    try:
        yield lib
    finally:
        lib.close()
