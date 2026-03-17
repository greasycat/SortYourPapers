# SortYourPapers
Use LLMs to sort papers.

## What It Does
- Scans a folder for PDFs (optional recursive mode)
- Ignores files larger than a configurable limit (default `16MB`)
- Extracts text from first `N` pages (default `1`)
- Extracts file-keyword pairs in LLM batches (default `20` files per batch)
- Assigns papers to destination folders in LLM batches (default `10` files per batch)
- Supports taxonomy synthesis in either one global pass or `batch-merge` mode (`4` papers per batch by default, followed by one final merge request). `batch-merge` is the default.
- Uses an LLM to:
  - extract keywords per paper
  - synthesize folder taxonomy
  - place each PDF into one destination folder
- Supports preview mode by default and real moves with `--apply`
- Supports rebuild mode to ignore existing folder names and reclassify all PDFs
- Persists each run under the XDG cache dir so interrupted runs can be resumed
- Prints stage progress by default for preprocessing, keyword batching, taxonomy batching, and merge steps

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
cargo run -- resume
```

Resume a specific run id:
```bash
cargo run -- resume run-123456789
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

Suppress stage output and final summary:
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
- `resume [RUN_ID]` list saved runs and prompt for a choice when `RUN_ID` is omitted, or resume a specific run id from the XDG cache state directory  
- `-q, --quiet` suppress stage progress output and final summary
- `-v, --verbose` enable verbose diagnostics such as timings and resume/extraction details
- `-vv` enable debug output, including full LLM request payloads
- `-i, --input <PATH>` default `.`  
- `-o, --output <PATH>` default `./sorted`  
- `-r, --recursive` default `false`  
- `-s, --max-file-size-mb <u64>` default `16`  
- `-p, --page-cutoff <u8>` default `1`  
- `--pdf-extract-workers <usize>` default `8`
- `-d, --category-depth <u8>` default `2`  
- `--taxonomy-mode <global|batch-merge>` default `batch-merge`
- `--taxonomy-batch-size <usize>` default `4` (papers per partial taxonomy batch in `batch-merge` mode)
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
`resume` without a `RUN_ID` prints all saved runs and asks which one to continue.
`resume <RUN_ID>` reloads the saved config and continues from the first missing stage instead of repeating earlier LLM calls.
Use `resume --quiet` if you only want the exit status without the stage stream.

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
