# AGENTS.md

## Project Context

- Project name: `SortYourPapers` (`syp`)
- Primary language/runtime: Rust workspace (`edition = "2024"`) with Python 3.11 maintainer tooling under `python/`

## Working Rules

### Version Control

- Always commit after completing a change.
- Always use conventional commit messages such as `feat`, `fix`, `docs`, `refactor`, `chore`, and similar types.
- Always create a new branch before starting a large change.

### Editing

- Prefer small, targeted changes.
- Keep new code consistent with the existing workspace structure and style.
- During refactors, backward compatibility is not required. The project is still in development.

### Communication

- Be concise and action-oriented.
- Summarize what changed and how it was verified.

## Task-Specific Notes

- Constraints: preserve the Rust workspace layout across `crates/syp-core`, `crates/syp`, `crates/syptui`, `crates/paperdb`, and the separate Python tooling under `python/`.
- Preferences: default to pragmatic implementation, keep diffs reviewable, and treat `README.md` as the source of truth for user-facing behavior and commands.
- Commands to know: `cargo fmt --all`, `cargo test --workspace`, `cargo clippy --workspace --all-targets`, `cargo run -p syp -- --help`, `cargo run -p syptui -- --help`, `uv run --project python pytest`.
- Definition of done: code or docs are updated, relevant verification has been run or explicitly noted if skipped, and the change is committed with a conventional commit message.
