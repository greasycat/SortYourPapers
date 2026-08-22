from __future__ import annotations

from dataclasses import replace
from pathlib import Path

import pytest
from conftest import write_pdf

from syp_prototype.db import Paper
from syp_prototype.library import Library, LibraryError
from syp_prototype.naming import link_name, new_paper_id, store_name


def _paper(tags: list[str], **overrides) -> Paper:
    paper_id = overrides.pop("file_id", new_paper_id())
    fields = {
        "file_id": paper_id,
        "content_hash": "hash-" + paper_id,
        # The folder carries id and tags; the file inside carries the readable
        # name, exactly as ingest builds them.
        "store_name": store_name(paper_id, tags, suffix=""),
        "title": "Attention Is All You Need",
        "authors": ["Ashish Vaswani"],
        "year": 2017,
        "tags": tags,
    }
    fields.update(overrides)
    fields.setdefault(
        "document_name",
        link_name(
            fallback=f"{fields['store_name']}.pdf",
            authors=fields.get("authors") or [],
            year=fields.get("year"),
            title=fields.get("title"),
        ),
    )
    return Paper(**fields)


def test_filing_gives_the_document_its_own_folder_in_the_store(
    library: Library, tmp_path: Path
) -> None:
    source = write_pdf(tmp_path / "src" / "raw.pdf", "text")
    paper = _paper(["AI", "Transformers"])

    filing = library.file_paper(paper, source)

    assert not source.exists(), "the source should have been moved, not copied"
    # The folder is the document's home; the file sits inside it.
    assert filing.store_path.parent == library.store_dir / paper.store_name
    assert filing.store_path.name == paper.document_name
    assert filing.store_path.is_file()
    assert paper.store_name.startswith(paper.file_id)
    assert paper.store_name.endswith("__AI__Transformers")


def test_the_tree_is_symlinks_pointing_back_into_the_store(
    library: Library, tmp_path: Path
) -> None:
    source = write_pdf(tmp_path / "src" / "raw.pdf", "text")
    paper = _paper(["AI", "Transformers"])

    filing = library.file_paper(paper, source)

    link = filing.link_path
    # One link per document, pointing at its folder — so opening it shows the
    # document together with anything kept beside it.
    assert link.is_symlink()
    assert link.parent == library.tree_dir / "AI" / "Transformers"
    assert link.name == "vaswani_2017_attention-is-all-you-need"
    assert link.resolve() == library.document_dir(paper).resolve()
    assert (link / paper.document_name).is_file()


def test_links_are_relative_so_the_library_can_be_moved(
    library: Library, tmp_path: Path
) -> None:
    source = write_pdf(tmp_path / "src" / "raw.pdf", "text")
    filing = library.file_paper(_paper(["AI"]), source)
    library.close()

    moved = tmp_path / "moved-library"
    library.root.rename(moved)
    relocated = moved / filing.link_path.relative_to(library.root)

    assert relocated.is_symlink()
    assert relocated.resolve().is_dir(), "relative link should survive the move"
    assert any(relocated.iterdir()), "and still lead to the document"


def test_a_paper_with_no_usable_metadata_links_under_its_store_name(
    library: Library, tmp_path: Path
) -> None:
    source = write_pdf(tmp_path / "src" / "raw.pdf", "text")
    paper = _paper(["AI"], title=None, authors=[], year=None)

    filing = library.file_paper(paper, source)

    assert filing.link_path.name == paper.store_name


def test_retagging_renames_the_file_and_moves_the_link(
    library: Library, tmp_path: Path
) -> None:
    source = write_pdf(tmp_path / "src" / "raw.pdf", "text")
    filing = library.file_paper(_paper(["AI", "Transformers"]), source)
    old_store_path = filing.store_path.parent

    updated = library.retag(filing.file_id, ["Systems", "Databases"])

    assert not old_store_path.exists()
    assert (library.store_dir / updated.store_name).is_dir()
    assert (library.store_dir / updated.store_name / updated.document_name).is_file()
    assert updated.store_name.endswith("__Systems__Databases")
    assert updated.store_name.startswith(filing.file_id), "the id must not change"
    new_link = library.tree_dir / "Systems" / "Databases" / filing.link_path.name
    assert new_link.is_symlink()
    assert not (library.tree_dir / "AI").exists(), "the emptied branch should be pruned"


def test_retagging_an_unknown_paper_is_refused(library: Library) -> None:
    with pytest.raises(LibraryError, match="no document"):
        library.retag("ffffffffffff", ["AI"])


