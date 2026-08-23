from __future__ import annotations

import json
import os
import signal
import subprocess
import sys
from pathlib import Path

import pytest

from sypy.watchlock import WatchConflict, claim, locks_dir

# A real process holding a real claim. Death has to be real too: ownership is an
# `flock`, and the kernel is the only thing that can drop one — no amount of
# editing the file's contents makes a live watcher look dead, which is the whole
# point of not deciding it from a pid.
_HOLDER = """
import sys, time
from pathlib import Path
from sypy.watchlock import claim
claim(Path(sys.argv[1]), Path(sys.argv[2]))
print("held", flush=True)
time.sleep(300)
"""


def holding(inbox: Path, library: Path) -> subprocess.Popen:
    """Start a watcher process and return it once its claims are in place."""
    process = subprocess.Popen(
        [sys.executable, "-c", _HOLDER, str(inbox), str(library)],
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
    )
    line = process.stdout.readline()
    if line.strip() != "held":
        process.kill()
        raise AssertionError(f"holder did not start: {line}{process.stdout.read()}")
    return process


def kill(process: subprocess.Popen) -> None:
    """Kill it outright, the way a crash does: no release runs."""
    process.send_signal(signal.SIGKILL)
    process.wait(timeout=10)


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
    holder = holding(inbox, library)
    with pytest.raises(WatchConflict):
        claim(inbox, library)  # proof the claims were real while it lived
    kill(holder)

    taken = claim(inbox, library)

    assert taken.held
    taken.release()


def test_a_claim_naming_a_pid_since_reused_is_taken_over(folders) -> None:
    """Whether a claim is live is the lock, not the number written in it.

    Pids are handed out again. A watcher that crashed leaves a claim naming a
    number the system later gives to an unrelated process, and reading liveness
    off that number locks the folder out for as long as that stranger lives —
    which, if it is a login shell, is until the machine is rebooted.
    """
    inbox, library = folders
    holder = holding(inbox, library)
    kill(holder)

    # Stand in for the recycled pid: this test process is certainly running, and
    # it is not a watcher.
    for path in locks_dir().glob("*.json"):
        payload = json.loads(path.read_text())
        payload["pid"] = os.getpid()
        path.write_text(json.dumps(payload))

    taken = claim(inbox, library)

    assert taken.held, "a live pid in an unlocked claim is not an owner"
    taken.release()


def test_releasing_frees_the_folders(folders) -> None:
    inbox, library = folders
    claim(inbox, library).release()

    again = claim(inbox, library)

    assert again.held
    again.release()


def test_release_leaves_a_claim_another_watcher_has_taken_over(folders) -> None:
    # A claim is a file plus the lock on it. If the file at that name is no
    # longer the one we locked, it is somebody else's claim, and releasing ours
    # by deleting whatever is sitting there would hand their folder away.
    inbox, library = folders
    stale = claim(inbox, library)
    for path in stale.held:
        path.unlink()
        path.write_text(json.dumps({"pid": 999_999, "path": str(inbox)}))

    stale.release()

    survivors = sorted(locks_dir().glob("*.json"))
    assert len(survivors) == 2, "the other watcher's claims were deleted"
    assert all(
        json.loads(path.read_text())["pid"] == 999_999 for path in survivors
    )


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
    from sypy.cli import _claimed_folders

    inbox, library = folders
    holder = holding(inbox, library)
    running = _claimed_folders(locks_dir())
    assert set(running) == {inbox.resolve(), library.resolve()}
    assert set(running.values()) == {holder.pid}, "it should name who is running"

    kill(holder)

    assert _claimed_folders(locks_dir()) == {}


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


def test_a_claim_file_nobody_holds_is_taken_over(folders) -> None:
    # A claim file left behind by a process that is gone — including one so
    # broken nothing can be read out of it — is not worth respecting.
    inbox, library = folders
    directory = locks_dir()
    directory.mkdir(parents=True, exist_ok=True)
    claim(inbox, library).release()
    for name in ("input", "library"):
        for path in directory.glob(f"{name}-*.json"):
            path.write_text("", encoding="utf-8")

    taken = claim(inbox, library)

    assert taken.held
    taken.release()


def test_an_empty_claim_someone_holds_is_still_refused(folders) -> None:
    """The file's contents are not what says a folder is taken.

    A claim being unreadable used to be enough to take it over, which meant a
    watcher whose claim was truncated — by a full disk, by a crash mid-write —
    could be joined by a second one while it was still running.
    """
    inbox, library = folders
    holder = holding(inbox, library)
    try:
        for path in locks_dir().glob("*.json"):
            path.write_text("", encoding="utf-8")

        with pytest.raises(WatchConflict):
            claim(inbox, library)
    finally:
        kill(holder)
