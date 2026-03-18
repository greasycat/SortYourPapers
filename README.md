# SortYourPapers
Use LLMs to sort papers.

## What It Does
- Scans a folder for PDFs (optional recursive mode)
- Ignores files larger than a configurable limit (default `16MB`)
- Extracts text from first `N` pages (default `1`)
- Extracts file-keyword pairs plus a per-file preliminary `k`-depth category text in LLM batches (default `20` files per batch)
- Builds the global taxonomy from the aggregated preliminary category texts, batching those aggregated entries when needed, and returns a linear path array that is rebuilt into a tree
- Displays the merged taxonomy immediately after synthesis and, in interactive terminals, lets you iteratively suggest improvements until you accept it before placement generation begins
- Remaps papers to final destination folders in stable LLM batches (default `10` files per batch) using each file's keywords, preliminary category text, and the synthesized taxonomy
- Prints the final synthesized category tree at the end of a successful run
- Keeps `taxonomy-mode` for CLI/config compatibility, and uses `taxonomy-batch-size` to control batching of aggregated preliminary-category entries during taxonomy synthesis
- Saves completed taxonomy batches during synthesis so an interrupted run can resume without redoing finished batches
- Uses an LLM to:
  - extract keywords per paper
  - suggest a preliminary category text per paper
  - synthesize folder taxonomy from the aggregated preliminary texts
  - remap each PDF into one final destination folder
- Supports preview mode by default and real moves with `--apply`
- Supports rebuild mode to ignore existing folder names and reclassify all PDFs
- Persists each run under the XDG cache dir so interrupted runs can be resumed, including partial keyword/preliminary-category batches
- Shows `indicatif` progress bars by default for preprocessing, keyword batching, taxonomy synthesis, placement batching, and apply-mode moves
- Keeps warnings/errors and the final summary visible by default while suppressing most staging chatter unless `-v` or `-vv` is enabled

## Configuration Priority
`CLI > ENV > XDG config > defaults`

XDG config path:
- `$XDG_CONFIG_HOME/sortyourpapers/config.toml`
- fallback: `~/.config/sortyourpapers/config.toml`

## Quick Start
Create default XDG config:
```bash
cargo run -- init
```

Overwrite existing config:
```bash
cargo run -- init --force
```

Then run sorting:
```bash
cargo run -- \
  --input ./papers \
  --output ./sorted \
  --recursive \
  --llm-provider ollama \
  --llm-model llama3.1
```

If a run is interrupted after some stages completed, list saved runs and choose one to resume:
```bash
cargo run -- session resume
```

Resume a specific run id:
```bash
cargo run -- session resume run-123456789
```

List saved sessions without resuming:
```bash
cargo run -- session list
```

Remove a saved session:
```bash
cargo run -- session remove run-123456789
```

Clear all incomplete sessions for the current workspace:
```bash
cargo run -- session clear
```

Show verbose timing and resume diagnostics:
```bash
cargo run -- \
  --input ./papers \
  --output ./sorted \
  -v
```

Show full debug output including raw LLM requests:
```bash
cargo run -- \
  --input ./papers \
  --output ./sorted \
  -vv
```

Suppress progress bars and final summary:
```bash
cargo run -- \
  --input ./papers \
  --output ./sorted \
  --quiet
```

Apply real moves:
```bash
cargo run -- \
  --input ./papers \
  --output ./sorted \
  --llm-provider openai \
  --llm-model gpt-4o-mini \
  --api-key "$SYP_API_KEY" \
  --apply
```

Manual text extraction for debugging:
```bash
cargo run -- extract-text \
  --page-cutoff 2 \
  --pdf-extract-workers 4 \
  --extractor pdf-oxide \
  -vv \
  ./papers/sample.pdf
```

Compare `batch-merge` and `global` taxonomy timing on the same corpus:
```bash
cargo run --bin compare_taxonomy_modes -- \
  --input ./papers \
  --taxonomy-batch-size 3 \
  -v
```

