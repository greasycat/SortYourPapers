# AGENTS.md

## Project Context

- Project name: `SortYourPapers` (`syp`)
- Primary language/runtime: Python 3.11+. The tool lives in `python/` and installs as `sypy`.
- The Rust workspace this project began as is preserved on the `old-rust` branch and is not maintained.

## Working Rules

- If a suitable well-known library exists for the task, use it rather than re-inventing the wheel, unless explicitly told not to.

### Version Control

- Always commit after completing a change.
- Always use conventional commit messages such as `feat`, `fix`, `docs`, `refactor`, `chore`, and similar types.
- Always create a new branch before starting a large change.
- Always output a one-liner change to CHANGELOG.md for dev-friendly inspection, should be more verbose than the commit message.

### Editing

- Prefer small, targeted changes.
- Keep new code consistent with the existing structure and style.
- During refactors, backward compatibility is not required. The project is still in development.

### Communication

- Be concise and action-oriented.
- Summarize what changed and how it was verified.

## Tmux (Recommended but still optional)
- Always use tmux to run time-intensive background tests
- Always use an short and relevant session name
- Always outputs logs in additional to tmux

## Task-Specific Notes

- Commands to know: `python/.venv/bin/python -m pytest` (run from `python/`), `./python/scripts/sypy-path wire`, `sypy --help`.
- Every test is capped at 60 seconds by `pytest-timeout` using the signal method, so a loop that stops awaiting cannot wedge the machine.
