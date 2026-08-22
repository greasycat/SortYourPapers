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

Creation is atomic (`O_EXCL`), so two watchers starting at the same moment
cannot both win. A claim whose owner is gone is taken over rather than
respected, so a crash does not lock a folder out forever.
"""

from __future__ import annotations

import hashlib
import json
import os
import time
from dataclasses import dataclass, field
from pathlib import Path


class WatchConflict(RuntimeError):
    """Raised when a folder is already claimed by a running watcher."""


def locks_dir() -> Path:
    """Where claims are kept: outside every library, so they are machine-wide."""
    state = os.environ.get("SYPY_STATE_DIR") or os.environ.get("XDG_STATE_HOME")
    base = Path(state) if state else Path.home() / ".local" / "state"
    return Path(base) / "sypy" / "watch-locks"


@dataclass
class WatchClaim:
    """The claims one watcher holds. Release them when it stops."""

    held: list[Path] = field(default_factory=list)

    def release(self) -> None:
        """Give up every claim this watcher took.

        Only removes claims still owned by this process, so a lock another
        watcher has since taken over is left alone.
        """
        for path in self.held:
            holder = _read_holder(path)
            if holder is not None and holder.get("pid") == os.getpid():
                path.unlink(missing_ok=True)
        self.held.clear()


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
            taken.held.append(_acquire(directory, kind, path))
        except WatchConflict:
            taken.release()
            raise
    return taken


def _acquire(directory: Path, kind: str, path: Path) -> Path:
    resolved = path.resolve()
    digest = hashlib.sha256(str(resolved).encode("utf-8")).hexdigest()[:16]
    lock_path = directory / f"{kind}-{digest}.json"

    # Two passes at most: the second is for a claim found to be abandoned.
    for _ in range(2):
        try:
            handle = os.open(lock_path, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
        except FileExistsError:
            holder = _read_holder(lock_path)
            if holder is not None and _alive(holder.get("pid")):
                raise WatchConflict(
                    f"{resolved} is already being watched as the {kind} of a "
                    f"watcher running as pid {holder.get('pid')}"
                )
            # The owner is gone, so the claim is not worth respecting.
            lock_path.unlink(missing_ok=True)
            continue

        with os.fdopen(handle, "w", encoding="utf-8") as stream:
            json.dump(
                {
                    "pid": os.getpid(),
                    "kind": kind,
                    "path": str(resolved),
                    "claimed_at_ms": int(time.time() * 1000),
                },
                stream,
            )
        return lock_path

    raise WatchConflict(f"could not claim {resolved} as {kind}")


def _read_holder(lock_path: Path) -> dict | None:
    try:
        return json.loads(lock_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        # Unreadable or half-written: treat it as nobody's, so a crash midway
        # through writing one does not lock a folder out forever.
        return None


def _alive(pid: object) -> bool:
    if not isinstance(pid, int):
        return False
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True  # running, just not ours to signal
    return True
