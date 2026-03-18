# SortYourPapers Rust CLI Plan

## Summary
Build a greenfield Rust CLI app that scans PDFs, extracts first-page text, uses LLM calls with strict JSON schemas, and organizes files into category folders.  
Repo state today is minimal (`README.md` only), so this plan defines full architecture, interfaces, and implementation decisions.

## Skills Used
No listed skill was used because this task is product/engineering planning, not skill creation or installation.

## Scope and Success Criteria
1. Accept CLI arguments with defaults and XDG config support.
2. Scan PDFs from an input folder, optionally recursive.
3. Filter PDFs by max size (default `8MB`): process files `<= max`, skip larger files.
4. Extract text from first `N` pages (default `5`).
5. Call LLM to extract keywords.
6. Call LLM again to build categories from keyword set.
7. If output folder is non-empty, place files using current tree map, with selectable placement mode.
8. If output folder is empty, generate folder/subfolder taxonomy with depth control.
9. Support manual rebuild mode that ignores existing folder names and reclassifies full corpus.
10. Run in dry-run by default; apply changes only with explicit flag.

## Public Interfaces and Types

### CLI (clap)
1. `--input <PATH>` default `.`  
2. `--output <PATH>` default `./sorted`  
3. `--recursive` default `false`  
4. `--max-file-size-mb <u64>` default `8`  
5. `--page-cutoff <u8>` default `1`  
6. `--category-depth <u8>` default `2`  
7. `--placement-mode <existing-only|allow-new>` default `existing-only`  
8. `--rebuild` default `false`  
9. `--dry-run` default `true`  
10. `--apply` sets `dry_run=false`  
11. `--llm-provider <openai|ollama>` required via CLI/config  
12. `--llm-model <STRING>` required via CLI/config  
13. `--llm-base-url <URL>` optional provider endpoint override  
14. `--api-key <STRING>` optional (for OpenAI-compatible)

### Config File (XDG)
Path: `$XDG_CONFIG_HOME/sortyourpapers/config.toml` (fallback `~/.config/sortyourpapers/config.toml`)  
Priority: `CLI > ENV > XDG config > defaults`

### ENV Keys
1. `SYP_INPUT`
2. `SYP_OUTPUT`
3. `SYP_RECURSIVE`
4. `SYP_MAX_FILE_SIZE_MB`
5. `SYP_PAGE_CUTOFF`
6. `SYP_CATEGORY_DEPTH`
7. `SYP_PLACEMENT_MODE`
8. `SYP_REBUILD`
9. `SYP_DRY_RUN`
10. `SYP_LLM_PROVIDER`
11. `SYP_LLM_MODEL`
12. `SYP_LLM_BASE_URL`
13. `SYP_API_KEY`

### Core Rust Types
1. `AppConfig` (resolved runtime config)
2. `PdfCandidate { path, size_bytes }`
3. `PaperText { file_id, path, extracted_text, pages_read }`
4. `KeywordSet { file_id, keywords: Vec<String> }`
5. `CategoryTree { name, children }`
6. `PlacementDecision { file_id, target_rel_path, rationale?, confidence? }`
7. `PlanAction { source, destination, action: Move }`
8. `RunReport { scanned, processed, skipped, failed, actions, dry_run }`

## LLM Contract (Strict JSON + Validation + Retry)
All LLM steps must parse into typed structs; invalid JSON triggers bounded retry with schema reminder.

1. Keyword extraction schema  
`{ "file_id": "...", "keywords": ["..."] }`

2. Category synthesis schema  
`{ "categories": [ { "name": "...", "children": [...] } ] }`  
Validator enforces max depth = `category_depth` for global taxonomy synthesis and for the final merge stage in `batch-merge` mode.

3. Placement schema  
`{ "placements": [ { "file_id": "...", "target_rel_path": "...", "confidence": 0.0 } ] }`  
Validator enforces folder constraints based on `placement_mode`.

