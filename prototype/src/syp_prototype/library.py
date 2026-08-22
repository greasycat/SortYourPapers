"""The library on disk: a folder per document, and a symlink tree over them.

Every document has exactly one home, `store/<id>__<Tag>__<Tag>/`, holding the
document itself and whatever its owner keeps beside it — notes, figures,
supplements. That folder is the durable thing.

`tree/` is a view and nothing else: it can be deleted and rebuilt at any time.
Each document gets a real folder there holding a single symlink to its store
folder, so browsing the tree stays in the tree instead of jumping into the store
the moment a document is opened.

That does leave real directories in a part of the library that is disposable, so
anything found sitting in the tree is reported rather than quietly kept: see
`tree_litter`.

Re-tagging renames the store folder, which carries its contents along, and the
link is re-pointed. Links are relative, so the whole library can be moved.
"""

from __future__ import annotations

import os
import shutil
from dataclasses import dataclass, field, replace
from enum import Enum
from pathlib import Path
from typing import Sequence

from .db import Paper, PaperDb
from .discovery import file_id as hash_file
from .naming import disambiguate, link_name, store_name

STORE_DIR = "store"
NOTE_FILE = "notes.md"
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


@dataclass
class RescanReport:
    """What a rescan of the store found."""

    checked: int = 0
    rehashed: int = 0
    changed: list[tuple[str, str, str]] = field(default_factory=list)
    missing: list[Paper] = field(default_factory=list)


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

    def document_dir(self, paper: Paper) -> Path:
        """The document's folder in the store: its home, and its owner's."""
        return self.store_dir / paper.store_name

    def store_path(self, paper: Paper) -> Path:
        """The document file itself, inside its folder."""
        return self.document_dir(paper) / paper.document_name

    def plan_filing(self, paper: Paper, source: Path) -> PlannedFiling:
        """Where a paper would land, without touching the filesystem."""
        return PlannedFiling(
            file_id=paper.file_id,
            source=source,
            store_path=self.store_path(paper),
            link_path=self._link_path(paper, taken=set()),
        )

    def place_file(
        self, paper: Paper, source: Path, mode: FilingMode = FilingMode.MOVE
    ) -> Path:
        """Create the document's folder and put its file inside.

        Touches no database, so a pass can place every file of a batch before
        taking the write lock once for all of them.

        Raises:
            LibraryError: in preview mode, or if the folder already exists.
        """
        if mode is FilingMode.PREVIEW:
            raise LibraryError("place_file called in preview mode")

        directory = self.document_dir(paper)
        if directory.exists():
            raise LibraryError(f"store already holds {directory.name}")
        directory.mkdir(parents=True)

        target = directory / paper.document_name
        # `shutil` rather than `os.replace` because the source folder is often
        # on a different filesystem than the library, which `os.replace`
        # refuses.
        if mode is FilingMode.COPY:
            shutil.copy2(source, target)
        else:
            shutil.move(str(source), str(target))

        stat = target.stat()
        paper.size_bytes = stat.st_size
        paper.stored_mtime_ms = int(stat.st_mtime * 1000)
        return target

    def record(self, papers: Sequence[Paper]) -> None:
        """Write documents to the database, in one transaction."""
        self.db.upsert_many(papers)

    def link_paper(self, paper: Paper, taken: set[Path] | None = None) -> Path:
        """Link a document into the tree. Touches no database."""
        return self._link(paper, taken)

    def file_paper(
        self, paper: Paper, source: Path, mode: FilingMode = FilingMode.MOVE
    ) -> PlannedFiling:
        """Place, record, and link one document.

        The three steps in sequence, for a caller handling a single document.
        Ingest drives the same steps in phases across a whole batch so the
        database is only touched once the file work is done.

        The file lands before the database write, so a crash leaves a file in
        the store with no row rather than a row pointing at nothing. The first
        is repairable by re-ingesting; the second is a dangling reference.
        """
        target = self.place_file(paper, source, mode)
        self.record([paper])
        link = self.link_paper(paper)
        return PlannedFiling(
            file_id=paper.file_id, source=source, store_path=target, link_path=link
        )

    def retag(self, file_id: str, tags: list[str]) -> Paper:
        """Give a document new tags.

        Renames its folder in the store, so notes and supplements kept beside it
        travel with it, and re-points the link. Nothing is copied and nothing in
        the folder is touched.

        Raises:
            LibraryError: if the document is unknown or its folder is missing.
        """
        paper = self.db.get(file_id)
        if paper is None:
            raise LibraryError(f"no document with id {file_id}")

        old_dir = self.document_dir(paper)
        new_name = store_name(paper.file_id, tags, suffix="")
        new_dir = self.store_dir / new_name

        if not old_dir.is_dir():
            raise LibraryError(f"store is missing {paper.store_name}")
        if new_dir != old_dir and new_dir.exists():
            raise LibraryError(f"store already holds {new_name}")

        self._unlink(paper)
        if new_dir != old_dir:
            os.replace(old_dir, new_dir)
        self.db.set_tags(file_id, tags, new_name)

        updated = self.db.get(file_id)
        assert updated is not None  # just written
        self._link(updated)
        return updated

    def remove(self, file_id: str) -> Paper:
        """Take a document out of the library: link, folder, and rows.

        The link goes first and the rows last, so an interruption leaves the
        library holding less than it claims rather than claiming more than it
        holds. A stale row pointing at a deleted folder is the harder state to
        notice.

        Raises:
            LibraryError: if the document is unknown.
        """
        paper = self.db.get(file_id)
        if paper is None:
            raise LibraryError(f"no document with id {file_id}")

        self._unlink(paper)
        # The whole folder goes, including anything kept beside the document.
        # That is why removal asks first.
        shutil.rmtree(self.document_dir(paper), ignore_errors=True)
        self.db.delete(file_id)
        return paper

    def rescan(self) -> RescanReport:
        """Bring the recorded content hashes back in line with the store.

        A file edited in place — annotated, re-saved — keeps its name but no
        longer matches its recorded hash. Left alone that hash goes stale, and
        since it is the key that recognises a document the library already has,
        the edited copy arriving later would be ingested a second time.

        Runs in phases: read the rows, drop the lock, stat and hash, then write
        once. The file work is the slow part and holds no lock while it happens.
        Size and mtime are checked first, so an unchanged library reads nothing.
        """
        report = RescanReport()
        papers = self.db.all_papers()
        self.db.release()

        updates: list[tuple[str, str, int, int]] = []
        for paper in papers:
            path = self.store_path(paper)
            if not path.exists():
                report.missing.append(paper)
                continue

            report.checked += 1
            stat = path.stat()
            size, mtime_ms = stat.st_size, int(stat.st_mtime * 1000)
            if paper.size_bytes == size and paper.stored_mtime_ms == mtime_ms:
                continue

            # The stat moved, so the content might have. Only now is it worth
            # reading the file.
            report.rehashed += 1
            digest = hash_file(path)
            if digest != paper.content_hash:
                report.changed.append((paper.file_id, paper.content_hash, digest))
            # Recorded even when the hash is unchanged — a touched file should
            # not be re-read on every future scan.
            updates.append((paper.file_id, digest, size, mtime_ms))

        self.db.set_stored_file_states(updates)
        return report

    def note_path(self, paper: Paper) -> Path:
        """Where a document's notes live: in its folder, beside it."""
        return self.document_dir(paper) / NOTE_FILE

    def migrate_store_layout(self) -> list[str]:
        """Move documents from the old flat store into a folder each.

        Libraries written before documents had folders hold
        `store/<id>__<Tag>.pdf`. Each becomes `store/<id>__<Tag>/<name>.pdf`, so
        the document has somewhere to keep notes beside it.

        Idempotent and resumable: a document already in a folder is left alone,
        so an interrupted run is finished by running it again. Returns the ids
        it moved.
        """
        moved: list[str] = []
        for paper in self.db.all_papers():
            if paper.document_name and self.store_path(paper).is_file():
                continue

            flat = self.store_dir / paper.store_name
            if not flat.is_file():
                continue  # already a folder, or genuinely missing

            folder_name = Path(paper.store_name).stem
            suffix = Path(paper.store_name).suffix or ".pdf"
            document_name = link_name(
                fallback=f"{folder_name}{suffix}",
                authors=paper.authors,
                year=paper.year,
                title=paper.title,
                suffix=suffix,
            )
            folder = self.store_dir / folder_name
            folder.mkdir(parents=True, exist_ok=True)
            os.replace(flat, folder / document_name)
            self.db.set_store_layout(paper.file_id, folder_name, document_name)
            moved.append(paper.file_id)
        return moved

    def existing_categories(self, limit: int | None = None) -> list[str]:
        """Category paths already in use, for steering a new document."""
        return self.db.tag_paths(limit=limit)

    def rebuild_tree(self) -> int:
        """Rebuild every symlink from the database, discarding the old tree.

        Only the links are discarded. Anything else found in the tree — a note
        filed beside a document, a folder someone made — is left alone, because
        a rebuild converging the links is not a reason to delete work.
        """
        _clear_links(self.tree_dir)

        taken: set[Path] = set()
        linked = 0
        for paper in self.db.all_papers():
            if not self.store_path(paper).exists():
                continue
            self._link(paper, taken=taken)
            linked += 1
        return linked

    def tree_litter(self) -> list[Path]:
        """Real files sitting in the tree, which is not where work is safe.

        The tree is rebuildable and is not part of a backup of the store and the
        database, so a file kept here is one deleted tree away from being gone.
        Reported rather than removed: it is not this tool's to delete.

        Dotfiles are skipped — `.DS_Store` and its kind are the filesystem's
        litter, not the owner's work.
        """
        found: list[Path] = []
        if not self.tree_dir.exists():
            return found
        for parent, dirnames, filenames in os.walk(self.tree_dir, followlinks=False):
            here = Path(parent)
            dirnames[:] = [name for name in dirnames if not (here / name).is_symlink()]
            for name in filenames:
                entry = here / name
                if not entry.is_symlink() and not name.startswith("."):
                    found.append(entry)
        return sorted(found)

    def missing_files(self) -> list[Paper]:
        """Papers the database knows about whose file is gone from the store."""
        return [
            paper
            for paper in self.db.all_papers()
            if not self.store_path(paper).exists()
        ]

    # ---- links -------------------------------------------------------------

    def _document_dir_in_tree(self, paper: Paper, taken: set[Path]) -> Path:
        """The document's folder in the tree — a real folder, not a link.

        Browsing stops here rather than being thrown into the store, so the
        category a document was reached through stays visible.
        """
        parent = self.tree_dir.joinpath(*paper.tags) if paper.tags else self.tree_dir
        name = link_name(
            fallback=paper.store_name,
            authors=paper.authors,
            year=paper.year,
            title=paper.title,
            suffix="",
        )
        candidate = parent / name
        if candidate in taken:
            candidate = parent / disambiguate(name, paper.file_id)
        return candidate

    def _link_path(self, paper: Paper, taken: set[Path]) -> Path:
        """The one link inside that folder, pointing at the store folder."""
        directory = self._document_dir_in_tree(paper, taken)
        return directory / directory.name

    def _link(self, paper: Paper, taken: set[Path] | None = None) -> Path:
        taken = taken if taken is not None else set()
        directory = self._document_dir_in_tree(paper, taken)
        directory.mkdir(parents=True, exist_ok=True)
        link = directory / directory.name
        if link.is_symlink() or link.exists():
            link.unlink()
        link.symlink_to(
            os.path.relpath(self.document_dir(paper), link.parent),
            target_is_directory=True,
        )
        taken.add(directory)
        return link

    def _unlink(self, paper: Paper) -> None:
        """Remove a document's link, and any branches it leaves empty."""
        link = self._link_path(paper, taken=set())
        if link.is_symlink() or link.exists():
            link.unlink()
        _prune_empty(link.parent, stop=self.tree_dir)


def _clear_links(root: Path) -> None:
    """Remove every symlink under `root`, and the folders left empty.

    Never descends into a link: they point at store folders, and walking into
    one would mean walking the library's real contents.
    """
    if not root.exists():
        return
    directories: list[Path] = []
    for parent, dirnames, filenames in os.walk(root, followlinks=False):
        here = Path(parent)
        directories.append(here)
        for name in list(dirnames):
            entry = here / name
            if entry.is_symlink():
                entry.unlink()
                dirnames.remove(name)
        for name in filenames:
            entry = here / name
            if entry.is_symlink():
                entry.unlink()

    for directory in sorted(directories, key=lambda item: len(item.parts), reverse=True):
        if directory == root:
            continue
        try:
            directory.rmdir()
        except OSError:
            pass  # holds something that is not ours


def _prune_empty(directory: Path, stop: Path) -> None:
    current = directory
    while current != stop and stop in current.parents:
        try:
            current.rmdir()
        except OSError:
            return
        current = current.parent
