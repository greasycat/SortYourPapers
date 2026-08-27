from __future__ import annotations

from pathlib import Path

import pytest
from conftest import write_pdf, write_scanned_pdf

from sortyourpaperya.extract import ExtractionError, extract_paper_text


def test_extracts_the_text_layer(tmp_path: Path) -> None:
    path = write_pdf(tmp_path / "a.pdf", "Attention Is All You Need")

    paper = extract_paper_text(path, "id1", page_cutoff=1)

    assert "Attention Is All You Need" in paper.text
    assert paper.has_text_layer
    assert paper.pages_read == 1


def test_a_scanned_pdf_opens_but_reports_no_text_layer(tmp_path: Path) -> None:
    path = write_scanned_pdf(tmp_path / "scan.pdf")

    paper = extract_paper_text(path, "id1", page_cutoff=1)

    assert not paper.has_text_layer


def test_an_unreadable_pdf_is_an_extraction_error(tmp_path: Path) -> None:
    path = tmp_path / "broken.pdf"
    path.write_bytes(b"not a pdf at all")

    with pytest.raises(ExtractionError):
        extract_paper_text(path, "id1", page_cutoff=1)