## Processing Workflow

1. Resolve config from CLI/ENV/XDG/defaults.
2. Discover PDFs in `input` with recursive option.
3. Filter by max size (`<= max` processed).
4. Extract text from first `page_cutoff` pages using PDF text library.
5. Build per-file keyword sets via LLM call #1.
6. Build global category taxonomy via LLM call #2.
7. Detect output state.
8. If output empty:
1. Ask LLM for taxonomy constrained by `category_depth`.
2. Create folder plan.
3. Place files into generated taxonomy.
9. If output non-empty and not rebuild:
1. Build directory tree map from existing output.
2. Ask LLM to place each file into existing folders by default.
3. If `placement_mode=allow-new`, allow additional folder creation with depth constraint.
10. If rebuild mode:
1. Corpus = scanned PDFs + existing PDFs under output.
2. Ignore existing folder names.
3. Regenerate taxonomy and re-place full corpus.
11. Build move action plan.
12. Resolve filename conflicts with deterministic suffixes (`_1`, `_2`, ...).
13. Execute actions only when `--apply`; otherwise print dry-run plan.
14. Emit final report and non-zero exit on failures.

## Project Structure
1. `src/main.rs` CLI entrypoint and error boundary.
2. `src/config.rs` config loading and precedence merge.
3. `src/discovery.rs` PDF scanning and size filter.
4. `src/pdf_extract.rs` first-N-pages text extraction adapter.
5. `src/llm/mod.rs` provider trait and shared request/response models.
6. `src/llm/openai.rs` OpenAI-compatible adapter.
7. `src/llm/ollama.rs` Ollama adapter.
8. `src/categorize.rs` keyword aggregation and taxonomy validation.
9. `src/place.rs` folder map parsing and placement validation.
10. `src/planner.rs` move plan generation + conflict resolver.
11. `src/execute.rs` dry-run/apply engine.
12. `src/report.rs` terminal output summary.
13. `src/error.rs` typed errors with thiserror/anyhow boundary.
14. `tests/` integration tests.

## Dependencies
1. `clap`
2. `serde`, `serde_json`, `toml`
3. `directories` or `xdg`
4. `walkdir`
5. `tokio`, `reqwest`
6. `schemars` or manual schema validation
7. `thiserror`, `anyhow`
8. PDF extractor crate (pick one after benchmark spike: `lopdf` + extractor helper, or `pdf-extract`)

## Test Cases and Scenarios

1. Config precedence test: CLI overrides ENV/XDG/default.
2. Discovery test: recursive off/on behavior.
3. Size filter test: exactly at 8MB included; above excluded.
4. PDF extraction test: respects `page_cutoff`.
5. LLM schema validation test: malformed JSON retries then fails.
6. Empty output taxonomy generation test: folder depth never exceeds `category_depth`.
7. Non-empty output placement test: `existing-only` rejects unseen folders.
8. Placement mode override test: `allow-new` permits new folders.
9. Rebuild test: ignores existing names and reclassifies combined corpus.
10. Name conflict test: deterministic suffix behavior.
11. Dry-run test: no filesystem mutation.
12. Apply test: files moved correctly and report matches.
13. Failure behavior test: LLM/API failure stops run with non-zero exit.
14. Provider parity test: same typed contract works for OpenAI/Ollama adapters.

## Assumptions and Defaults Locked
1. Max-file rule means process files up to threshold, skip larger files.
2. Output action is move-based organization.
3. Each PDF gets exactly one destination folder.
4. Dry-run is default; `--apply` required for mutation.
5. Rebuild reclassifies full corpus (new scan + existing output PDFs).
6. Default placement when omitted is `existing-only`.
7. Placement mode is runtime-selectable per run.
8. Strict JSON schema is required for all LLM steps.
9. On LLM/network failure, fail fast and report.
10. Config precedence is `CLI > ENV > XDG > defaults`.
