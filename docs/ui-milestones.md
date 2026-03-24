# SortYourPapers UI Milestones

This document proposes a practical milestone plan for the terminal UI based on the current `crates/syptui/src/tui/` design. It is not a greenfield redesign. The goal is to improve the existing workflow-first TUI without fighting the current architecture.

## Current TUI Baseline

The current TUI already has a solid structure:

- A `Home` screen with three top-level actions: run papers, inspect sessions, and quit.
- A `RunForm` screen with a three-column layout covering paths, extraction, taxonomy, placement, provider settings, and run toggles.
- A `Sessions` screen for resume, rerun, review, delete, and clear operations.
- An `Operation` screen that combines status, progress gauges, details, and recent logs.
- Overlay flows for field editing, rerun-stage selection, confirmation dialogs, taxonomy inspection prompts, and continue-improving prompts.

This means the UI is already workflow-oriented rather than dashboard-oriented. That is the right direction for a terminal app like this.

## What The Current Design Does Well

- It mirrors the real application flow instead of inventing a separate UI-only model.
- It keeps long-running work visible through progress gauges and log streaming.
- It exposes session recovery as a first-class feature instead of burying resume/rerun in CLI-only commands.
- The run form is dense but coherent: related controls are grouped and keyboard navigation is predictable.
- The taxonomy review loop is already integrated into the UI model through overlays instead of being treated as a separate tool.

## Main Gaps In The Current Design

These are the highest-value issues suggested by the current implementation.

- The run form exposes raw config keys. It is accurate, but it reads like a config editor rather than an operator-focused workflow.
- Validation is mostly deferred until run start. The UI should catch obvious mistakes before the user launches work.
- The operation view is informative but passive. Logs, details, and progress are visible, but not very navigable.
- The sessions screen is functional but not decision-friendly. It lists raw metadata instead of helping the user quickly answer "which run should I resume or inspect?"
- The taxonomy improvement overlays are usable, but still feel like prompt plumbing rather than a dedicated review experience.
- The app has the beginnings of extract-preview support, but that capability is not yet surfaced as a first-class screen in the TUI.
- Visual state semantics need tightening. Success, idle, failure, and busy states should be easier to distinguish at a glance.

## Design Direction

The next UI iterations should preserve these principles:

- Keep the TUI keyboard-first.
- Keep screens task-oriented, not menu-heavy.
- Prefer exposing application concepts like run, session, taxonomy review, and preview/apply over exposing internal config names.
- Reuse the existing screen-and-overlay structure instead of introducing a complicated navigation framework.
- Treat the CLI as the source of truth for behavior, while the TUI becomes the fastest way to configure and inspect runs.

## Milestone 1: Make The Existing Screens Easier To Use

Goal: improve clarity and trust without changing the overall information architecture.

Suggested scope:

- Replace raw field labels with user-facing labels while preserving the same underlying config mapping.
- Add short helper text or contextual hints for selected fields, especially provider, placement mode, taxonomy mode, and apply/rebuild.
- Add inline validation feedback for numeric fields and obvious path mistakes before the user presses run.
- Show a compact run summary panel or "resolved plan" preview before launch.
- Tighten status colors and labels so failure does not visually resemble idle or success.
- Make the sessions list easier to scan with human-readable timestamps and more compact summaries.

Why this milestone first:

- It builds directly on the current form/session/operation screens.
- It reduces user error without requiring major state-model changes.
- It makes the existing TUI feel intentional instead of merely functional.

## Milestone 2: Improve Operation Visibility

Goal: make active and completed runs easier to inspect during long operations.

Suggested scope:

- Add scroll support for logs and detail panes.
- Split the operation area into lightweight modes or tabs such as summary, logs, taxonomy, and report.
- Preserve important warnings and failures in a pinned section instead of letting them scroll away.
- Show clearer stage-level progress text, including which batch or stage is currently active.
- Add explicit success/failure completion cards with next actions such as review taxonomy, view report, or return to sessions.

Why this milestone fits the current design:

- The app already collects logs, report data, category trees, and progress entries.
- The existing `Operation` screen is the natural place to make this richer without adding a new workflow.

## Milestone 3: Turn Sessions Into A Real Recovery Workspace

Goal: make `Sessions` the control center for interrupted or historical runs.

Suggested scope:

- Add filtering for latest, completed, incomplete, and failed-looking runs.
- Show more useful run metadata in the details panel, such as whether report data exists and what stage artifacts are available.
- Add a preview of the saved taxonomy or report summary directly from the sessions screen.
- Make rerun stage selection more explanatory by showing the practical consequence of choosing each stage.
- Add a safe refresh model after resume/rerun/delete actions so the list always reflects the current state.

Why this matters:

- Resume and rerun are central product features.
- The underlying session workspace model is already strong; the UI should capitalize on it.

## Milestone 4: Build A Dedicated Taxonomy Review Experience

Goal: move taxonomy review from prompt overlay mechanics to a clearer guided workflow.

Suggested scope:

- Present the category tree and the edit prompt in a more balanced split view.
- Support a visible history of improvement iterations during the same review cycle.
- Show the current suggestion text separately from the accepted taxonomy so the user can reason about changes.
- Add explicit actions like accept, suggest change, continue iterating, and cancel review.
- Make the review flow feel like a product feature instead of a blocking prompt.

Why this is a good fit:

- The app already emits inspect-review and continue-improving prompts.
- The current overlay model proves the workflow exists; it just needs a stronger UI surface.

## Milestone 5: Expand TUI Workflow Coverage

Goal: surface related tooling that already exists in the codebase.

Suggested scope:

- Add an `Extract Text` workflow in the TUI using the existing extract-preview support.
- Add a lightweight config-init or config-diagnostics screen.
- Add provider-specific guidance when a selected LLM backend needs `api_key` or `llm_base_url`.
- Add an optional debug-oriented screen when `--debug-tui` is enabled.

Why this is valuable:

- It extends the TUI from "run sorter" to "operate the tool".
- The extract-preview code already exists, so this is more integration work than invention.

## Milestone 6: Polish And Systematize The UI

Goal: make the TUI feel cohesive as a product rather than a set of connected screens.

Suggested scope:

- Standardize panel titles, action wording, and state labels across screens.
- Define a small visual system for status colors, selected rows, warnings, destructive actions, and success states.
- Improve small-screen behavior for narrower terminals.
- Extract repeated panel and list patterns into reusable TUI helpers.
- Expand rendering tests around layout stability and key workflows.

## Concrete Suggestions For The Current Design

If only a few improvements are implemented soon, these are the best candidates:

1. Rename run-form labels from config keys to user-facing labels.
2. Add inline validation and a pre-run summary.
3. Make the operation view scrollable and split report/tree/log content more clearly.
4. Improve session readability with humanized timestamps and more useful state summaries.
5. Promote taxonomy review from a simple prompt overlay to a dedicated guided screen.
6. Expose the existing extract-preview capability through the TUI.

## Suggested Implementation Order

1. Milestone 1, because it improves daily usability with low architectural risk.
2. Milestone 2, because long-running visibility is the most important runtime UX.
3. Milestone 3, because resume and rerun are already core product behavior.
4. Milestone 4, because taxonomy review is a distinctive workflow worth polishing.
5. Milestone 5, because it broadens the TUI once the main flows feel solid.
6. Milestone 6, as the consolidation pass after the bigger workflow improvements land.