## Core Flags
- `init [-f|--force]` create default XDG config file  
- `extract-text [--page-cutoff <u8>] [--extractor <auto|pdf-oxide|pdftotext>] [-v|-vv] <PDF...>` extract text directly  
- `session|ses resume [RUN_ID]` list saved sessions and prompt for a choice when `RUN_ID` is omitted, or resume a specific run id from the XDG cache state directory
- `session|ses review [RUN_ID]` display the synthesized category tree for a completed saved session
- `session|ses list|ls` print saved sessions for the current workspace
- `session|ses remove|rm [RUN_ID ...]` delete one or more saved sessions, or prompt for one when omitted interactively
- `session|ses clear|clr` delete incomplete saved sessions for the current workspace
- `-q, --quiet` suppress progress bars and final summary while still printing warnings/errors
- normal mode prints one concise numbered line per top-level stage, for example `[3/10] Filter oversized PDFs`
- `-v, --verbose` enable detailed stage diagnostics such as run/resume headlines and timings
- `-vv` enable full debug output, including raw LLM request payloads
- `-i, --input <PATH>` default `.`  
- `-o, --output <PATH>` default `./sorted`  
- `-r, --recursive` default `false`  
- `-s, --max-file-size-mb <u64>` default `16`  
- `-p, --page-cutoff <u8>` default `1`  
- `--pdf-extract-workers <usize>` default `8`
- `-d, --category-depth <u8>` default `2` (used for per-file preliminary category suggestions and the final synthesized taxonomy)  
- `--taxonomy-mode <global|batch-merge>` default `batch-merge` (both values use the same preliminary-category batching + final merge flow)
- `--taxonomy-batch-size <usize>` default `4` (aggregated preliminary-category entries per taxonomy batch before the final merge request)
- `--placement-batch-size <usize>` default `10` (papers per placement request)
- `-M, --placement-mode <existing-only|allow-new>` default `existing-only`  
- `-R, --rebuild` default `false`  
- `-a, --apply` move files instead of running in preview mode  
- `-P, --llm-provider <openai|ollama|gemini>` default `gemini`  
- `-m, --llm-model <STRING>` default `gemini-3-flash-preview`  
- `-u, --llm-base-url <URL>` optional  
- `-k, --api-key <STRING>` optional
- `--keyword-batch-size <usize>` default `20` (keyword extraction batch size)

## Resume State
Each normal sorting run creates a state directory under `$XDG_CACHE_HOME/sortyourpapers/resume/<cwd-hash>/runs/<run-id>`.
Fallback when `XDG_CACHE_HOME` is unset: `~/.cache/sortyourpapers/resume/<cwd-hash>/runs/<run-id>`.
Completed stage outputs are written as JSON files, and `latest_run` is stored alongside that workspace’s `runs/` directory.
Interrupted keyword extraction also saves partial batch progress so resume can skip already completed `(file_id, keywords, preliminary_categories_k_depth)` work.
Interrupted taxonomy synthesis also saves completed preliminary-category batches so resume can skip them and continue from the remaining batch plus final merge.
Interrupted placement generation also saves completed placement batches so resume can skip them, preserving the same per-run batch membership.
If a run resumes after taxonomy synthesis but before placement generation, the merged taxonomy is reloaded from `07-synthesize-categories.json` and the inspect stage can re-render it once before placements continue.
`session resume` without a `RUN_ID` prints all saved sessions and asks which one to continue.
`session resume <RUN_ID>` reloads the saved config and continues from the first missing stage instead of repeating earlier LLM calls.
`session review` prints the synthesized category tree for a completed saved session.
`session list` prints saved sessions without resuming.
`session remove` deletes specific saved sessions, and `session clear` removes incomplete ones for the current workspace.
Use `session resume --quiet` if you only want the exit status without the progress stream or final summary.

## Environment Variables
- `SYP_INPUT`
- `SYP_OUTPUT`
- `SYP_RECURSIVE`
- `SYP_MAX_FILE_SIZE_MB`
- `SYP_PAGE_CUTOFF`
- `SYP_PDF_EXTRACT_WORKERS`
- `SYP_CATEGORY_DEPTH`
- `SYP_TAXONOMY_MODE`
- `SYP_TAXONOMY_BATCH_SIZE`
- `SYP_PLACEMENT_BATCH_SIZE`
- `SYP_PLACEMENT_MODE`
- `SYP_REBUILD`
- `SYP_BATCH_START_DELAY_MS`
- `SYP_LLM_PROVIDER`
- `SYP_LLM_MODEL`
- `SYP_LLM_BASE_URL`
- `SYP_API_KEY`
- `SYP_KEYWORD_BATCH_SIZE`
