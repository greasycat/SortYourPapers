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

## What leaves your machine

**Every document you file is sent to OpenAI.** There is no local model and no
offline mode: labelling a document means uploading the text of its first page
(or, for a scan, an image of it) to `api.openai.com`, under the key in your
`.env`. Bills, contracts, medical letters, and anything else that lands in a
watched folder go the same way as a paper does — the watcher does not ask, and
under the service it happens without anyone at the keyboard.

Concretely, each request carries:

- up to 4,000 characters of the document's text, or the first page rendered as
  an image when it carries no text layer
- the filename
- the list of category paths your library already uses, which describes the
  shape of your whole collection

The file itself is never uploaded, and nothing is sent for a document the
library already holds. What OpenAI does with the rest is between you and their
terms; this tool cannot promise anything about it. If that is not acceptable for
a folder, do not point a watch at it.

## What it costs

The watcher runs unattended under `KeepAlive`, which means the thing that
reliably restarts it is failure — and a pass that dies partway through comes
back, re-reads the same folder, and pays again. So spending is capped:

```bash
sypy budget           # what the last 24 hours cost, against the ceiling
sypy budget --reset   # start the window over
```

Requests and tokens are counted in a rolling 24-hour window, machine-wide rather
than per-library, in `~/.local/state/sypy/spend.json`. A request that would start
past the ceiling is refused before it is sent, because a limit checked afterwards
has already been paid. Defaults are 500 requests and 1,000,000 tokens a day;
raise them with `SYP_MAX_REQUESTS_PER_DAY` and `SYP_MAX_TOKENS_PER_DAY`, or set
either to `0` to turn that ceiling off.

Each request also has a retry ceiling and a timeout (`SYP_LLM_MAX_RETRIES`,
default 2; `SYP_LLM_TIMEOUT_SECONDS`, default 120), so one failing or hanging
request gives up instead of holding a pass open and billing for every attempt.

The ceilings bound the damage; they do not price it. This tool does not know
what the model costs, and does not try to guess.

## Backups

```bash
sypy backup ~/Backups/sypy-2026-08-22
```

The store and the database are only useful together — the store alone is a
folder of documents nothing can find, the database alone is a catalogue of files
that are gone — so one command copies both. `tree/` is skipped; `sypy tree`
rebuilds it.

The database is copied first, and through DuckDB rather than as a file: a
database being written has changes in a log beside it, and a file copy taken at
the wrong moment restores short of its most recent rows or will not open at all.
The ordering matters for the same reason filing writes the file before the row.
If the watcher files something between the two halves, the copy holds a folder
with no row — an orphan, which `sypy fsck --adopt` brings back — rather than a
row pointing at a document that no longer exists anywhere.

For a nightly copy, run it from cron or a launchd agent.

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

launchd gives an agent a bare `PATH` that does not include Homebrew, so the
service is installed with poppler's directory added explicitly. Without that,
scans fail only under the service while working by hand — install warns if
`pdftoppm` cannot be found.

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

## When the two halves disagree

```bash
sypy fsck            # what is wrong
sypy fsck --adopt    # bring orphaned folders back in
```

Filing puts a document's file down before writing its row, so that an
interruption leaves a folder nothing points at rather than a row pointing at
nothing. That is the better half to be left holding, but only because `fsck`
can find it: an unclaimed folder is otherwise absent from `list`, from the tree,
from de-duplication, and from `remove` — and under `--mode move` it is the only
copy of the document.

`--adopt` gives such a folder a row. The id and tags come back from its name and
the hash from its bytes; title, authors, and year do not, because they only ever
lived in the database. Re-ingesting does **not** heal an orphan — ids are minted
fresh, so it just files a second copy.

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
sypy fsck [--adopt]                    # check the store and database agree
sypy tree                              # rebuild the symlink tree from the database
sypy backup <dir>                      # copy the store and the database, together
sypy budget                            # what the last 24 hours cost at the API

./prototype/scripts/sypy-path unwire   # remove the link
```

Nothing is written without `--mode`. Use `copy` for a folder you did not create
— a Downloads folder keeps its files and the library gets copies. Re-run `wire`
after changing dependencies.

## The registry

One file says what this machine watches, at
`~/.config/sypy/config.toml`:

```toml
default = "downloads"

