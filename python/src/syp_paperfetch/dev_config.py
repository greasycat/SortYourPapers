from __future__ import annotations

import os
import tomllib
from pathlib import Path

DEV_CONFIG_FILE = "dev.toml"
TESTSETS_SECTION = "testsets"
CACHE_DIR_KEY = "cache_dir"


def shared_testsets_cache_root() -> Path:
    for start in (Path.cwd(), Path(__file__).resolve()):
        resolved = shared_testsets_cache_root_from(start)
        if resolved is not None:
            return resolved
    return xdg_testsets_cache_root()


def shared_testsets_cache_root_from(start: Path) -> Path | None:
    dev_config_path = find_dev_config_path(start)
    if dev_config_path is None:
        return None

    raw = tomllib.loads(dev_config_path.read_text(encoding="utf-8"))
    section = raw.get(TESTSETS_SECTION, {})
    if not isinstance(section, dict) or CACHE_DIR_KEY not in section:
        raise ValueError(f"missing [{TESTSETS_SECTION}].{CACHE_DIR_KEY} in {dev_config_path}")

    relative = Path(str(section[CACHE_DIR_KEY]))
    return dev_config_path.parent.joinpath(relative)


def xdg_testsets_cache_root() -> Path:
    xdg_cache_home = os.environ.get("XDG_CACHE_HOME")
    if xdg_cache_home:
        return Path(xdg_cache_home) / "sortyourpapers" / "testsets"
    return Path.home() / ".cache" / "sortyourpapers" / "testsets"


def find_dev_config_path(start: Path) -> Path | None:
    candidate = start.resolve()
    anchor = candidate if candidate.is_dir() else candidate.parent
    for directory in (anchor, *anchor.parents):
        path = directory / DEV_CONFIG_FILE
        if path.is_file():
            return path
    return None