def test_filing_over_an_existing_store_entry_is_refused(
    library: Library, tmp_path: Path
) -> None:
    paper = _paper(["AI"])
    library.file_paper(paper, write_pdf(tmp_path / "a" / "raw.pdf", "one"))
    second = write_pdf(tmp_path / "b" / "raw.pdf", "two")

    with pytest.raises(LibraryError, match="already holds"):
        library.file_paper(paper, second)

    assert second.exists(), "a refused filing must not consume the source"


def test_rebuilding_the_tree_restores_it_after_deletion(
    library: Library, tmp_path: Path
) -> None:
    for index in range(3):
        library.file_paper(
            _paper(["AI", f"Sub{index}"], title=f"Paper {index}"),
            write_pdf(tmp_path / f"src{index}" / "raw.pdf", f"text {index}"),
        )

    import shutil

    shutil.rmtree(library.tree_dir)
    linked = library.rebuild_tree()

    assert linked == 3
    links = [p for p in library.tree_dir.rglob("*") if p.is_symlink()]
    assert len(links) == 3
    assert all(link.resolve().is_dir() for link in links)


def test_rebuilding_disambiguates_two_papers_that_want_the_same_link_name(
    library: Library, tmp_path: Path
) -> None:
    # Same author, year, and title, so both want the identical link name.
    for index in range(2):
        library.file_paper(
            _paper(["AI"]), write_pdf(tmp_path / f"src{index}" / "raw.pdf", f"t{index}")
        )

    library.rebuild_tree()

    links = sorted(p.name for p in (library.tree_dir / "AI").iterdir())
    assert len(links) == 2, f"one link overwrote the other: {links}"
    assert len({link for link in links}) == 2


def test_missing_files_are_reported_not_linked(
    library: Library, tmp_path: Path
) -> None:
    filing = library.file_paper(
        _paper(["AI"]), write_pdf(tmp_path / "src" / "raw.pdf", "text")
    )
    filing.store_path.unlink()

    linked = library.rebuild_tree()

    assert linked == 0
    assert [paper.file_id for paper in library.missing_files()] == [filing.file_id]


def test_removing_takes_the_link_the_file_and_the_record(
    library: Library, tmp_path: Path
) -> None:
    filing = library.file_paper(
        _paper(["AI", "Transformers"]), write_pdf(tmp_path / "src" / "raw.pdf", "text")
    )

    removed = library.remove(filing.file_id)

    assert removed.file_id == filing.file_id
    assert not filing.store_path.exists(), "the stored file should be gone"
    assert not filing.link_path.exists(), "the link should be gone"
    assert library.db.get(filing.file_id) is None
    assert not (library.tree_dir / "AI").exists(), "the emptied branch is pruned"


def test_removing_leaves_the_other_documents_alone(
    library: Library, tmp_path: Path
) -> None:
    keep = library.file_paper(
        _paper(["AI"], title="Kept"), write_pdf(tmp_path / "a" / "raw.pdf", "one")
    )
    drop = library.file_paper(
        _paper(["Systems"], title="Dropped"), write_pdf(tmp_path / "b" / "raw.pdf", "two")
    )

    library.remove(drop.file_id)

    assert keep.store_path.is_file()
    assert keep.link_path.is_symlink()
    assert [p.file_id for p in library.db.all_papers()] == [keep.file_id]


def test_removing_an_unknown_document_is_refused(library: Library) -> None:
    with pytest.raises(LibraryError, match="no document"):
        library.remove("ffffffffffff")


def test_existing_categories_are_the_paths_already_in_use(
    library: Library, tmp_path: Path
) -> None:
    library.file_paper(
        _paper(["Medicine", "Radiology"]), write_pdf(tmp_path / "a" / "r.pdf", "one")
    )
    library.file_paper(
        _paper(["Medicine", "Radiology"]), write_pdf(tmp_path / "b" / "r.pdf", "two")
    )
    library.file_paper(
        _paper(["Finance", "Bills"]), write_pdf(tmp_path / "c" / "r.pdf", "three")
    )

    assert library.existing_categories() == ["Finance/Bills", "Medicine/Radiology"]


