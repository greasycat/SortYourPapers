# Python Prototype

A Python prototype of the front half of the pipeline: read a folder of PDFs,
label each one with an LLM, and keep doing it as new documents arrive.

Documents are not assumed to be academic papers. Bills, receipts, and manuals
are categorized on their own terms.

This is deliberately **not** the whole product. It covers ingest and the
watcher. Taxonomy synthesis, placement, and moving files stay in the Rust
crates.

## What it does

`ingest` walks the input folder, skips anything already known or too large,
pulls text from the first page of each PDF, and sends batches of them to the
model for keywords, a category, and the title, authors, and year the links are
named by — all in one request per batch. Batches run four at a time,
matching the Rust pipeline's ceiling, because the stage is bound by round-trips
rather than tokens.

`watch` does that continuously. Filesystem events are only a wake-up hint — the
folder scan is the source of truth, so the loop stays correct when events are
coalesced, missed, or caused by its own writes. A folder must stop changing for
three seconds before a pass starts, so a PDF still being copied in is never read
half-written.

## How the library is laid out

Every PDF lives in exactly one place — a single flat folder — and the browsable
folder tree is made only of symlinks over it:

```
library/
  papers.duckdb
  store/
    5112ee75ddcf__Machine Learning__Deep Learning__Transformers.pdf
  tree/
    Machine Learning/Deep Learning/Transformers/
      vaswani_2017_attention-is-all-you-need.pdf -> ../../../../store/5112ee75ddcf__...pdf
```

The store filename is `<id>__<Tag>__<Tag>.pdf`. The id is permanent and the tags
are not, so **re-tagging a paper is a rename plus a moved link** — no file is
ever copied and nothing has to be found again. Tags are sanitized so no tag can
contain `__`, which keeps the name unambiguous to parse.

Links are named `author_year_title`, cleaned for filesystem safety, falling back
to the store name when the model could not find all three. They are relative, so
the whole library can be moved without breaking.

**DuckDB is the source of truth.** Filenames and the tree are projections of it,
which is why `sypy tree` can throw the tree away and rebuild it exactly, and why
a tag list too long for a filename loses nothing. Papers are keyed by a hash of
their contents, so re-running costs nothing and the same paper arriving twice
under different names is recognised.

Expanding the schema means appending to `_MIGRATIONS` in `db.py`; anything not
worth a column yet goes in `paper_attributes` as a key/value pair.

## Usage

Put `sypy` on PATH, then use it from anywhere:

```bash
./prototype/scripts/sypy-path wire     # install and link

sypy ingest --input ./inbox                 # preview: nothing is written
sypy ingest --input ./inbox --mode copy     # copy in, leave the source alone
sypy ingest --input ./inbox --mode move     # move in, draining the source
sypy watch  --input ./inbox --mode copy     # keep doing it as documents arrive

sypy list                              # what the library holds
sypy retag <id> "Systems/Databases"    # re-tag: renames the file, moves the link
sypy tree                              # rebuild the symlink tree from the database

./prototype/scripts/sypy-path unwire   # remove the link
```

Nothing is written without `--mode`. Use `copy` for a folder you did not create
— a Downloads folder keeps its files and the library gets copies. Re-run `wire`
after changing dependencies.

## Running it as a service

```bash
./prototype/scripts/sypy-service install ~/Downloads ~/Documents/sypy-library
./prototype/scripts/sypy-service status
./prototype/scripts/sypy-service logs
./prototype/scripts/sypy-service uninstall
```

Installs a launchd agent that watches in copy mode, so the watched folder is
indexed but never rearranged. `SYPY_MODE=move` overrides that. Logs go to
`~/Library/Logs/sypy/sypy.log`.

The watcher drops its database lock whenever it is idle, so `sypy list`, `tree`,
and `retag` all work while the service is running. DuckDB locks per process, so
without that a running service would block every other command.

`wire` builds a virtualenv at `prototype/.venv`, installs the package into it in
editable mode so source edits take effect without reinstalling, and symlinks
`sypy` into `~/.local/bin`. It refuses to replace a `sypy` it did not create,
and `unwire` refuses to delete one, so an unrelated command of the same name
survives both. Override the locations with `SYPY_VENV_DIR` and `SYPY_BIN_DIR`.

Without wiring, run it through the project directly:

```bash
uv run --project prototype sypy ingest --input ./inbox
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
- No resumable run state beyond the database: an interrupted pass keeps the
  papers it filed and redoes the batch it was in the middle of.
- A preview run still creates an empty `papers.duckdb`, though it writes no rows.
- Preview and apply are separate model calls, so the categories a preview shows
  are not always the ones a later run produces.
- Link naming needs title, authors, *and* year; a document missing only the year
  falls back to the opaque store name.
- There is no way to remove a document from the library short of editing the
  database by hand.
