"""The library on disk: one flat store of files, and a symlink tree over it.

Every PDF lives in exactly one place, `store/<id>__<Tag>__<Tag>.pdf`. The folder
tree under `tree/` is made only of symlinks, so a paper can be re-tagged by
renaming one file and moving one link, and the tree can be thrown away and
rebuilt from the database whenever it drifts.

Links are relative, so the whole library can be moved without breaking.
"""

from __future__ import annotations

import os
import shutil
from dataclasses import dataclass
from enum import Enum
from pathlib import Path

from .db import Paper, PaperDb
from .naming import disambiguate, link_name, store_name

STORE_DIR = "store"
TREE_DIR = "tree"
DB_FILE = "papers.duckdb"


class FilingMode(str, Enum):
    """What filing is allowed to do to the source file.

    `COPY` exists so a real folder someone else owns — a Downloads folder, a
    shared drive — can be indexed without being rearranged underneath them.
    """

    PREVIEW = "preview"
    COPY = "copy"
    MOVE = "move"

    @property
    def writes(self) -> bool:
        return self is not FilingMode.PREVIEW


class LibraryError(RuntimeError):
    """Raised when the library on disk cannot be brought to the intended state."""


@dataclass(frozen=True)
class PlannedFiling:
    """What filing one paper would do, before anything is touched."""

    file_id: str
    source: Path
    store_path: Path
    link_path: Path

    def describe(self) -> str:
        return f"{self.source.name} -> {self.store_path.name} @ {self.link_path.parent}"


