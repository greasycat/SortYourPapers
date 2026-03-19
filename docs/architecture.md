# SortYourPapers Architecture

This document is a repo-structure reference for the current codebase. It reflects the source tree as it exists now, not the original implementation plan.

## Repository Layout
- `assets/papers/`: sample PDFs used for local runs and manual testing.
- `docs/`: project documentation.
- `docs/archive/`: historical planning material kept for reference.
- `src/`: application code for the library crate and both binaries.
- `target/`: Cargo build output.

## Binary Entrypoints
- `src/main.rs`: primary `sortyourpapers` binary. Parses the standard clap CLI and dispatches into the library entrypoints.
- `src/bin/syp.rs`: alternate `syp` binary. Defaults to the TUI, but can forward to the clap CLI via `syp cli ...`.
- `src/lib.rs`: crate module wiring plus the public re-exports used by both binaries.

## Top-Level Source Modules
- `src/cli.rs`: clap argument types for the batch CLI, including run arguments, session commands, and `extract-text`.
- `src/syp.rs`: `syp` frontend command model and dispatch rules.
- `src/entrypoints.rs`: shared CLI dispatch and top-level error hint printing.
- `src/error.rs`: application error types and shared result aliases.
- `src/report.rs`: final run report structures and file action summaries.
- `src/app/`: orchestration for a full sorting run, including config resolution handoff, debug-TUI seeded runs, and report rendering.
- `src/config/`: config loading and precedence resolution across CLI args, environment variables, XDG config, and defaults.
- `src/llm/`: provider-specific clients plus shared batching, retry, and schema logic.
- `src/papers/`: the paper-processing pipeline, including discovery, extraction, preprocessing, taxonomy synthesis, placement, and filesystem planning.
- `src/session/`: persisted run state, resume/rerun/review commands, and workspace artifact management.
- `src/terminal/`: CLI output backend, verbosity handling, and report printing helpers.
- `src/tui/`: `ratatui` frontend, forms, backend event handling, and session/run screens.

## Papers Pipeline Layout
The core workflow is grouped under `src/papers/` rather than split across top-level modules.

- `src/papers/discovery.rs`: scans input and output trees for candidate PDFs.
- `src/papers/extract.rs`: PDF text extraction and extraction batching helpers.
- `src/papers/preprocess.rs` and `src/papers/preprocess/`: text cleanup and term preparation before LLM calls.
- `src/papers/taxonomy/`: keyword extraction, preliminary category generation, taxonomy synthesis, batching, prompts, and validation.
- `src/papers/placement/`: output inspection, placement prompts, placement batching/runtime logic, and validation.
- `src/papers/fs_ops/`: plan construction and filesystem execution for preview/apply mode.
- `src/papers/mod.rs`: shared pipeline data types such as `PdfCandidate`, `PaperText`, keyword state, and synthesized category state.

## TUI Layout
- `src/tui/app.rs`: application state and key-driven behavior.
- `src/tui/render.rs`: screen rendering.
- `src/tui/forms/`: run configuration forms for interactive launches.
- `src/tui/session_view.rs`: saved-session browsing and review views.
- `src/tui/backend.rs` and `src/tui/input.rs`: backend event integration and input handling.

## Run Stage Flow
The persisted run pipeline is modeled in `src/session/workspace.rs` via `RunStage`.

1. `DiscoverInput`
2. `DiscoverOutput`
3. `Dedupe`
4. `FilterSize`
5. `ExtractText`
6. `BuildLlmClient`
7. `ExtractKeywords`
8. `SynthesizeCategories`
9. `InspectOutput`
10. `GeneratePlacements`
11. `BuildPlan`
12. `ExecutePlan`

`BuildLlmClient` and `ExecutePlan` are runtime-only stages. The other major stages persist JSON artifacts for resume and rerun flows.

## Persisted Session Workspace
Run state is stored under the XDG cache tree managed by `src/session/workspace.rs`:

- `<cache>/sortyourpapers/resume/<cwd-hash>/runs/<run-id>/manifest.json`: run metadata and last completed stage.
- `<cache>/sortyourpapers/resume/<cwd-hash>/runs/<run-id>/config.json`: resolved config used for the run.
- `<cache>/sortyourpapers/resume/<cwd-hash>/runs/<run-id>/report.json`: final run report when available.
- Stage artifacts such as `01-discover-input.json` through `10-build-plan.json`: persisted intermediate results used by resume and rerun commands.
- `<cache>/sortyourpapers/resume/<cwd-hash>/latest_run`: pointer to the latest run for the current workspace.

## Notes
- Taxonomy, placement, and filesystem planning now live under `src/papers/`. Older references to top-level `src/taxonomy/`, `src/placement/`, or `src/fs_*` directories are outdated.
- The original greenfield implementation plan lives in `docs/archive/initial-implementation-plan.md`.
