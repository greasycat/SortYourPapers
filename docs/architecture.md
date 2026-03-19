# SortYourPapers Architecture

## Crate Layout
- `src/cli.rs`: clap argument parsing and user-facing command definitions.
- `src/config/`: runtime config loading and precedence resolution for CLI, env, and XDG config.
- `src/app/`: top-level run entrypoints and stage orchestration.
- `src/session/`: run orchestration runtime, persisted workspace state, and session-oriented commands like resume and rerun.
- `src/papers/`: PDF discovery, extraction, and text preprocessing.
- `src/taxonomy/`: keyword extraction and taxonomy synthesis.
- `src/placement/`: output inspection, placement prompt construction, and placement validation.
- `src/fs_ops/`: plan building and filesystem execution.
- `src/llm/`: provider clients plus shared schema, retry, and client abstractions.
- `src/terminal/`: verbosity/progress helpers and terminal rendering.

## Compatibility Notes
- CLI commands, flags, environment variables, and XDG paths remain unchanged.
- Resume artifact filenames and serialized state shapes are intentionally preserved.
- Internal session module paths are not treated as stable during development; callers should use root-level `session` re-exports.

## Historical Docs
- The original greenfield implementation plan now lives at `docs/archive/initial-implementation-plan.md`.