def test_editing_a_stored_file_in_place_refreshes_its_hash(
    library: Library, tmp_path: Path
) -> None:
    filing = library.file_paper(
        _paper(["AI"]), write_pdf(tmp_path / "src" / "raw.pdf", "text")
    )
    original_hash = library.db.get(filing.file_id).content_hash

    # Same name, different bytes — an annotation, or a re-save.
    filing.store_path.write_bytes(filing.store_path.read_bytes() + b"\n% annotated\n")
    report = library.rescan()

    refreshed = library.db.get(filing.file_id)
    assert refreshed.content_hash != original_hash
    assert [(f, o, n) for f, o, n in report.changed] == [
        (filing.file_id, original_hash, refreshed.content_hash)
    ]


def test_a_refreshed_hash_stops_the_edited_copy_being_ingested_twice(
    library: Library, tmp_path: Path
) -> None:
    # The stale hash matters because it is the key that recognises a document
    # the library already holds.
    filing = library.file_paper(
        _paper(["AI"]), write_pdf(tmp_path / "src" / "raw.pdf", "text")
    )
    filing.store_path.write_bytes(filing.store_path.read_bytes() + b"\n% annotated\n")
    edited_hash = _sha_of(filing.store_path)

    assert library.db.find_by_content_hash(edited_hash) is None, "stale before rescan"
    library.rescan()
    assert library.db.find_by_content_hash(edited_hash) == filing.file_id


def test_an_unchanged_library_is_not_reread(library: Library, tmp_path: Path) -> None:
    # The whole point of recording size and mtime: a quiet library costs one
    # stat per document, not a full read of every file.
    for index in range(3):
        library.file_paper(
            _paper(["AI"], title=f"Paper {index}"),
            write_pdf(tmp_path / f"src{index}" / "raw.pdf", f"text {index}"),
        )

    report = library.rescan()

    assert report.checked == 3
    assert report.rehashed == 0, "nothing changed, so nothing should be reread"
    assert report.changed == []


def test_a_touched_but_unchanged_file_is_reread_once_then_settles(
    library: Library, tmp_path: Path
) -> None:
    import os

    filing = library.file_paper(
        _paper(["AI"]), write_pdf(tmp_path / "src" / "raw.pdf", "text")
    )
    # The fixture fabricates a hash rather than reading the file, and the stat
    # gate means a rescan would never notice. Record the real one, so a moved
    # mtime is the only thing this test changes.
    stat = filing.store_path.stat()
    library.db.set_stored_file_state(
        filing.file_id,
        _sha_of(filing.store_path),
        stat.st_size,
        int(stat.st_mtime * 1000),
    )
    os.utime(filing.store_path, (stat.st_atime, stat.st_mtime + 5))

    first = library.rescan()
    second = library.rescan()

    assert first.rehashed == 1, "a moved mtime must be checked"
    assert first.changed == [], "the bytes did not change"
    assert second.rehashed == 0, "the new stat should have been recorded"


def test_rescan_reports_a_file_that_vanished_rather_than_failing(
    library: Library, tmp_path: Path
) -> None:
    filing = library.file_paper(
        _paper(["AI"]), write_pdf(tmp_path / "src" / "raw.pdf", "text")
    )
    filing.store_path.unlink()

    report = library.rescan()

    assert [paper.file_id for paper in report.missing] == [filing.file_id]
    assert report.checked == 0


def _sha_of(path: Path) -> str:
    import hashlib

    return hashlib.sha256(path.read_bytes()).hexdigest()[:16]


def test_a_note_filed_beside_a_document_survives_a_rebuild(
    library: Library, tmp_path: Path
) -> None:
    """The reason each document gets a folder: things can be kept in it.

    A rebuild converges the links. It is not a licence to delete work someone
    filed alongside a document.
    """
    filing = library.file_paper(
        _paper(["AI", "Transformers"]), write_pdf(tmp_path / "src" / "raw.pdf", "text")
    )
    note = filing.link_path.parent / "my-notes.md"
    note.write_text("what I thought of this paper")

    library.rebuild_tree()

    assert note.is_file(), "a rebuild must not delete files it did not create"
    assert note.read_text() == "what I thought of this paper"
    assert filing.link_path.is_symlink(), "the link is still rebuilt"


def test_a_note_follows_its_document_when_it_is_retagged(
    library: Library, tmp_path: Path
) -> None:
    filing = library.file_paper(
        _paper(["AI", "Transformers"]), write_pdf(tmp_path / "src" / "raw.pdf", "text")
    )
    (filing.link_path / "my-notes.md").write_text("notes")

    library.retag(filing.file_id, ["Systems", "Databases"])

    moved = (
        library.tree_dir / "Systems" / "Databases" / filing.link_path.name / "my-notes.md"
    )
    assert moved.is_file(), "the note lives in the store folder and moves with it"
    assert moved.read_text() == "notes"
    assert not (library.tree_dir / "AI").exists(), "the emptied branch is pruned"


