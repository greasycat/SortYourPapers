"""Desktop notifications, so a watcher running in the background can be seen.

A service's account of a pass goes to a log file nobody is reading while it
runs, and a document filed under the wrong category is worth knowing about at
the time rather than the next time the log is opened.

Best-effort by design. A machine with no notifier, a session bus a service
cannot reach, a notifier that hangs — none of them may cost the watcher a pass,
so every failure is swallowed after a line at debug level.
"""

from __future__ import annotations

import logging
import shutil
import subprocess
import sys

log = logging.getLogger(__name__)

# A notifier that never returns must not stall the pass waiting behind it.
TIMEOUT_SECONDS = 5.0


def notify(title: str, body: str) -> bool:
    """Show one desktop notification. True if a notifier took it.

    Never raises: the caller is a loop that must outlive anything this does.
    """
    command = _command(title, body)
    if command is None:
        log.debug("no desktop notifier on this machine")
        return False
    try:
        subprocess.run(
            command, check=True, timeout=TIMEOUT_SECONDS, capture_output=True
        )
    except (OSError, subprocess.SubprocessError) as err:
        log.debug("could not notify: %s", err)
        return False
    return True


def _command(title: str, body: str) -> list[str] | None:
    """The notifier this machine has, or None if it has neither.

    Both take the text as arguments rather than built into a script or a shell
    line, so a title or a category holding a quote cannot change what is run.
    """
    if sys.platform == "darwin":
        return [
            "osascript",
            "-e", "on run argv",
            "-e", "display notification (item 1 of argv) with title (item 2 of argv)",
            "-e", "end run",
            "--", body, title,
        ]
    if shutil.which("notify-send"):
        return ["notify-send", "--app-name=sortyourpaperya", "--", title, body]
    return None
