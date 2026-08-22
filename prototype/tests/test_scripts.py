"""The install and service scripts.

They are the first thing anyone runs and the only part of the project that
touches a supervisor, so what they generate is worth pinning. The systemd path
cannot run on macOS and the launchd path cannot run on Linux, so both are
exercised with the supervisor command stubbed on PATH: the unit the script
writes is the thing being checked, not whether this machine can load it.
"""

from __future__ import annotations

import os
import shutil
import stat
import subprocess
from pathlib import Path

import pytest

PROJECT = Path(__file__).resolve().parents[2]
INSTALL = PROJECT / "install.sh"
SERVICE = PROJECT / "prototype" / "scripts" / "sypy-service"
WIRE = PROJECT / "prototype" / "scripts" / "sypy-path"


def _stub(directory: Path, name: str, body: str = "exit 0") -> Path:
    """A command that answers instead of doing anything."""
    directory.mkdir(parents=True, exist_ok=True)
    path = directory / name
    path.write_text(f"#!/usr/bin/env bash\n{body}\n", encoding="utf-8")
    path.chmod(path.stat().st_mode | stat.S_IEXEC | stat.S_IXGRP | stat.S_IXOTH)
    return path


def _run(script: Path, *args: str, env: dict, cwd: Path | None = None):
    return subprocess.run(
        [str(script), *args],
        capture_output=True,
        text=True,
        env={**os.environ, **env},
        cwd=str(cwd or PROJECT),
    )


@pytest.fixture
def home(tmp_path: Path) -> Path:
    (tmp_path / "home").mkdir()
    return tmp_path / "home"


@pytest.fixture
def watched(tmp_path: Path) -> tuple[Path, Path]:
    inbox, library = tmp_path / "inbox", tmp_path / "library"
    inbox.mkdir()
    return inbox, library


def _service_env(home: Path, tmp_path: Path, init: str, extra: dict | None = None) -> dict:
    """Point the script at a throwaway HOME with a stubbed supervisor."""
    stubs = tmp_path / "stubs"
    _stub(stubs, "systemctl", 'echo "systemctl $*" >> "$SYPY_TEST_CALLS"')
    _stub(stubs, "launchctl", 'echo "launchctl $*" >> "$SYPY_TEST_CALLS"; exit 1')
    _stub(stubs, "loginctl")
    env = {
        "HOME": str(home),
        "SYPY_INIT_SYSTEM": init,
        "SYPY_UNIT_DIR": str(home / "units"),
        "SYPY_LOG_DIR": str(home / "logs"),
        "SYPY_VENV_DIR": str(tmp_path / "venv"),
        "SYPY_TEST_CALLS": str(tmp_path / "calls.txt"),
        "PATH": f"{stubs}:{os.environ['PATH']}",
    }
    env.update(extra or {})
    # A `sypy` for the script to find. It never runs: both folders are passed.
    fake_venv_bin = tmp_path / "venv" / "bin"
    _stub(fake_venv_bin, "sypy")
    return env


# ---- syntax ----------------------------------------------------------------


@pytest.mark.parametrize("script", [INSTALL, SERVICE, WIRE], ids=lambda p: p.name)
def test_the_script_parses(script: Path) -> None:
    assert subprocess.run(["bash", "-n", str(script)]).returncode == 0


@pytest.mark.parametrize("script", [INSTALL, SERVICE, WIRE], ids=lambda p: p.name)
def test_the_script_is_executable(script: Path) -> None:
    # It is documented as `./install.sh`, which only works if it is.
    assert os.access(script, os.X_OK)


# ---- what the service scripts generate -------------------------------------


def test_the_systemd_unit_runs_the_watcher_on_the_named_folders(
    home: Path, tmp_path: Path, watched: tuple[Path, Path]
) -> None:
    inbox, library = watched
    env = _service_env(home, tmp_path, "systemd")

    result = _run(SERVICE, "install", str(inbox), str(library), env=env)

    assert result.returncode == 0, result.stderr
    unit = (home / "units" / "sypy.service").read_text()
    assert f'--input "{inbox}"' in unit
    assert f'--library "{library}"' in unit
    assert "--mode copy" in unit


def test_the_systemd_unit_restarts_but_gives_up_on_a_crashloop(
    home: Path, tmp_path: Path, watched: tuple[Path, Path]
) -> None:
    """Restarting is how a transient failure recovers.

    A service that fails instantly and forever should stop instead of spinning,
    leaving the reason somewhere it can be read.
    """
    inbox, library = watched
    _run(SERVICE, "install", str(inbox), str(library), env=_service_env(home, tmp_path, "systemd"))

    unit = (home / "units" / "sypy.service").read_text()
    assert "Restart=always" in unit
    assert "RestartSec=" in unit
    assert "StartLimitBurst=" in unit


