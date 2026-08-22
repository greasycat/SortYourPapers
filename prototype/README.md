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
named by — all in one request per batch.

Each request also carries the category paths the library already uses, so a new
document joins an existing branch instead of inventing a parallel one. Without
it an electricity bill and a water bill land under unrelated top-level folders;
with it the second joins the first. Batches within one pass run concurrently and
cannot see each other's choices, so two new documents arriving together can
still diverge — arriving one at a time, as under the watcher, they cannot. Batches run four at a time,
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
    5112ee75ddcf__Machine Learning__Deep Learning__Transformers/
      vaswani_2017_attention-is-all-you-need.pdf
      notes.md              <- yours, and as durable as the document
      figure-3.png          <- likewise
  tree/
    Machine Learning/Deep Learning/Transformers/
      vaswani_2017_attention-is-all-you-need/          <- a real folder
        vaswani_2017_attention-is-all-you-need -> ../../../../store/5112ee75ddcf__...
```

Every document has one home: a folder in the store holding the document and
whatever you keep beside it. **That folder is the durable thing** — it is what
gets backed up, what re-tagging renames, and what removal deletes.

`tree/` is a view and nothing else: delete it and `sypy tree` rebuilds it
exactly. Each document has a real folder there holding a single link to its
store folder, so browsing a category and opening a document keeps you in the
tree rather than throwing you into the store; following that one link is what
takes you to the document and its notes.

The cost of those folders being real is that things can be written into them,
and the tree is neither backed up nor preserved across a rebuild-from-scratch.
So `sypy tree` reports any file it finds living there and tells you to move it
into the document's folder. It never deletes it — that is not this tool's call.

The store filename is `<id>__<Tag>__<Tag>.pdf`. The id is permanent and the tags
are not, so **re-tagging a paper is a rename plus a moved link** — no file is
ever copied and nothing has to be found again. Tags are sanitized so no tag can
contain `__`, which keeps the name unambiguous to parse.

Links are named from whichever of author, year, and title are known —
`vaswani_2017_attention-is-all-you-need`, or `vaswani_attention…` when the year
is missing, or `attention…` when only the title is. A document with neither an
author nor a title falls back to the store name, since a bare year names
nothing. A long title is cut between words rather than inside one, so a name
never trails off mid-word.

Because those names are derived, changing how they are derived leaves existing
files spelled the old way. `sypy migrate-store` renames them to match. They are relative, so
the whole library can be moved without breaking.

**DuckDB is the source of truth.** Filenames and the links are projections of
it, which is why `sypy tree` can discard the links and rebuild them exactly, and
why a tag list too long for a filename loses nothing. Papers are keyed by a hash of
their contents, so re-running costs nothing and the same paper arriving twice
under different names is recognised.

Expanding the schema means appending to `_MIGRATIONS` in `db.py`; anything not
worth a column yet goes in `paper_attributes` as a key/value pair.

## Scanned documents

A PDF with no text layer opens fine and yields nothing to read. Its pages are
rendered with `pdftoppm` and handed to the model, which writes plain text
standing in for the text the document does not carry — the title, the authors,
the date, and a short description of what it covers.

That text then goes through the ordinary labelling call, so a scan is batched,
steered, and named exactly like any other document. It costs one extra request
per scanned document, and `from_page_images` records which documents were read
this way, since their metadata is a model's reading of a picture rather than the
document's own words.

Needs poppler (`brew install poppler`), the same dependency the Rust pipeline
already has. Without it, a scan is reported as a failure naming what to install.

## Notes

```bash
sypy note <id>
```

Opens `notes.md` in the document's folder, creating it with the title as a
heading. With no `$EDITOR` set it prints the path instead, so
`$(sypy note <id>)` composes.

Notes are just files in the folder, so anything else you put there — figures,
supplements, a scanned appendix — gets the same treatment. All of it is backed
up with the document, follows it through a re-tag, and is deleted with it, which
is why `sypy remove` asks first.

## Editing a stored file

Annotating or re-saving a file in the store keeps its name but changes its
bytes, which makes the recorded hash stale. That matters because the hash is how
the library recognises a document it already holds: left stale, the edited copy
arriving in a watched folder later would be ingested a second time.

Every ingest reconciles the store before deciding what is already known, so
this is handled without anyone remembering to do it — including under the
watcher. It compares each stored file's size and mtime against what was recorded
and only reads the ones that moved, so a quiet library costs one stat per
document rather than a full re-read. Changed files get a fresh hash; a file that
vanished is reported rather than silently dropped. A store that cannot be read is
reported and stepped over, because failing to reconcile risks a duplicate while
refusing to run files nothing at all.

`sypy scan` runs the same reconciliation on its own, for checking a library
without ingesting into it.

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
sypy note <id>                         # open this document's notes ($EDITOR)
sypy remove <id>                       # delete link, folder, and record (asks first)
sypy scan                              # refresh hashes of files edited in place
sypy tree                              # rebuild the symlink tree from the database

./prototype/scripts/sypy-path unwire   # remove the link
```

