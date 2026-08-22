from __future__ import annotations

from pathlib import Path

import pytest
from conftest import write_scanned_pdf

from syp_prototype.render import (
    RenderError,
    render_pages,
    renderer_available,
)

needs_poppler = pytest.mark.skipif(
    not renderer_available(), reason="pdftoppm (poppler) is not installed"
)


@needs_poppler
def test_renders_a_page_to_png_bytes(tmp_path: Path) -> None:
    path = write_scanned_pdf(tmp_path / "scan.pdf")

    images = render_pages(path, page_cutoff=1)

    assert len(images) == 1
    assert images[0].media_type == "image/png"
    assert images[0].data.startswith(b"\x89PNG"), "should be a real PNG"


@needs_poppler
def test_a_cutoff_past_the_end_stops_at_the_last_page(tmp_path: Path) -> None:
    # A one-page PDF asked for three pages yields the one it has, rather than
    # failing on the pages that do not exist.
    path = write_scanned_pdf(tmp_path / "scan.pdf")

    images = render_pages(path, page_cutoff=3)

    assert len(images) == 1


@needs_poppler
def test_writes_no_files_beside_the_pdf(tmp_path: Path) -> None:
    # pdftoppm writes to stdout only when given no output prefix; passing "-"
    # silently creates a file called "-.png" next to the document instead.
    path = write_scanned_pdf(tmp_path / "scan.pdf")

    render_pages(path, page_cutoff=1)

    assert sorted(p.name for p in tmp_path.iterdir()) == ["scan.pdf"]


@needs_poppler
def test_an_unreadable_pdf_is_a_render_error(tmp_path: Path) -> None:
    path = tmp_path / "broken.pdf"
    path.write_bytes(b"not a pdf at all")

    with pytest.raises(RenderError):
        render_pages(path, page_cutoff=1)


def test_a_missing_pdftoppm_says_what_to_install(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    import subprocess

    path = write_scanned_pdf(tmp_path / "scan.pdf")

    def absent(*args, **kwargs):
        raise FileNotFoundError("pdftoppm")

    monkeypatch.setattr(subprocess, "run", absent)

    with pytest.raises(RenderError, match="install poppler"):
        render_pages(path, page_cutoff=1)
