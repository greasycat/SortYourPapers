"""Being told is a convenience; filing is the job. It must survive the other."""

from __future__ import annotations

import subprocess

import pytest

from sortyourpaperya import notify as notify_module
from sortyourpaperya.notify import notify


def test_a_notifier_that_fails_is_swallowed(monkeypatch: pytest.MonkeyPatch) -> None:
    """The caller is a loop that must outlive a missing session bus."""
    monkeypatch.setattr(notify_module, "_command", lambda title, body: ["notify"])
    monkeypatch.setattr(
        notify_module.subprocess,
        "run",
        lambda *a, **k: (_ for _ in ()).throw(
            subprocess.CalledProcessError(1, "notify")
        ),
    )

    assert notify("Filed 1 document", "a -> AI") is False


def test_a_notifier_that_hangs_is_given_a_deadline(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    seen: dict = {}
    monkeypatch.setattr(notify_module, "_command", lambda title, body: ["notify"])
    monkeypatch.setattr(
        notify_module.subprocess, "run", lambda *a, **k: seen.update(k)
    )

    notify("t", "b")

    assert seen["timeout"] == notify_module.TIMEOUT_SECONDS


def test_a_machine_with_no_notifier_says_so_rather_than_failing(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(notify_module.sys, "platform", "linux")
    monkeypatch.setattr(notify_module.shutil, "which", lambda name: None)

    assert notify("t", "b") is False


@pytest.mark.parametrize("platform", ["darwin", "linux"])
def test_the_text_is_an_argument_and_never_part_of_a_script(
    monkeypatch: pytest.MonkeyPatch, platform: str
) -> None:
    """A category comes from a model, so it is not this tool's to interpolate."""
    monkeypatch.setattr(notify_module.sys, "platform", platform)
    monkeypatch.setattr(notify_module.shutil, "which", lambda name: "/usr/bin/" + name)

    hostile = 'Law" & (do shell script "touch /tmp/pwned") & "'
    command = notify_module._command("Filed 1 document", hostile)

    assert hostile in command, command
    assert not any(hostile in part for part in command if part != hostile), command