[watch.downloads]
input   = "~/Downloads"
library = "~/Documents/sypy-library"
mode    = "copy"
```

```bash
sypy watches        # what is declared, and which are running
sypy watch          # run the default watch
sypy watch papers   # run a named one
```

With a registry, the other commands stop needing `--library`: it resolves
CLI > `SYP_OUTPUT` > the registry's default watch > `./sorted`. A single
declared watch is the default without saying so; past that, `default` has to
name one, because picking would be a guess.

Two watches sharing an input folder or a library are refused when the file is
read, naming both — so the mistake surfaces while it is being made rather than
hours later when the second watcher will not start.

Nothing needs the registry. Passing `--input` and `--library` still works, and a
missing file is an empty registry rather than an error.

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

Who owns a claim is decided by an `flock` on the claim file, not by the pid
written inside it. A pid is not an identity — it gets reused — so a claim left by
a crashed watcher would otherwise name whatever process was handed that number
next, and reading liveness off it locks the folder out for as long as that
stranger lives. The kernel drops an `flock` when the holder dies, however it
dies. The pid is still recorded, because "already watched by pid 63643" is what
makes the refusal actionable, but nothing is decided from it.

`sypy ingest` is not covered by this. Running one by hand while a watcher is
going can still file a document twice, because both check before either writes.

## Running it as a service

```bash
./prototype/scripts/sypy-service install            # the registry's default watch
./prototype/scripts/sypy-service install papers     # a named watch
./prototype/scripts/sypy-service status
./prototype/scripts/sypy-service logs
./prototype/scripts/sypy-service uninstall
```

Installs a launchd agent that watches in copy mode, so the watched folder is
indexed but never rearranged. `SYPY_MODE=move` overrides that.

Logs go to `~/Library/Logs/sypy/sypy.log` through a rotating handler — 2MB a
file, three kept — so a service left running for months cannot fill the disk.
Every line carries a timestamp, because the log is read hours later and often
after a restart, and every document that failed is named along with why: a full
disk, an expired key, and a corrupt PDF all read the same as "1 failed".

launchd's own capture of the process goes to `sypy.crash.log` beside it and
holds only what logging never sees — a traceback from a crash. `sypy-service
logs` shows the tail of both.

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

`wire` builds a virtualenv at `prototype/.venv`, installs the dependencies from
`requirements.lock`, installs the package into it in editable mode so source
edits take effect without reinstalling, and symlinks `sypy` into `~/.local/bin`.

The lock is what stops two machines set up a week apart running different code,
which is what makes a break arriving with a dependency indistinguishable from
one arriving with a commit. Move it forward deliberately:

```bash
./prototype/scripts/sypy-path relock    # re-resolve, then review the diff
```

It resolves in a throwaway virtualenv, so the test dependencies and whatever a
debugging session left behind stay out of it. Versions are pinned, not hashes:
it says what is installed, and does not try to prove the index handed over the
same bytes as last time. It refuses to replace a `sypy` it did not create,
and `unwire` refuses to delete one, so an unrelated command of the same name
survives both. Override the locations with `SYPY_VENV_DIR` and `SYPY_BIN_DIR`.

Without wiring, run it through the project directly:

```bash
uv run --project prototype sypy ingest --input ./inbox
```

Running the tests needs the dev extras, which `wire` does not install:

```bash
prototype/.venv/bin/python -m pip install -e "prototype[dev]"
prototype/.venv/bin/python -m pytest
```

Every test is capped at 60 seconds by `pytest-timeout`, using the signal method
so it can interrupt a loop that never yields. Several tests drive the watcher,
and `asyncio.wait_for` cannot preempt a coroutine that stops awaiting — without
the cap, a change that removes an `await` from the watch loop spins at full CPU
until the machine is unusable.

Without `--input` the current directory is watched, and the library defaults to
`sorted` inside it.

## Configuration

Settings resolve CLI > environment > defaults, reusing the Rust pipeline's
`SYP_*` names so a folder set up for one reads the same to the other:
`SYP_INPUT`, `SYP_OUTPUT`, `SYP_RECURSIVE`, `SYP_MAX_FILE_SIZE_MB`,
`SYP_PAGE_CUTOFF`, `SYP_KEYWORD_BATCH_SIZE`, `SYP_LLM_MODEL`.

What the prototype may spend, and how hard it tries:
`SYP_MAX_REQUESTS_PER_DAY`, `SYP_MAX_TOKENS_PER_DAY`, `SYP_LLM_MAX_RETRIES`,
`SYP_LLM_TIMEOUT_SECONDS`.

Where it keeps what one invocation leaves for the next — watch claims and the
spend ledger — is `SYPY_STATE_DIR`, defaulting to `~/.local/state/sypy`. The
registry is `SYPY_CONFIG_DIR`, and `SYPY_LOG_FILE` turns on the rotating log
(the service sets it; a second process rotating the same file can lose lines).

The API key is read from `OPENAI_API_KEY`, `SYP_API_KEY`, or `OEPNAI_API_KEY`,
including from the repository-root `.env`. The third spelling is a typo this
repo's `.env` currently carries; it is accepted so the prototype works as-is,
and the correct spelling wins when both are set.

## Known gaps

- Only OpenAI is wired up, because that is the key the repo carries.
- No resumable run state beyond the database: an interrupted pass keeps the
  papers it filed and redoes the batch it was in the middle of.
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
- Moving a link by hand still does not re-tag anything: links are relative, so
  moving one to a different depth breaks it, and a rebuild puts it back. Use
  `sypy retag`. Files you add to a document's folder are safe either way.
- `sypy remove` deletes the document's whole store folder, including notes and
  anything else kept in it. It confirms first.
- A file written into the tree rather than the document's folder is reported by
  `sypy tree`, not moved or deleted. It is not durable where it sits. If it is
  sitting exactly where a document's link belongs, that document is linked into
  an id-decorated folder beside it instead; the file is never replaced.
- The spend ceiling counts requests and tokens, not money. It bounds the damage
  from a restart loop; it does not know what the model costs.
- Four batches run at once and each reserves its request before sending, so the
  ceiling is enforced at the request that crosses it — the tokens that request
  turns out to cost are recorded after, and can carry the day slightly past the
  token ceiling.
- `sypy backup` copies; it does not rotate, prune, or verify old backups.
- The lock pins versions, not hashes.
- A library made before documents had folders needs `sypy migrate-store` once.
