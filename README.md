# SortYourPapers
Use LLMs to sort papers.

## What It Does
- Scans a folder for PDFs (optional recursive mode)
- Ignores files larger than a configurable limit (default `8MB`)
- Extracts text from first `N` pages (default `5`)
- Extracts file-keyword pairs in LLM batches (default `50` files per batch)
- Uses an LLM to:
  - extract keywords per paper
  - synthesize folder taxonomy
  - place each PDF into one destination folder
- Supports dry-run by default and real moves with `--apply`
- Supports rebuild mode to ignore existing folder names and reclassify all PDFs

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

Show stage timing and extractor timing:
```bash
cargo run -- \
  --input ./papers \
  --output ./sorted \
  --debug
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
  --extractor lopdf \
  --debug \
  ./papers/sample.pdf
```

## Core Flags
- `init [-f|--force]` create default XDG config file  
- `extract-text [--page-cutoff <u8>] [--extractor <auto|lopdf|pdftotext>] [--debug] <PDF...>` extract text directly  
- `-i, --input <PATH>` default `.`  
- `-o, --output <PATH>` default `./sorted`  
- `-r, --recursive` default `false`  
- `-s, --max-file-size-mb <u64>` default `8`  
- `-p, --page-cutoff <u8>` default `5`  
- `-d, --category-depth <u8>` default `2`  
- `-M, --placement-mode <existing-only|allow-new>` default `existing-only`  
- `-R, --rebuild` default `false`  
- `-n, --dry-run` default `true`  
- `-a, --apply` sets `dry_run=false`  
- `-P, --llm-provider <openai|ollama|gemini>` default `gemini`  
- `-m, --llm-model <STRING>` default `gemini-2.5-flash`  
- `-u, --llm-base-url <URL>` optional  
- `-k, --api-key <STRING>` optional
- `--keyword-batch-size <usize>` default `50` (keyword extraction batch size)
- `--debug` print stage timing and per-file extraction timing

## Environment Variables
- `SYP_INPUT`
- `SYP_OUTPUT`
- `SYP_RECURSIVE`
- `SYP_MAX_FILE_SIZE_MB`
- `SYP_PAGE_CUTOFF`
- `SYP_CATEGORY_DEPTH`
- `SYP_PLACEMENT_MODE`
- `SYP_REBUILD`
- `SYP_DRY_RUN`
- `SYP_LLM_PROVIDER`
- `SYP_LLM_MODEL`
- `SYP_LLM_BASE_URL`
- `SYP_API_KEY`
- `SYP_KEYWORD_BATCH_SIZE`
