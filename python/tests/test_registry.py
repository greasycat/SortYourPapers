from __future__ import annotations

import os
from pathlib import Path

import pytest

from sortyourpaperya.library import FilingMode
from sortyourpaperya.registry import RegistryError, load_registry, registry_path


def _write(body: str) -> Path:
    path = registry_path()
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(body, encoding="utf-8")
    return path


def test_a_missing_registry_is_empty_not_an_error() -> None:
    registry = load_registry()

    assert registry.watches == {}
    assert registry.default() is None


def test_reads_declared_watches() -> None:
    _write(
        """
        [watch.downloads]
        input   = "~/Downloads"
        library = "~/Documents/lib"
        mode    = "copy"
        """
    )

    entry = load_registry().get("downloads")

    assert entry.name == "downloads"
    assert entry.input_dir == Path.home().joinpath("Downloads").resolve()
    assert entry.mode is FilingMode.COPY


def test_two_watches_sharing_a_library_are_refused_by_name(tmp_path: Path) -> None:
    """Caught when the file is read, not when the second watcher starts."""
    _write(
        f"""
        [watch.one]
        input   = "{tmp_path / 'a'}"
        library = "{tmp_path / 'lib'}"

        [watch.two]
        input   = "{tmp_path / 'b'}"
        library = "{tmp_path / 'lib'}"
        """
    )

    with pytest.raises(RegistryError, match="'one' and 'two' share the same library"):
        load_registry()


def test_two_watches_sharing_an_input_are_refused_by_name(tmp_path: Path) -> None:
    _write(
        f"""
        [watch.one]
        input   = "{tmp_path / 'in'}"
        library = "{tmp_path / 'lib1'}"

        [watch.two]
        input   = "{tmp_path / 'in'}"
        library = "{tmp_path / 'lib2'}"
        """
    )

    with pytest.raises(RegistryError, match="share the same input"):
        load_registry()


def test_the_same_folder_written_two_ways_still_collides(tmp_path: Path) -> None:
    _write(
        f"""
        [watch.one]
        input   = "{tmp_path / 'in'}"
        library = "{tmp_path / 'lib1'}"

        [watch.two]
        input   = "{tmp_path / 'sub' / '..' / 'in'}"
        library = "{tmp_path / 'lib2'}"
        """
    )

    with pytest.raises(RegistryError, match="share the same input"):
        load_registry()


def test_a_lone_watch_is_the_default_without_saying_so(tmp_path: Path) -> None:
    _write(
        f"""
        [watch.only]
        input   = "{tmp_path / 'in'}"
        library = "{tmp_path / 'lib'}"
        """
    )

    assert load_registry().default().name == "only"


def test_several_watches_need_a_default_named(tmp_path: Path) -> None:
    # Picking one would be a guess, so there is no default until one is named.
    _write(
        f"""
        [watch.a]
        input   = "{tmp_path / 'a'}"
        library = "{tmp_path / 'la'}"

        [watch.b]
        input   = "{tmp_path / 'b'}"
        library = "{tmp_path / 'lb'}"
        """
    )

    assert load_registry().default() is None

    _write(
        f"""
        default = "b"

        [watch.a]
        input   = "{tmp_path / 'a'}"
        library = "{tmp_path / 'la'}"

        [watch.b]
        input   = "{tmp_path / 'b'}"
        library = "{tmp_path / 'lb'}"
        """
    )

    assert load_registry().default().name == "b"


def test_a_default_naming_nothing_is_refused(tmp_path: Path) -> None:
    _write(
        f"""
        default = "ghost"

        [watch.a]
        input   = "{tmp_path / 'a'}"
        library = "{tmp_path / 'la'}"
        """
    )

    with pytest.raises(RegistryError, match="not declared"):
        load_registry()


@pytest.mark.parametrize(
    "body,message",
    [
        ('[watch.a]\nlibrary = "/tmp/l"\n', "missing `input`"),
        ('[watch.a]\ninput = "/tmp/i"\n', "missing `library`"),
        ('[watch.a]\ninput="/tmp/i"\nlibrary="/tmp/l"\nmode="sideways"\n', "not one of"),
        ("this is not toml at all [[[", "could not read"),
    ],
)
def test_a_broken_declaration_says_what_is_wrong(body, message) -> None:
    _write(body)

    with pytest.raises(RegistryError, match=message):
        load_registry()


def test_an_unknown_name_lists_what_is_declared(tmp_path: Path) -> None:
    _write(
        f"""
        [watch.downloads]
        input   = "{tmp_path / 'in'}"
        library = "{tmp_path / 'lib'}"
        """
    )

    with pytest.raises(RegistryError, match="downloads"):
        load_registry().get("nope")