class Library:
    """The store folder, the symlink tree, and the database that describes them."""

    def __init__(self, root: Path) -> None:
        self.root = root
        self.store_dir = root / STORE_DIR
        self.tree_dir = root / TREE_DIR
        self.db = PaperDb(root / DB_FILE)

    def close(self) -> None:
        self.db.close()

    def release(self) -> None:
        """Drop the database lock while idle, so other commands can run."""
        self.db.release()

    def __enter__(self) -> "Library":
        return self

    def __exit__(self, *_exc: object) -> None:
        self.close()

    def store_path(self, paper: Paper) -> Path:
        return self.store_dir / paper.store_name

    def plan_filing(self, paper: Paper, source: Path) -> PlannedFiling:
        """Where a paper would land, without touching the filesystem."""
        target = self.store_dir / paper.store_name
        return PlannedFiling(
            file_id=paper.file_id,
            source=source,
            store_path=target,
            link_path=self._link_path(paper, taken=set()),
        )

    def file_paper(
        self, paper: Paper, source: Path, mode: FilingMode = FilingMode.MOVE
    ) -> PlannedFiling:
        """Put a paper into the store, record it, and link it into the tree.

        The file lands before the database write, so a crash leaves a file in
        the store with no row rather than a row pointing at nothing. The first
        is repairable by re-ingesting; the second is a dangling reference.

        `shutil` rather than `os.replace` because the source folder is often on
        a different filesystem than the library, which `os.replace` refuses.
        """
        if mode is FilingMode.PREVIEW:
            raise LibraryError("file_paper called in preview mode")

        target = self.store_dir / paper.store_name
        self.store_dir.mkdir(parents=True, exist_ok=True)
        if target.exists():
            raise LibraryError(f"store already holds {target.name}")

        if mode is FilingMode.COPY:
            shutil.copy2(source, target)
        else:
            shutil.move(str(source), str(target))
        self.db.upsert(paper)
        link = self._link(paper)
        return PlannedFiling(
            file_id=paper.file_id, source=source, store_path=target, link_path=link
        )

    def retag(self, file_id: str, tags: list[str]) -> Paper:
        """Give a paper new tags: rename it in the store and move its link.

        Raises:
            LibraryError: if the paper is unknown or its file is missing.
        """
        paper = self.db.get(file_id)
        if paper is None:
            raise LibraryError(f"no paper with id {file_id}")

        old_path = self.store_dir / paper.store_name
        new_name = store_name(paper.file_id, tags, suffix=old_path.suffix or ".pdf")
        new_path = self.store_dir / new_name

        if not old_path.exists():
            raise LibraryError(f"store is missing {paper.store_name}")
        if new_path != old_path and new_path.exists():
            raise LibraryError(f"store already holds {new_name}")

        self._unlink_existing(paper)
        if new_path != old_path:
            os.replace(old_path, new_path)
        self.db.set_tags(file_id, tags, new_name)

        updated = self.db.get(file_id)
        assert updated is not None  # just written
        self._link(updated)
        return updated

    def remove(self, file_id: str) -> Paper:
        """Take a document out of the library: link, stored file, and rows.

        The link goes first and the rows last, so an interruption leaves the
        library holding less than it claims rather than claiming more than it
        holds. A stale row pointing at a deleted file is the harder state to
        notice.

        Raises:
            LibraryError: if the document is unknown.
        """
        paper = self.db.get(file_id)
        if paper is None:
            raise LibraryError(f"no document with id {file_id}")

        self._unlink_existing(paper)
        (self.store_dir / paper.store_name).unlink(missing_ok=True)
        self.db.delete(file_id)
        return paper

    def existing_categories(self, limit: int | None = None) -> list[str]:
        """Category paths already in use, for steering a new document."""
        return self.db.tag_paths(limit=limit)

    def rebuild_tree(self) -> int:
        """Rebuild every symlink from the database, discarding the old tree.

        The tree holds nothing but links, so throwing it away costs nothing and
        is the simplest way to converge after edits made outside the tool.
        """
        if self.tree_dir.exists():
            _remove_tree(self.tree_dir)

        taken: set[Path] = set()
        linked = 0
        for paper in self.db.all_papers():
            if not (self.store_dir / paper.store_name).exists():
                continue
            self._link(paper, taken=taken)
            linked += 1
        return linked

    def missing_files(self) -> list[Paper]:
        """Papers the database knows about whose file is gone from the store."""
        return [
            paper
            for paper in self.db.all_papers()
            if not (self.store_dir / paper.store_name).exists()
        ]

    # ---- links -------------------------------------------------------------

    def _link_path(self, paper: Paper, taken: set[Path]) -> Path:
        directory = self.tree_dir.joinpath(*paper.tags) if paper.tags else self.tree_dir
        suffix = Path(paper.store_name).suffix or ".pdf"
        name = link_name(
            fallback=paper.store_name,
            authors=paper.authors,
            year=paper.year,
            title=paper.title,
            suffix=suffix,
        )
        candidate = directory / name
        if candidate in taken or (candidate.exists() and not candidate.is_symlink()):
            candidate = directory / disambiguate(name, paper.file_id)
        return candidate

    def _link(self, paper: Paper, taken: set[Path] | None = None) -> Path:
        taken = taken if taken is not None else set()
        link = self._link_path(paper, taken)
        link.parent.mkdir(parents=True, exist_ok=True)
        if link.is_symlink() or link.exists():
            link.unlink()
        target = self.store_dir / paper.store_name
        link.symlink_to(os.path.relpath(target, link.parent))
        taken.add(link)
        return link

    def _unlink_existing(self, paper: Paper) -> None:
        """Remove a paper's current link, and any folders it leaves empty."""
        link = self._link_path(paper, taken=set())
        if link.is_symlink() or link.exists():
            link.unlink()
        _prune_empty(link.parent, stop=self.tree_dir)


def _remove_tree(root: Path) -> None:
    """Delete a tree of symlinks and directories, never following the links."""
    for path in sorted(root.rglob("*"), key=lambda item: len(item.parts), reverse=True):
        if path.is_symlink() or path.is_file():
            path.unlink()
        elif path.is_dir():
            path.rmdir()
    root.rmdir()


def _prune_empty(directory: Path, stop: Path) -> None:
    current = directory
    while current != stop and stop in current.parents:
        try:
            current.rmdir()
        except OSError:
            return
        current = current.parent
