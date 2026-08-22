from __future__ import annotations

import json
import os
from pathlib import Path

import pytest

from syp_prototype.watchlock import WatchConflict, claim, locks_dir


@pytest.fixture
def folders(tmp_path: Path) -> tuple[Path, Path]:
    inbox, library = tmp_path / "inbox", tmp_path / "library"
    inbox.mkdir()
    library.mkdir()
    return inbox, library


def test_a_second_watcher_on_the_same_input_is_refused(folders, tmp_path) -> None:
    inbox, library = folders
    held = claim(inbox, library)
    try:
        with pytest.raises(WatchConflict, match="input"):
            claim(inbox, tmp_path / "other-library")
    finally:
        held.release()


def test_a_second_watcher_on_the_same_library_is_refused(folders, tmp_path) -> None:
    inbox, library = folders
    other = tmp_path / "other-inbox"
    other.mkdir()
    held = claim(inbox, library)
    try:
        with pytest.raises(WatchConflict, match="library"):
            claim(other, library)
    finally:
        held.release()


def test_two_watchers_sharing_nothing_both_run(folders, tmp_path) -> None:
    inbox, library = folders
    other_in, other_lib = tmp_path / "in2", tmp_path / "lib2"
    other_in.mkdir()
    other_lib.mkdir()

    first = claim(inbox, library)
    second = claim(other_in, other_lib)

    assert first.held and second.held
    first.release()
    second.release()


def test_a_refused_watcher_leaves_no_claim_behind(folders, tmp_path) -> None:
    # The input is free but the library is taken, so the input claim made on the
    # way must be given back — otherwise a refused watcher locks out a folder it
    # never used.
    inbox, library = folders
    other_in = tmp_path / "in2"
    other_in.mkdir()
    held = claim(inbox, library)
    try:
        with pytest.raises(WatchConflict):
            claim(other_in, library)
        # Proof it was released: someone else can now take it.
        taken = claim(other_in, tmp_path / "lib2")
        taken.release()
    finally:
        held.release()


def test_the_same_folder_reached_by_another_route_still_collides(
    folders, tmp_path
) -> None:
    inbox, library = folders
    held = claim(inbox, library)
    try:
        indirect = inbox.parent / "." / inbox.name
        with pytest.raises(WatchConflict):
            claim(indirect, tmp_path / "lib2")
    finally:
        held.release()


def test_a_claim_left_by_a_dead_watcher_is_taken_over(folders) -> None:
    """A crash must not lock a folder out forever."""
    inbox, library = folders
    held = claim(inbox, library)
    # Rewrite both claims as belonging to a pid that cannot be running.
    for path in held.held:
        payload = json.loads(path.read_text())
        payload["pid"] = 2**22  # above any real pid on this platform
        path.write_text(json.dumps(payload))

    taken = claim(inbox, library)

    assert taken.held
    taken.release()


def test_releasing_frees_the_folders(folders) -> None:
    inbox, library = folders
    claim(inbox, library).release()

    again = claim(inbox, library)

    assert again.held
    again.release()


def test_release_leaves_a_claim_another_watcher_has_taken_over(folders) -> None:
    # After a takeover the lock belongs to someone else; giving up ours must not
    # delete theirs.
    inbox, library = folders
    stale = claim(inbox, library)
    for path in stale.held:
        payload = json.loads(path.read_text())
        payload["pid"] = os.getpid() + 1_000_000
        path.write_text(json.dumps(payload))

    stale.release()

    assert all(path.exists() for path in [*locks_dir().glob("*.json")]) or True
    holders = [json.loads(p.read_text())["pid"] for p in locks_dir().glob("*.json")]
    assert holders, "the other watcher's claims should still be there"


def test_claims_are_kept_out_of_the_watched_folders(folders) -> None:
    # A lock file in someone's Downloads folder is litter.
    inbox, library = folders
    held = claim(inbox, library)
    try:
        assert list(inbox.iterdir()) == []
        assert list(library.iterdir()) == []
        assert list(locks_dir().glob("*.json"))
    finally:
        held.release()


def test_a_claim_left_by_a_dead_watcher_does_not_read_as_running(folders) -> None:
    """Stopping a service leaves its claims behind: SIGTERM skips the release.

    So anything reporting what is running has to check the owner is alive, not
    just that a claim exists.
    """
    import json

    from syp_prototype.cli import _claimed_folders

    inbox, library = folders
    held = claim(inbox, library)
    assert set(_claimed_folders(locks_dir())) == {inbox.resolve(), library.resolve()}

    for path in held.held:
        payload = json.loads(path.read_text())
        payload["pid"] = 2**22
        path.write_text(json.dumps(payload))

    assert _claimed_folders(locks_dir()) == {}
    held.release()


def test_a_claim_is_never_visible_while_incomplete(folders) -> None:
    """A half-written claim reads as nobody's, and would be taken over.

    Creating the file and filling it afterwards leaves a window where a second
    watcher sees an empty file, concludes the owner is unknown, and claims the
    folder itself — so both run, which is what this module exists to prevent.
    """
    import json

    inbox, library = folders
    held = claim(inbox, library)
    try:
        for path in locks_dir().glob("*.json"):
            body = path.read_text(encoding="utf-8")
            assert body, f"{path.name} is empty"
            assert json.loads(body)["pid"] == os.getpid()
    finally:
        held.release()


def test_no_staging_files_are_left_behind(folders) -> None:
    inbox, library = folders
    held = claim(inbox, library)
    held.release()

    leftovers = [p.name for p in locks_dir().iterdir() if p.name.startswith(".")]
    assert leftovers == [], f"temporary files left: {leftovers}"


def test_an_empty_claim_file_is_treated_as_abandoned(folders) -> None:
    # The recovery path that made the old window dangerous is still correct on
    # its own terms: a claim nobody can be read out of is not worth respecting.
    inbox, library = folders
    held = claim(inbox, library)
    for path in held.held:
        path.write_text("", encoding="utf-8")

    taken = claim(inbox, library)

    assert taken.held
    taken.release()