def test_the_link_still_resolves_after_a_retag_changes_depth(
    library: Library, tmp_path: Path
) -> None:
    # Links are relative, so a folder that moves to a different depth needs its
    # link re-pointed rather than carried along as it was.
    filing = library.file_paper(
        _paper(["AI", "Vision", "Detection"]),
        write_pdf(tmp_path / "src" / "raw.pdf", "text"),
    )

    updated = library.retag(filing.file_id, ["Systems"])

    link = library.tree_dir / "Systems" / filing.link_path.name
    assert link.is_symlink()
    assert link.resolve() == (library.store_dir / updated.store_name).resolve()
    assert (link / updated.document_name).is_file(), "and still reaches the document"


def test_removing_leaves_a_folder_that_still_holds_something(
    library: Library, tmp_path: Path
) -> None:
    filing = library.file_paper(
        _paper(["AI"]), write_pdf(tmp_path / "src" / "raw.pdf", "text")
    )
    note = filing.link_path.parent / "my-notes.md"
    note.write_text("notes")

    library.remove(filing.file_id)

    assert not filing.link_path.exists(), "the link goes"
    assert not filing.store_path.exists(), "the stored file goes"
    assert note.is_file(), "someone else's file is not this command's to delete"


def test_removing_takes_the_folder_when_it_is_empty(
    library: Library, tmp_path: Path
) -> None:
    filing = library.file_paper(
        _paper(["AI"]), write_pdf(tmp_path / "src" / "raw.pdf", "text")
    )

    library.remove(filing.file_id)

    assert not filing.link_path.parent.exists()
    assert not (library.tree_dir / "AI").exists()


def test_migrating_a_flat_store_gives_each_document_a_folder(
    library: Library, tmp_path: Path
) -> None:
    """A library written before documents had folders is moved into them."""
    paper = _paper(["AI", "Transformers"])
    # Recreate the old layout: the document loose in the store, no inner name.
    flat_name = f"{paper.store_name}.pdf"
    library.store_dir.mkdir(parents=True, exist_ok=True)
    write_pdf(library.store_dir / flat_name, "text")
    library.db.upsert(replace(paper, store_name=flat_name, document_name=""))

    moved = library.migrate_store_layout()

    assert moved == [paper.file_id]
    migrated = library.db.get(paper.file_id)
    assert migrated.store_name == paper.store_name, "the folder keeps id and tags"
    assert migrated.document_name == "vaswani_2017_attention-is-all-you-need.pdf"
    assert library.store_path(migrated).is_file()
    assert not (library.store_dir / flat_name).exists(), "the flat file is gone"


def test_migrating_twice_changes_nothing_the_second_time(
    library: Library, tmp_path: Path
) -> None:
    # Idempotent, so an interrupted migration is finished by running it again.
    paper = _paper(["AI"])
    flat_name = f"{paper.store_name}.pdf"
    library.store_dir.mkdir(parents=True, exist_ok=True)
    write_pdf(library.store_dir / flat_name, "text")
    library.db.upsert(replace(paper, store_name=flat_name, document_name=""))

    first = library.migrate_store_layout()
    second = library.migrate_store_layout()

    assert first == [paper.file_id]
    assert second == [], "a document already in a folder is left alone"


def test_migration_leaves_an_already_foldered_document_untouched(
    library: Library, tmp_path: Path
) -> None:
    filing = library.file_paper(
        _paper(["AI"]), write_pdf(tmp_path / "src" / "raw.pdf", "text")
    )
    note = filing.store_path.parent / "notes.md"
    note.write_text("keep me")

    assert library.migrate_store_layout() == []
    assert note.read_text() == "keep me"


def test_a_note_lives_in_the_store_so_it_outlives_the_tree(
    library: Library, tmp_path: Path
) -> None:
    """The reason the folder moved into the store.

    A note kept only in the tree is lost when the tree is deleted, and the tree
    is disposable by design.
    """
    import shutil as _shutil

    filing = library.file_paper(
        _paper(["AI"]), write_pdf(tmp_path / "src" / "raw.pdf", "text")
    )
    paper = library.db.get(filing.file_id)
    library.note_path(paper).write_text("why I saved this")

    _shutil.rmtree(library.tree_dir)
    library.rebuild_tree()

    assert library.note_path(paper).read_text() == "why I saved this"
    assert (filing.link_path / "notes.md").is_file(), "and is reachable through the tree"
