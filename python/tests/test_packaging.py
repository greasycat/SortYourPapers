"""The lock file: what `wire` actually installs."""

from __future__ import annotations

import re
import tomllib
from pathlib import Path

PROJECT = Path(__file__).resolve().parents[1]
LOCK = PROJECT / "requirements.lock"


def _normalise(name: str) -> str:
    return re.sub(r"[-_.]+", "-", name).lower()


def _locked() -> dict[str, str]:
    pinned: dict[str, str] = {}
    for line in LOCK.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        name, _, version = line.partition("==")
        assert version, f"{line!r} is not pinned to an exact version"
        pinned[_normalise(name)] = version
    return pinned


def _declared() -> list[str]:
    payload = tomllib.loads((PROJECT / "pyproject.toml").read_text(encoding="utf-8"))
    return [
        _normalise(re.split(r"[<>=!~\[]", spec, maxsplit=1)[0].strip())
        for spec in payload["project"]["dependencies"]
    ]


def test_every_dependency_is_locked() -> None:
    """A dependency added without relocking is one `wire` resolves fresh.

    That is how two machines set up a week apart end up running different code,
    and how a break that arrived with a dependency becomes indistinguishable
    from one that arrived with a commit.
    """
    missing = sorted(set(_declared()) - _locked().keys())

    assert not missing, (
        f"not in requirements.lock: {missing}. "
        "Run ./python/scripts/sypy-path relock"
    )


def test_the_lock_pins_transitive_dependencies_too() -> None:
    # Pinning only the direct ones leaves the resolver free where most of the
    # code actually comes from.
    locked = _locked()

    assert len(locked) > len(_declared())
    assert "httpx2" in locked or "httpx" in locked, "openai's transport is not pinned"


def test_the_lock_says_how_to_regenerate_it() -> None:
    # A pinned file nobody knows how to move forward is one people edit by hand.
    assert "relock" in LOCK.read_text(encoding="utf-8")