def test_the_systemd_unit_carries_poppler_and_the_rotating_log(
    home: Path, tmp_path: Path, watched: tuple[Path, Path]
) -> None:
    # A user service gets a bare PATH; without pdftoppm on it every scanned PDF
    # fails under the service while working by hand. Without SYPY_LOG_FILE the
    # log lands in a capture file nothing trims.
    inbox, library = watched
    poppler = tmp_path / "poppler"
    _stub(poppler, "pdftoppm")
    env = _service_env(home, tmp_path, "systemd")
    env["PATH"] = f"{poppler}:{env['PATH']}"

    _run(SERVICE, "install", str(inbox), str(library), env=env)

    unit = (home / "units" / "sypy.service").read_text()
    assert f"Environment=PATH={poppler}:" in unit
    assert f"Environment=SYPY_LOG_FILE={home}/logs/sypy.log" in unit


def test_installing_a_systemd_service_enables_it(
    home: Path, tmp_path: Path, watched: tuple[Path, Path]
) -> None:
    # Written but never enabled is a service that does nothing until reboot.
    inbox, library = watched
    env = _service_env(home, tmp_path, "systemd")

    _run(SERVICE, "install", str(inbox), str(library), env=env)

    calls = Path(env["SYPY_TEST_CALLS"]).read_text()
    assert "systemctl --user daemon-reload" in calls
    assert "systemctl --user enable --now sypy.service" in calls


def test_the_launchd_plist_runs_the_watcher_on_the_named_folders(
    home: Path, tmp_path: Path, watched: tuple[Path, Path]
) -> None:
    import plistlib

    inbox, library = watched
    env = _service_env(home, tmp_path, "launchd")

    _run(SERVICE, "install", str(inbox), str(library), env=env)

    plist = home / "Library" / "LaunchAgents" / "com.sortyourpapers.sypy.plist"
    payload = plistlib.loads(plist.read_bytes())
    args = payload["ProgramArguments"]
    assert args[args.index("--input") + 1] == str(inbox)
    assert args[args.index("--library") + 1] == str(library)
    assert payload["KeepAlive"] is True
    assert payload["EnvironmentVariables"]["SYPY_LOG_FILE"].endswith("sypy.log")


def test_a_watched_folder_with_a_space_survives_the_unit(
    home: Path, tmp_path: Path
) -> None:
    """`~/My Documents` is an ordinary thing to watch.

    systemd splits ExecStart on whitespace, so an unquoted path would reach the
    watcher as two arguments and the service would never start.
    """
    inbox = tmp_path / "My Downloads"
    inbox.mkdir()
    library = tmp_path / "My Library"
    env = _service_env(home, tmp_path, "systemd")

    _run(SERVICE, "install", str(inbox), str(library), env=env)
    unit = (home / "units" / "sypy.service").read_text()
    assert f'--input "{inbox}"' in unit

    # And it reads back as one argument, which is what refuses a second folder.
    other = tmp_path / "other"
    other.mkdir()
    result = _run(SERVICE, "install", str(other), str(library), env=env)
    assert "My Downloads" in result.stderr


@pytest.mark.parametrize("init", ["systemd", "launchd"])
def test_installing_over_a_different_folder_is_refused(
    home: Path, tmp_path: Path, watched: tuple[Path, Path], init: str
) -> None:
    """One service, one name.

    Installing for a different folder would replace the running one without
    saying so, leaving someone believing both folders were watched.
    """
    inbox, library = watched
    other = tmp_path / "other-inbox"
    other.mkdir()
    env = _service_env(home, tmp_path, init)
    if init == "launchd":
        # `is_loaded` asks the supervisor; say yes so the guard is reached.
        _stub(tmp_path / "stubs", "launchctl", 'echo "launchctl $*" >> "$SYPY_TEST_CALLS"')
    _run(SERVICE, "install", str(inbox), str(library), env=env)

    result = _run(SERVICE, "install", str(other), str(library), env=env)

    assert result.returncode != 0
    assert "already watching" in result.stderr
    assert str(inbox) in result.stderr


def test_reinstalling_the_same_folder_is_how_settings_are_picked_up(
    home: Path, tmp_path: Path, watched: tuple[Path, Path]
) -> None:
    inbox, library = watched
    env = _service_env(home, tmp_path, "systemd")
    _run(SERVICE, "install", str(inbox), str(library), env=env)

    result = _run(SERVICE, "install", str(inbox), str(library), env={**env, "SYPY_MODE": "move"})

    assert result.returncode == 0, result.stderr
    assert "--mode move" in (home / "units" / "sypy.service").read_text()


