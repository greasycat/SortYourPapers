"""The one file that says what is watched, and where it is filed.

Without it every command carries its own `--input` and `--library`, nothing can
say what is configured as opposed to what happens to be running, and two watches
pointed at one library are only discovered when the second one starts.

Watches are declared by name here instead:

    default = "downloads"

    [watch.downloads]
    input   = "~/Downloads"
    library = "~/Documents/sypy-library"
    mode    = "copy"

Conflicts are refused when the file is read, so a mistake is a message about
two names rather than a watcher that will not start hours later. This is
configuration; the running claims in `watchlock` are what actually enforce one
watcher per folder.
"""

from __future__ import annotations

import os
import tomllib
from dataclasses import dataclass
from pathlib import Path

from .library import FilingMode

CONFIG_FILE = "config.toml"


class RegistryError(RuntimeError):
    """Raised when the registry cannot be read or contradicts itself."""


@dataclass(frozen=True)
class WatchEntry:
    """One declared watch."""

    name: str
    input_dir: Path
    library_dir: Path
    mode: FilingMode


@dataclass(frozen=True)
class Registry:
    """Every declared watch, and which one commands fall back to."""

    path: Path
    watches: dict[str, WatchEntry]
    default_name: str | None = None

    def get(self, name: str) -> WatchEntry:
        """Look up a watch by name.

        Raises:
            RegistryError: if no watch has that name.
        """
        if name not in self.watches:
            known = ", ".join(sorted(self.watches)) or "none declared"
            raise RegistryError(f"no watch named {name!r} in {self.path} ({known})")
        return self.watches[name]

    def default(self) -> WatchEntry | None:
        """The watch commands use when none is named, if there is one.

        A single declared watch is the default without having to say so; past
        that, silence would be a guess, so `default` must name one.
        """
        if self.default_name:
            return self.watches.get(self.default_name)
        if len(self.watches) == 1:
            return next(iter(self.watches.values()))
        return None


def registry_path() -> Path:
    base = os.environ.get("SYPY_CONFIG_DIR") or os.environ.get("XDG_CONFIG_HOME")
    root = Path(base) if base else Path.home() / ".config"
    return Path(root) / "sypy" / CONFIG_FILE


def load_registry() -> Registry:
    """Read the registry, or an empty one when the file does not exist.

    Raises:
        RegistryError: if the file is unreadable, malformed, or declares two
            watches sharing an input folder or a library.
    """
    path = registry_path()
    if not path.is_file():
        return Registry(path=path, watches={})

    try:
        raw = tomllib.loads(path.read_text(encoding="utf-8"))
    except (OSError, tomllib.TOMLDecodeError) as err:
        raise RegistryError(f"could not read {path}: {err}") from err

    declared = raw.get("watch") or {}
    if not isinstance(declared, dict):
        raise RegistryError(f"{path}: `watch` must be a table of named watches")

    watches: dict[str, WatchEntry] = {}
    for name, body in declared.items():
        watches[name] = _entry(path, name, body)

    _refuse_shared_folders(path, watches)

    default_name = raw.get("default")
    if default_name is not None:
        if not isinstance(default_name, str):
            raise RegistryError(f"{path}: `default` must be a watch name")
        if default_name not in watches:
            raise RegistryError(
                f"{path}: `default` names {default_name!r}, which is not declared"
            )
    return Registry(path=path, watches=watches, default_name=default_name)


def _entry(path: Path, name: str, body: object) -> WatchEntry:
    if not isinstance(body, dict):
        raise RegistryError(f"{path}: [watch.{name}] must be a table")
    for key in ("input", "library"):
        if not body.get(key):
            raise RegistryError(f"{path}: [watch.{name}] is missing `{key}`")

    raw_mode = body.get("mode", FilingMode.COPY.value)
    try:
        mode = FilingMode(raw_mode)
    except ValueError as err:
        allowed = ", ".join(item.value for item in FilingMode)
        raise RegistryError(
            f"{path}: [watch.{name}] mode {raw_mode!r} is not one of {allowed}"
        ) from err

    return WatchEntry(
        name=name,
        input_dir=_expand(body["input"]),
        library_dir=_expand(body["library"]),
        mode=mode,
    )


def _refuse_shared_folders(path: Path, watches: dict[str, WatchEntry]) -> None:
    """Refuse two watches sharing a folder, naming both.

    Caught here rather than when the second watcher starts, so the mistake is
    visible while it is still being made.
    """
    for field, attribute in (("input", "input_dir"), ("library", "library_dir")):
        seen: dict[Path, str] = {}
        for entry in watches.values():
            folder = getattr(entry, attribute)
            other = seen.get(folder)
            if other is not None:
                first, second = sorted((other, entry.name))
                raise RegistryError(
                    f"{path}: watches {first!r} and {second!r} share the same "
                    f"{field} folder {folder}. Two watchers on one folder file "
                    "the same document twice."
                )
            seen[folder] = entry.name


def _expand(value: object) -> Path:
    return Path(str(value)).expanduser().resolve()
