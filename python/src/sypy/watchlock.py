"""Claims that stop two watchers sharing an input folder or a library.

Two watchers on one library file the same document twice: each checks what the
library already holds before either writes, so both decide it is new. Two
watchers on one input folder do the same thing from the other direction. Neither
is caught by the database lock, which is held only in bursts and not across the
gap where the decision is made.

So a watcher claims both folders before it starts and holds them until it stops.
Claims live outside the folders they cover — a lock file in someone's Downloads
folder would be litter — and are keyed by the resolved path, so two watchers
reaching the same folder by different routes still collide.

A claim becomes visible only once it is complete: it is written to a temporary
file and then hard-linked into place, and `os.link` fails if the name is taken.
Creating it empty and filling it afterwards — which `O_EXCL` alone does — leaves
a window where a second watcher reads an empty file, concludes the owner is
unknown, and takes the claim over. Both would then be running, which is the one
thing this module exists to prevent.

**Who owns a claim is decided by an `flock` on the claim file, not by the pid
written inside it.** A pid is not an identity: it is reused, and a claim left by
a watcher that crashed points at whatever process was handed that number next —
so a `kill(pid, 0)` check reads a stranger's shell as the owner and locks the
folder out for as long as that process lives. The kernel drops an `flock` when
the process holding it dies, however it dies, so an abandoned claim is always
recognisable and a live one is never mistaken for abandoned. The pid is still
recorded, because "already watched by pid 63643" is what makes the refusal
actionable, but nothing decides anything from it.
"""

from __future__ import annotations

import errno
import fcntl
import hashlib
import json
import os
import time
from dataclasses import dataclass, field
from pathlib import Path

from .config import state_dir

# How many times to retry an acquisition that raced someone else's release.
# Each pass either wins the claim or proves someone holds it; the retries only
# cover the window where the file is unlinked out from under us.
_ACQUIRE_ATTEMPTS = 5


class WatchConflict(RuntimeError):
    """Raised when a folder is already claimed by a running watcher."""


def locks_dir() -> Path:
    """Where claims are kept: outside every library, so they are machine-wide."""
    return state_dir() / "watch-locks"


@dataclass
class _Held:
    path: Path
    fd: int


@dataclass
class WatchClaim:
    """The claims one watcher holds. Release them when it stops."""

    entries: list[_Held] = field(default_factory=list)

    @property
    def held(self) -> list[Path]:
        return [entry.path for entry in self.entries]

    def release(self) -> None:
        """Give up every claim this watcher took.

        The file is unlinked before the lock is dropped, so there is no moment
        where the claim exists unlocked and a watcher starting up could read it
        as an abandoned claim to take over rather than as free ground.

        A file that is no longer the one we locked is left alone: it belongs to
        somebody else, and deleting it would hand their folder away.
        """
        for entry in self.entries:
            if _is_current(entry.fd, entry.path):
                entry.path.unlink(missing_ok=True)
            os.close(entry.fd)  # drops the flock
        self.entries.clear()


def claim(input_dir: Path, library_dir: Path) -> WatchClaim:
    """Claim an input folder and a library for this process.

    Raises:
        WatchConflict: if either is already claimed by a running watcher. Any
            claim taken before the conflict is released, so a refused watcher
            leaves nothing behind.
    """
    directory = locks_dir()
    directory.mkdir(parents=True, exist_ok=True)

    taken = WatchClaim()
    for kind, path in (("input", input_dir), ("library", library_dir)):
        try:
            taken.entries.append(_acquire(directory, kind, path))
        except WatchConflict:
            taken.release()
            raise
    return taken