def test_an_unknown_mode_is_refused_before_anything_is_written(
    home: Path, tmp_path: Path, watched: tuple[Path, Path]
) -> None:
    inbox, library = watched
    env = _service_env(home, tmp_path, "systemd", {"SYPY_MODE": "delete-everything"})

    result = _run(SERVICE, "install", str(inbox), str(library), env=env)

    assert result.returncode != 0
    assert "SYPY_MODE" in result.stderr
    assert not (home / "units" / "sypy.service").exists()


def test_uninstalling_removes_the_unit(
    home: Path, tmp_path: Path, watched: tuple[Path, Path]
) -> None:
    inbox, library = watched
    env = _service_env(home, tmp_path, "systemd")
    _run(SERVICE, "install", str(inbox), str(library), env=env)
    assert (home / "units" / "sypy.service").exists()

    result = _run(SERVICE, "uninstall", env=env)

    assert result.returncode == 0, result.stderr
    assert not (home / "units" / "sypy.service").exists()


# ---- the installer ---------------------------------------------------------


def test_check_reports_a_missing_interpreter_and_changes_nothing(
    home: Path, tmp_path: Path
) -> None:
    # An empty PATH but for the shell: no python of any name is findable.
    bare = tmp_path / "bare"
    bare.mkdir()
    for name in ("bash", "uname", "dirname", "basename", "command", "id"):
        found = shutil.which(name)
        if found:
            (bare / name).symlink_to(found)

    result = _run(
        INSTALL, "--check", env={"HOME": str(home), "PATH": str(bare), "NO_COLOR": "1"}
    )

    assert result.returncode != 0
    assert "no Python" in result.stdout + result.stderr
    assert not (tmp_path / "venv").exists()


def test_a_python_too_old_to_run_this_is_refused(home: Path, tmp_path: Path) -> None:
    """Present is not the same as usable.

    A distribution's `python3` is often older than the newest it also ships, so
    finding one says nothing until its version has been asked for.
    """
    stubs = tmp_path / "stubs"
    for name in ("python3.14", "python3.13", "python3.12", "python3.11", "python3"):
        _stub(stubs, name, "exit 1")  # fails the >= 3.11 check

    result = _run(
        INSTALL,
        "--check",
        env={
            "HOME": str(home),
            "PATH": f"{stubs}:{os.environ['PATH']}",
            "NO_COLOR": "1",
        },
    )

    assert result.returncode != 0
    assert "no Python 3.11 or newer" in result.stdout + result.stderr


def test_check_names_the_package_that_provides_venv(home: Path, tmp_path: Path) -> None:
    """Debian ships `venv` separately, so a working python3 is not enough.

    Without this the failure is an opaque one from inside ensurepip.
    """
    stubs = tmp_path / "stubs"
    without_venv = (
        'if [ "$1" = "-c" ]; then '
        'case "$2" in *ensurepip*) exit 1;; *version_info*) exit 0;; esac; fi; exit 0'
    )
    # Every name the installer tries, or it finds a real interpreter instead.
    for name in ("python3.14", "python3.13", "python3.12", "python3.11", "python3"):
        _stub(stubs, name, without_venv)
    _stub(stubs, "apt-get")
    env = {
        "HOME": str(home),
        "PATH": f"{stubs}:{os.environ['PATH']}",
        "NO_COLOR": "1",
    }

    result = _run(INSTALL, "--check", env=env)

    assert result.returncode != 0
    assert "python3-venv" in result.stdout + result.stderr


def test_a_missing_pdftoppm_is_a_warning_not_a_refusal(
    home: Path, tmp_path: Path
) -> None:
    # Only scanned documents need it, and that failure is reported per document
    # rather than stopping a pass — so it must not block the install.
    bare = tmp_path / "bare"
    bare.mkdir()
    for name in ("bash", "uname", "dirname", "basename", "id", "python3", "apt-get"):
        found = shutil.which(name)
        if found and not (bare / name).exists():
            (bare / name).symlink_to(found)

    result = _run(INSTALL, "--check", env={"HOME": str(home), "PATH": str(bare), "NO_COLOR": "1"})

    output = result.stdout + result.stderr
    assert "pdftoppm not found" in output
    assert result.returncode == 0, output


def test_the_installer_says_how_to_put_it_on_path(home: Path, tmp_path: Path) -> None:
    # The one step that is not automatic, so it has to be spelled out.
    result = _run(
        INSTALL, "--help", env={"HOME": str(home), "NO_COLOR": "1"}
    )

    assert "SYPY_BIN_DIR" in result.stderr
