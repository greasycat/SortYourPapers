# SortYourPapers Architecture

This document is a repo-structure reference for the current codebase. It reflects the source tree as it exists now, not the original implementation plan.

## Repository Layout
- `assets/papers/`: sample PDFs used for local runs and manual testing.
- `assets/testsets/`: committed test-set manifests for fetched paper corpora.
- `crates/paper-db/`: DuckDB-backed paper and embedding storage crate.
- `python/`: `uv`-managed maintainer tooling for SciJudgeBench sampling and arXiv PDF materialization.
- `crates/syp-core/`: shared library crate.
- `crates/syp/`: batch CLI crate for the `syp` binary.
- `crates/syptui/`: TUI crate for the `syptui` binary.
- `docs/`: project documentation.
- `docs/archive/`: historical planning material kept for reference.
- `target/`: Cargo build output.

## Binary Entrypoints
- `crates/syp/src/main.rs`: `syp` binary. Parses the standard clap CLI and dispatches into `syp-core`.
- `crates/syptui/src/main.rs`: `syptui` binary. Starts the terminal UI directly.
- `crates/syp-core/src/lib.rs`: shared module wiring and public API used by both frontends.

## Top-Level Source Modules
- `crates/syp/src/cli.rs`: clap argument types for the batch CLI, including run arguments, session commands, and `extract-text`.
- `crates/syp/src/entrypoints.rs`: CLI dispatch and top-level error hint printing.
- `crates/paper-db/src/lib.rs`: DuckDB schema bootstrap, paper upserts, and embedding-sync APIs.
- `python/src/syp_paperfetch/`: SciJudgeBench catalog loading through Hugging Face Hub, deterministic sampling, manifest I/O, and arXiv PDF materialization.
- `crates/syp-core/src/error.rs`: application error types and shared result aliases.
- `crates/syp-core/src/report.rs`: final run report structures and file action summaries.
- `crates/syp-core/src/app/`: orchestration for a full sorting run, including config resolution handoff, debug-TUI seeded runs, and report rendering.
- `crates/syp-core/src/config/`: config loading and precedence resolution across overrides, environment variables, XDG config, and defaults.
- `crates/syp-core/src/llm/`: provider-specific clients plus shared batching, retry, and schema logic.
- `crates/syp-core/src/papers/`: the paper-processing pipeline, including discovery, extraction, preprocessing, taxonomy synthesis, placement, and filesystem planning.
- `crates/syp-core/src/session/`: persisted run state, resume/rerun/review commands, and workspace artifact management.
- `crates/syp-core/src/terminal/`: plain terminal output backend, verbosity handling, and report printing helpers.
- `crates/syptui/src/tui/`: `ratatui` frontend, forms, backend event handling, and session/run screens.
- `crates/syptui/src/prefs.rs`: TUI-only theme preference persistence.

## Papers Pipeline Layout
The core workflow is grouped under `crates/syp-core/src/papers/` rather than split across top-level modules.

- `crates/syp-core/src/papers/discovery.rs`: scans input and output trees for candidate PDFs.
- `crates/syp-core/src/papers/extract.rs`: PDF text extraction and extraction batching helpers.
- `crates/syp-core/src/papers/preprocess.rs` and `crates/syp-core/src/papers/preprocess/`: text cleanup and term preparation before LLM calls.
- `crates/syp-core/src/papers/taxonomy/`: keyword extraction, preliminary category generation, taxonomy synthesis, batching, prompts, and validation.
- `crates/syp-core/src/papers/placement/`: output inspection, placement prompts, placement batching/runtime logic, and validation.
- `crates/syp-core/src/papers/fs_ops/`: plan construction and filesystem execution for preview/apply mode.
- `crates/syp-core/src/papers/mod.rs`: shared pipeline data types such as `PdfCandidate`, `PaperText`, keyword state, and synthesized category state.

## Test-Set Fetching Layout
- `python/src/syp_paperfetch/catalog.py`: Hugging Face Hub dataset download and SciJudgeBench pair flattening.
- `python/src/syp_paperfetch/curate.py`: top/bottom/random citation sampling per category with subcategory caps.
- `python/src/syp_paperfetch/manifest.py`: TOML manifest load/save helpers.
- `python/src/syp_paperfetch/materialize.py`: arXiv PDF download, cache verification, and export helpers.
- `python/src/syp_paperfetch/cli.py`: `uv`-run CLI entrypoints for build/materialize/export.

## TUI Layout
- `crates/syptui/src/tui/app.rs`: application state and key-driven behavior.
- `crates/syptui/src/tui/render.rs`: screen rendering.
- `crates/syptui/src/tui/forms/`: run configuration forms for interactive launches.
- `crates/syptui/src/tui/session_view.rs`: saved-session browsing and review views.
- `crates/syptui/src/tui/backend.rs` and `crates/syptui/src/tui/input.rs`: backend event integration and input handling.

## Run Stage Flow
The persisted run pipeline is modeled in `crates/syp-core/src/session/workspace.rs` via `RunStage`.

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
Run state is stored under the XDG cache tree managed by `crates/syp-core/src/session/workspace.rs`:

- `<cache>/sortyourpapers/resume/<cwd-hash>/runs/<run-id>/manifest.json`: run metadata and last completed stage.
- `<cache>/sortyourpapers/resume/<cwd-hash>/runs/<run-id>/config.json`: resolved config used for the run.
- `<cache>/sortyourpapers/resume/<cwd-hash>/runs/<run-id>/report.json`: final run report when available.
- Stage artifacts such as `01-discover-input.json` through `10-build-plan.json`: persisted intermediate results used by resume and rerun commands.
- `<cache>/sortyourpapers/resume/<cwd-hash>/latest_run`: pointer to the latest run for the current workspace.

## Notes
- Taxonomy, placement, and filesystem planning now live under `crates/syp-core/src/papers/`. Older references to top-level `src/taxonomy/`, `src/placement/`, or `src/fs_*` directories are outdated.
- The original greenfield implementation plan lives in `docs/archive/initial-implementation-plan.md`.
