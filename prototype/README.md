# Python Prototype

A Python prototype of the front half of the pipeline: read a folder of PDFs,
label each one with an LLM, and keep doing it as new papers arrive.

This is deliberately **not** the whole product. It covers ingest and the
watcher. Taxonomy synthesis, placement, and moving files stay in the Rust
crates.

## What it does

`ingest` walks the input folder, skips anything already known or too large,
pulls text from the first page of each PDF, and sends batches of them to the
model for keywords and a preliminary category. Batches run four at a time,
matching the Rust pipeline's ceiling, because the stage is bound by round-trips
rather than tokens.

`watch` does that continuously. Filesystem events are only a wake-up hint — the
folder scan is the source of truth, so the loop stays correct when events are
coalesced, missed, or caused by its own writes. A folder must stop changing for
three seconds before a pass starts, so a PDF still being copied in is never read
half-written.

Results append to `ingest.jsonl` in the library folder, one line per paper,
flushed as each is recorded. That file is also the "already seen" marker: papers
are keyed by a hash of their contents, so re-running costs nothing and a renamed
paper is not paid for twice.

## Usage

```bash
uv run --project prototype sypy ingest --input ./inbox
uv run --project prototype sypy watch --input ./inbox
uv run --project prototype --extra dev python -m pytest prototype/tests
```

Without `--input` the current directory is watched, and the library defaults to
`sorted` inside it.

## Configuration

Settings resolve CLI > environment > defaults, reusing the Rust pipeline's
`SYP_*` names so a folder set up for one reads the same to the other:
`SYP_INPUT`, `SYP_OUTPUT`, `SYP_RECURSIVE`, `SYP_MAX_FILE_SIZE_MB`,
`SYP_PAGE_CUTOFF`, `SYP_KEYWORD_BATCH_SIZE`, `SYP_LLM_MODEL`.

The API key is read from `OPENAI_API_KEY`, `SYP_API_KEY`, or `OEPNAI_API_KEY`,
including from the repository-root `.env`. The third spelling is a typo this
repo's `.env` currently carries; it is accepted so the prototype works as-is,
and the correct spelling wins when both are set.

## Known gaps

- Scanned PDFs with no text layer are reported as failures. The Rust pipeline
  renders their pages and asks a vision model; this does not.
- Only OpenAI is wired up, because that is the key the repo carries.
- No resumable run state beyond the ingest index: an interrupted pass keeps the
  papers it finished and redoes the batch it was in the middle of.