def live_claims(directory: Path) -> dict[Path, int]:
    """Folders claimed by a watcher that is still running, by path.

    Liveness is checked rather than assumed: a claim outlives the watcher that
    took it — stopping the service sends `SIGTERM`, which does not run the
    release — so presence alone would report a stopped watch as running.

    The check is whether the claim's lock can be taken, not whether its pid
    answers: a recycled pid would otherwise report a stopped watch as running
    for as long as the unrelated process holding that number lives.
    """
    held: dict[Path, int] = {}
    if not directory.is_dir():
        return held
    for path in sorted(directory.glob("*.json")):
        holder = _read_holder(path)
        if holder is None or not _is_locked(path):
            continue
        try:
            held[Path(holder["path"])] = int(holder["pid"])
        except (KeyError, TypeError, ValueError):
            continue
    return held


def _acquire(directory: Path, kind: str, path: Path) -> _Held:
    resolved = path.resolve()
    digest = hashlib.sha256(str(resolved).encode("utf-8")).hexdigest()[:16]
    lock_path = directory / f"{kind}-{digest}.json"

    payload = json.dumps(
        {
            "pid": os.getpid(),
            "kind": kind,
            "path": str(resolved),
            "claimed_at_ms": int(time.time() * 1000),
        }
    )

    for _ in range(_ACQUIRE_ATTEMPTS):
        created = _create(directory, kind, digest, lock_path, payload)
        try:
            fd = os.open(lock_path, os.O_RDWR)
        except FileNotFoundError:
            continue  # released between our attempt to create it and this open

        try:
            fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except OSError as err:
            os.close(fd)
            if err.errno not in (errno.EACCES, errno.EAGAIN, errno.EWOULDBLOCK):
                raise
            holder = _read_holder(lock_path)
            owner = holder.get("pid") if holder else "unknown"
            raise WatchConflict(
                f"{resolved} is already being watched as the {kind} of a "
                f"watcher running as pid {owner}"
            ) from err

        # The lock is held, but a lock on a file that has since been unlinked
        # guards nothing: the previous owner releases by unlinking, and a third
        # watcher would create a fresh file at this name and lock that instead.
        if not _is_current(fd, lock_path):
            os.close(fd)
            continue

        if not created:
            # Taking over a claim whose owner is gone. Say who owns it now, so
            # the next watcher's refusal names a process that exists.
            os.ftruncate(fd, 0)
            os.pwrite(fd, payload.encode("utf-8"), 0)
        return _Held(path=lock_path, fd=fd)

    raise WatchConflict(f"could not claim {resolved} as {kind}")


def _create(
    directory: Path, kind: str, digest: str, lock_path: Path, payload: str
) -> bool:
    """Put a complete claim at `lock_path`. True if this call created it."""
    staged = directory / f".{kind}-{digest}.{os.getpid()}.tmp"
    try:
        staged.write_text(payload, encoding="utf-8")
        try:
            # Complete before it is visible, and atomic: link fails if taken.
            os.link(staged, lock_path)
            return True
        except FileExistsError:
            return False
    finally:
        staged.unlink(missing_ok=True)


def _is_current(fd: int, lock_path: Path) -> bool:
    """Whether `lock_path` still names the file this descriptor is open on."""
    try:
        on_disk = os.stat(lock_path)
    except OSError:
        return False
    held = os.fstat(fd)
    return (on_disk.st_dev, on_disk.st_ino) == (held.st_dev, held.st_ino)


def _is_locked(lock_path: Path) -> bool:
    """Whether some process still holds this claim's lock.

    Probing takes the lock for an instant and gives it straight back; only
    reporting paths use this, so the flicker cannot refuse a real watcher.
    """
    try:
        fd = os.open(lock_path, os.O_RDONLY)
    except OSError:
        return False
    try:
        fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except OSError:
        return True
    else:
        fcntl.flock(fd, fcntl.LOCK_UN)
        return False
    finally:
        os.close(fd)


def _read_holder(lock_path: Path) -> dict | None:
    try:
        holder = json.loads(lock_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        # Unreadable or half-written: nothing to report about its owner. The
        # lock, not this, is what says whether anyone still holds it.
        return None
    return holder if isinstance(holder, dict) else None
