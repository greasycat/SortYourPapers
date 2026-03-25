# Tomorrow

## Goal
Refactor the project from a strong v1 PDF sorter into a scalable library-centered paper management application where LLM and embedding services are reusable platform capabilities rather than pipeline-local helpers.

The immediate objective is not a user-visible workflow redesign. The immediate objective is to improve module decoupling, reduce oversized files, clean up naming, and create architecture boundaries that can support future capabilities such as metadata enrichment, retrieval, semantic search, recommendations, and additional organization workflows.

## Current Structural Pressure

### Oversized files
The current codebase has several large files that are carrying multiple responsibilities at once.

- `crates/syptui/src/tui/mod.rs`: `2543` lines
- `crates/syptui/src/tui/forms/run_form.rs`: `1298` lines
- `crates/syptui/src/tui/render.rs`: `1252` lines
- `crates/syp-core/src/session/workspace.rs`: `1179` lines
- `crates/syp-core/src/papers/placement/runtime.rs`: `1102` lines
- `crates/paper-db/src/lib.rs`: `1000` lines
- `crates/syp-core/src/session/commands.rs`: `845` lines
- `crates/syp-core/src/app/mod.rs`: `752` lines

Tests are also large in a few places, but the highest maintainability risk is concentrated in production modules.

### Coupling hotspots
The codebase currently mixes domain logic, orchestration, persistence, provider integrations, and frontend concerns more than is healthy for the next stage of growth.

- `syp-core` currently acts as domain library, app service layer, workflow engine, session persistence layer, and compatibility facade at the same time.
- `papers` is still the dominant module boundary, but the project direction is broader than a one-shot paper sorting pipeline.
- `session` currently owns both storage details and workflow behavior, which makes run-state evolution harder.
- `llm` is provider-focused, but the long-term product needs a broader AI service boundary that includes embeddings, retrieval evidence, and shared prompt contracts.
- `syptui` still has several god-module patterns where bootstrap, rendering, interaction logic, and view-specific helpers are collapsed together.

## Target Direction

### Product language
New architecture should move toward library or collection language instead of sorter-only language.

Use these concepts going forward:

- `library`
- `collection`
- `document`
- `organization`
- `retrieval`
- `workflow`
- `ai`

Keep academically specific names where they are actually specific to the feature, such as taxonomy, arXiv, citations, or curated test sets. Do not force renames where the concept is genuinely paper-specific.

### Crate strategy
Keep the current frontends and storage crate, but add domain crates over time.

Keep:

- `syp`: CLI frontend
- `syptui`: TUI frontend
- `paper-db`: storage and indexing crate

Add:

- `syp-ai`: provider-agnostic LLM and embedding interfaces, provider implementations, retry, batching, and prompt contract helpers
- `syp-library`: library-domain entities and services for documents, organization inputs and outputs, retrieval evidence, metadata, and future semantic capabilities
- `syp-workflow`: run execution, stage graph, session persistence, resume, rerun, review, and report assembly

`syp-core` should become a temporary compatibility facade that re-exports public entrypoints used by `syp` and `syptui`. It should stop accumulating long-lived business logic.

## Refactor Priorities

### Priority 1: Split current monolith files before moving crates
Do not start with broad crate churn. First create clean module seams inside the existing crates.

#### `crates/syp-core/src/app/mod.rs`
Split into:

- `run`
- `extract_text`
- `reference_index`
- `debug_seed`
- `path_resolution`

`app/mod.rs` should become a thin public entrypoint layer only.

#### `crates/syp-core/src/session/workspace.rs`
Split into:

- `types`
- `paths`
- `store`
- `queries`
- `cleanup`

This module currently holds too many concerns:

- stage definitions
- persisted structs
- path derivation
- JSON I/O
- latest-run bookkeeping
- run discovery
- removal logic

These should be separated before any crate extraction.

#### `crates/syp-core/src/session/commands.rs`
Split into:

- `resume`
- `rerun`
- `review`
- `list`
- `impact`

The public command surface should become thin wrappers over reusable workflow services.

#### `crates/syp-core/src/papers/placement/runtime.rs`
Split into:

- `engine`
- `embedding`
- `scoring`
- `llm_tiebreak`
- `progress`

The new embedding-primary placement work made the need here obvious. Deterministic scoring, evidence preparation, and LLM fallback should not live in one file.

#### `syptui`
Break the major TUI files into real submodules.

For `crates/syptui/src/tui/mod.rs`:

- keep only bootstrap and run loop logic

For `crates/syptui/src/tui/render.rs`:

- `header`
- `home`
- `operation`
- `overlay`
- `review`
- `layout`

For `crates/syptui/src/tui/forms/run_form.rs`:

- `state`
- `validation`
- `config_build`
- `draw`
- `directory_query`

## Crate Extraction Sequence