Nothing is written without `--mode`. Use `copy` for a folder you did not create
— a Downloads folder keeps its files and the library gets copies. Re-run `wire`
after changing dependencies.

## One watcher per folder

A watcher claims its input folder and its library before it starts, and refuses
if either is already claimed:

```
error: ~/Documents/sypy-library is already being watched as the library of a
       watcher running as pid 63643
```

Two watchers sharing either folder would file the same document twice: each
decides what the library already holds before either writes, and the database
lock is not held across that gap. Claims are kept in `~/.local/state/sypy`, not
in the folders themselves, so they work across libraries and leave no litter.

A claim whose owner is gone is taken over rather than respected, so a crash does
not lock a folder out. That is also what cleans up after `sypy-service
uninstall`: stopping the agent sends `SIGTERM`, which does not run the release,
so the claims are left behind until the next watcher takes them over.

`sypy ingest` is not covered by this. Running one by hand while a watcher is
going can still file a document twice, because both check before either writes.

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

There is one agent, so installing for a different folder is refused rather than
silently replacing the running one; `uninstall` first. Reinstalling the same
folder is how configuration changes are picked up.

DuckDB allows one writing process at a time, so a running service could shut
every other command out. Three things stop it.

The watcher drops its lock whenever it is idle. A pass runs in phases, so
hashing, parsing, calling the model, and copying all happen with no connection
open and the database is visited in three short bursts between them. And a
command that still collides waits rather than failing, up to 30 seconds, past
which the problem is a stuck process rather than contention.

The phases are what keep this true as the library grows: the bursts are
proportional to the number of documents, not to the bytes being read. On twelve
4MB documents the share of a pass with the lock unavailable is 3%, against 66%
when the file work was done with the connection held.

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

- Only OpenAI is wired up, because that is the key the repo carries.
- No resumable run state beyond the database: an interrupted pass keeps the
  papers it filed and redoes the batch it was in the middle of.
- A preview moves no files, but it does reconcile: a stored file edited in
  place has its hash refreshed even under `--mode preview`.
- Preview and apply are separate model calls, so the categories a preview shows
  are not always the ones a later run produces.
- `remove` deletes the stored file, which is the only copy when the document
  arrived by move. There is no unfile-but-keep option.
- Category steering sends at most 200 existing paths; a larger library sends its
  alphabetically first.
- Only watchers claim folders. A hand-run `sypy ingest` racing a watcher can
  still file the same document twice.
- Steering can also mislead. A 1990 radiology paper joined an existing
  `Machine Learning/Explainable AI/Medical Imaging` branch because it was the
  nearest thing present, where an unsteered run put it under
  `Medicine/Radiology`. `sypy retag` is the fix when it happens.
- The service log has no timestamps, so restarts are hard to tell apart.
- Moving a link by hand still does not re-tag anything: links are relative, so
  moving one to a different depth breaks it, and a rebuild puts it back. Use
  `sypy retag`. Files you add to a document's folder are safe either way.
- `sypy remove` deletes the document's whole store folder, including notes and
  anything else kept in it. It confirms first.
- A file written into the tree rather than the document's folder is reported by
  `sypy tree`, not moved or deleted. It is not durable where it sits.
- A library made before documents had folders needs `sypy migrate-store` once.
