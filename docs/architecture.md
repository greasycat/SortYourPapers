# SortYourPapers Architecture

A repo-structure reference for the current codebase. It reflects the source
tree as it exists now, not the original implementation plan.

The Rust workspace this document used to describe — `crates/syp`, `syp-core`,
`syp-ai`, `syp-library`, `syp-workflow`, `paper-db` — was removed from `main`
and is preserved on the `old-rust` branch.

## Repository Layout
- `prototype/`: the tool — a Python ingest pipeline and folder watcher.
- `python/`: `uv`-managed maintainer tooling for SciJudgeBench sampling and
  arXiv PDF materialization.
- `assets/testsets/`: committed test-set manifests for fetched paper corpora.
- `docs/`: project documentation.
- `docs/archive/`: historical planning material kept for reference.

## The Pipeline
`prototype/src/syp_prototype/` holds the whole of it. A pass runs in phases,
and the split is deliberate: DuckDB allows one writing process, so everything
slow happens with no connection open and the database is visited in short
bursts between. `ingest.py`'s module docstring is the authority on that order
and on why each boundary sits where it does.

- `cli.py`: the `sypy` command surface, and logging setup.
- `config.py`: settings resolved CLI > environment > defaults, plus the
  machine-wide state directory.
- `discovery.py`: finding candidate PDFs, and the content hash that identifies
  a document.
- `extract.py`: pulling a text layer out of a PDF.
- `render.py`: rendering pages with `pdftoppm`, for documents that carry none.
- `llm.py`: the one model call — text in, keywords and a category out — behind
  a Protocol, so the pipeline runs end to end without a network or a key.
- `budget.py`: the rolling-day ceiling on what may be spent at the API.
- `ingest.py`: one pass, in phases.
- `watch.py`: the long-running watcher. Filesystem events are a wake-up hint;
  the folder scan is the source of truth.
- `watchlock.py`: claims that stop two watchers sharing an input folder or a
  library, owned by an `flock` rather than by a pid.
- `registry.py`: `~/.config/sypy/config.toml`, the one file saying what this
  machine watches.
- `library.py`: the store, the symlink tree over it, and backups.
- `db.py`: the DuckDB schema and every query, including the bank of model
  answers that stops a document being paid for twice.
- `naming.py`: store names, link names, and the rules for cutting them.

## Test-Set Fetching Layout
- `python/src/syp_paperfetch/catalog.py`: Hugging Face Hub dataset download and
  SciJudgeBench pair flattening.
- `python/src/syp_paperfetch/curate.py`: top/bottom/random citation sampling per
  category with subcategory caps.
- `python/src/syp_paperfetch/manifest.py`: TOML manifest load/save helpers.
- `python/src/syp_paperfetch/materialize.py`: arXiv PDF download, cache
  verification, and export helpers.
- `python/src/syp_paperfetch/cli.py`: `uv`-run CLI entrypoints for
  build/materialize/export.

## Notes
- The original greenfield implementation plan lives in
  `docs/archive/initial-implementation-plan.md`. It describes the Rust design.
