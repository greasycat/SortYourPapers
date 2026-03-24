# AGENTS.md

## Project Context

- Project name: `SortYourPapers` (`syp`)
- Primary language/runtime: Rust workspace (`edition = "2024"`) with Python 3.12.11 maintainer tooling under `python/` (manged by uv)

## Working Rules

- If some popular crates or libraries exists and suitable for the task. Then use them instead of re-inventing the wheel unless explicityly note not to.

### Version Control

- Always commit after completing a change.
- Always use conventional commit messages such as `feat`, `fix`, `docs`, `refactor`, `chore`, and similar types.
- Always create a new branch before starting a large change.
- Always output a one-liner change to CHANGELOG.md for dev-friendly inspection, should be more verbose than the commit message.

### Editing

- Prefer small, targeted changes.
- Keep new code consistent with the existing workspace structure and style.
- During refactors, backward compatibility is not required. The project is still in development.

### Communication

- Be concise and action-oriented.
- Summarize what changed and how it was verified.

## Tmux (Recommended but still optional)
- Always use tmux to run time-intensive background tests
- Always use an short and relevant session name
- Always outputs logs in additional to tmux

## Task-Specific Notes

- Commands to know: `cargo fmt --all`, `cargo test --workspace`, `cargo clippy --workspace --all-targets`, `cargo run -p syp -- --help`, `cargo run -p syptui -- --help`, `uv run --project python pytest`.
