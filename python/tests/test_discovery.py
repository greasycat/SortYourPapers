from __future__ import annotations

from pathlib import Path

from conftest import write_pdf

from sortyourpaperya.discovery import discover_pdfs, file_id, snapshot_input


def test_non_recursive_scan_skips_nested_pdfs(tmp_path: Path) -> None:
    write_pdf(tmp_path / "a.pdf", "root")
    write_pdf(tmp_path / "nested" / "b.pdf", "nested")

    assert len(discover_pdfs(tmp_path, recursive=False)) == 1
    assert len(discover_pdfs(tmp_path, recursive=True)) == 2


def test_snapshot_excludes_the_library_so_it_is_never_reingested(tmp_path: Path) -> None:
    output = tmp_path / "sorted"
    write_pdf(tmp_path / "pending.pdf", "pending")
    write_pdf(output / "AI" / "filed.pdf", "already filed")

    pending = snapshot_input(tmp_path, output, recursive=True)

    assert [path.name for path in pending] == ["pending.pdf"]


def test_snapshot_excludes_oversized_files(tmp_path: Path) -> None:
    small = write_pdf(tmp_path / "small.pdf", "small")
    big = tmp_path / "big.pdf"
    write_pdf(big, "big")
    big.write_bytes(big.read_bytes() + b"\x00" * (2 * 1024 * 1024))

    pending = snapshot_input(tmp_path, tmp_path / "sorted", max_file_size_mb=1)

    assert list(pending) == [small]


def test_snapshot_tracks_size_so_a_growing_file_looks_unsettled(tmp_path: Path) -> None:
    path = write_pdf(tmp_path / "a.pdf", "a")
    output = tmp_path / "sorted"

    before = snapshot_input(tmp_path, output)
    path.write_bytes(path.read_bytes() + b"\x00" * 1024)
    after = snapshot_input(tmp_path, output)

    assert before != after


def test_file_id_is_content_addressed_not_path_addressed(tmp_path: Path) -> None:
    original = write_pdf(tmp_path / "a.pdf", "same text")
    renamed = tmp_path / "b.pdf"
    renamed.write_bytes(original.read_bytes())
    different = write_pdf(tmp_path / "c.pdf", "other text")

    assert file_id(original) == file_id(renamed)
    assert file_id(original) != file_id(different)


def test_a_file_vanishing_mid_scan_does_not_raise(tmp_path: Path) -> None:
    """A watched folder changes while it is read; that must not kill the scan.

    `discover_pdfs` runs on every pass of a long-lived watcher, so an exception
    here takes the whole service down over an ordinary download or rename.
    """
    from unittest import mock

    write_pdf(tmp_path / "a.pdf", "one")
    write_pdf(tmp_path / "b.pdf", "two")
    real_stat = Path.stat

    def vanish(self, *args, **kwargs):
        if self.name == "a.pdf":
            raise FileNotFoundError(self)
        return real_stat(self, *args, **kwargs)

    with mock.patch.object(Path, "stat", vanish):
        found = discover_pdfs(tmp_path)

    assert [p.path.name for p in found] == ["b.pdf"], "the survivor is still listed"
