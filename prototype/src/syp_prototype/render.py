"""Rendering the pages of a PDF that carries no text layer.

A scanned document opens fine and yields nothing to read. Its pages are pictures,
so the only way to learn what it is, is to look at them.

Uses `pdftoppm` from poppler, the same tool and the same arguments the Rust
pipeline uses, so the two read a scanned PDF identically and the project gains no
new dependency to install.
"""

from __future__ import annotations

import shutil
import subprocess
from dataclasses import dataclass
from pathlib import Path

# Enough to read body text on a page scanned at 300 DPI without sending pixels
# nobody looks at; a page comes out around 150-250KB.
RENDER_DPI = 150

# A page that has not rendered in this long is not going to.
RENDER_TIMEOUT_SECONDS = 60.0


class RenderError(RuntimeError):
    """Raised when a PDF's pages cannot be turned into images."""


@dataclass(frozen=True)
class PageImage:
    """One rendered page."""

    data: bytes
    media_type: str = "image/png"


def renderer_available() -> bool:
    """Whether `pdftoppm` can be found, so a caller can say so up front."""
    return shutil.which("pdftoppm") is not None


def render_pages(path: Path, page_cutoff: int) -> list[PageImage]:
    """Render the first `page_cutoff` pages of `path`.

    Stops at the first page that will not render, which is how a PDF shorter
    than the cutoff ends: the pages already rendered are returned.

    Raises:
        RenderError: if not even the first page renders.
    """
    images: list[PageImage] = []
    for page in range(1, page_cutoff + 1):
        try:
            images.append(_render_page(path, page))
        except RenderError:
            if not images:
                raise
            break
    return images


def _render_page(path: Path, page: int) -> PageImage:
    # With no output prefix pdftoppm writes the image to stdout; passing "-"
    # would create a file literally named "-.png" next to the PDF instead.
    try:
        result = subprocess.run(
            [
                "pdftoppm",
                "-png",
                "-r",
                str(RENDER_DPI),
                "-f",
                str(page),
                "-l",
                str(page),
                "-singlefile",
                str(path),
            ],
            capture_output=True,
            timeout=RENDER_TIMEOUT_SECONDS,
        )
    except FileNotFoundError as err:
        raise RenderError(
            "pdftoppm is needed to read scanned PDFs but was not found; "
            "install poppler (brew install poppler)"
        ) from err
    except subprocess.TimeoutExpired as err:
        raise RenderError(f"rendering page {page} of {path.name} timed out") from err

    if result.returncode != 0 or not result.stdout:
        detail = result.stderr.decode("utf-8", "replace").strip()
        raise RenderError(
            f"pdftoppm failed on page {page} of {path.name}"
            + (f": {detail}" if detail else "")
        )
    return PageImage(data=result.stdout)