### Phase 2: Introduce stable domain crates without behavior change
After the large files are split into coherent modules, extract crates in this order.

#### 1. `syp-ai`
Move out of `syp-core`:

- `llm/`
- embedding client support
- provider implementations
- retry and batching helpers
- provider-neutral AI service traits

This crate should own:

- chat interfaces
- embedding interfaces
- provider config types
- prompt execution helpers

It should not own session persistence or library-domain orchestration.

#### 2. `syp-library`
Move out of `syp-core`:

- shared document and organization data structures
- taxonomy inputs and outputs
- placement inputs and outputs
- retrieval evidence structures
- future metadata and recommendation services

This crate should become the main domain home for:

- documents
- organization plans
- semantic evidence
- feature extraction outputs

It should not depend on CLI or TUI code.

#### 3. `syp-workflow`
Move out of `syp-core`:

- session persistence
- run-stage execution
- resume and rerun logic
- report assembly
- stage graph semantics

This crate should own the workflow engine, while `syp` and `syptui` become thin callers.

### Phase 3: Reduce `syp-core` to facade status
Once the three domains above exist, `syp-core` should:

- re-export stable public APIs
- contain minimal compatibility glue
- avoid becoming a new catch-all module

If a new feature cannot clearly belong to `syp-ai`, `syp-library`, `syp-workflow`, `paper-db`, `syp`, or `syptui`, that is a design smell and should be resolved before implementation.

## Naming Strategy

### Keep stable for now
Keep these names in the first refactor pass:

- CLI flags
- persisted run stage filenames
- session artifact filenames
- current user-visible command structure

This avoids breaking resume and workflow compatibility while the architecture is moving.

### Move gradually
Use broader application language in new code and new modules.

Preferred naming:

- `papers` -> `library` over time
- `session runtime` -> `workflow runtime`
- `placement` -> keep for now internally, but future service naming can move toward `routing` or `organization placement`
- `taxonomy` -> keep where it genuinely describes folder/category hierarchy generation

Do not mass-rename internal namespaces just for aesthetics. Rename only when the new boundary is already being extracted.

## File Size and Structure Guardrails

Apply these guardrails during refactors:

- target max `600` LOC per non-test production Rust file in `syp-core`
- target max `800` LOC per non-test production Rust file in `syptui`
- do not add new behavior to already-oversized production files unless the same change also extracts code out of them
- tests may remain larger temporarily, but new tests should prefer smaller focused modules where practical

## Architectural Rules Going Forward

### Dependency direction
- `syp` and `syptui` depend on workflow and domain services
- `syp-workflow` depends on `syp-library`, `syp-ai`, and `paper-db`
- `syp-library` may depend on `syp-ai` abstractions if needed, but avoid direct provider coupling
- `paper-db` remains infrastructure storage and should not depend on CLI, TUI, or workflow modules

### Frontend boundaries
- CLI and TUI should not reach directly into provider-specific modules
- CLI and TUI should not own domain decision logic
- frontend code should depend on service interfaces and stable workflow entrypoints

### Persistence boundaries
- session artifact schemas should be owned by workflow modules, not scattered through app entrypoints
- storage schemas for embeddings and reference indices remain owned by `paper-db`
- compatibility-sensitive persisted file names stay stable until a deliberate migration plan exists

## Acceptance Criteria
This refactor direction is successful when all of the following are true.

- `syp-core` stops growing as the default destination for unrelated logic
- the large production files listed above are split below the line-count guardrails
- `syp` and `syptui` call workflow services instead of orchestration internals
- AI/provider code is isolated behind a clear crate or service boundary
- library-domain types are reusable for future features beyond sorting
- naming in new code follows library or collection language where appropriate
- current CLI behavior, TUI behavior, and persisted run compatibility remain intact during the first pass

## Verification Plan
Keep these checks as the baseline during each refactor slice.

- `cargo fmt --all`
- `cargo test -p paper-db`
- `cargo test -p syp-core`
- `cargo test -p syp`
- `cargo test -p syptui`

Add focused architectural tests as the refactor progresses:

- workflow tests for stage sequencing and rerun impact
- AI adapter tests against provider-neutral traits
- library service tests for organization inputs, retrieval evidence, and placement scoring
- serialized artifact tests to prevent accidental resume-format drift

## First Concrete Work Items
If implementation starts tomorrow, the first sequence should be:

1. Split `session/workspace.rs`
2. Split `papers/placement/runtime.rs`
3. Split `app/mod.rs`
4. Split `syptui/src/tui/mod.rs`
5. Split `syptui/src/tui/render.rs`
6. Split `syptui/src/tui/forms/run_form.rs`
7. Extract `syp-ai`
8. Extract `syp-library`
9. Extract `syp-workflow`
10. Reduce `syp-core` to a facade

This order is deliberate. It creates internal seams first, then crate seams, then naming cleanup.
