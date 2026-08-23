# Test Sets

Curated paper test sets, committed as TOML and JSON metadata artifacts. Only
metadata is committed; the PDFs they name were downloaded on demand.

**Nothing in `main` builds or reads these any more.** They were produced by
`syp-paperfetch`, a `uv`-managed maintainer package that lived in `python/`,
and scored by the Rust evaluation harness — the source of the measurements in
`docs/clustering_bench.md` and `docs/embedding_comparison.md`. The harness is on
the `old-rust` branch; `syp-paperfetch` is in this repository's history. They
are kept here as data, so the measurements those documents report stay
attributable to a named set of papers.

Every sample stores paper metadata plus both the arXiv abstract URL and the
direct PDF URL, so a set can be re-fetched by hand from what is committed.

## Sets

- `scijudgebench-diverse` — spread widely across subcategories (60 papers, 45
  subcategories), built from SciJudgeBench with a `5 top + 5 bottom + 5
  deterministic random` policy per category. Good for exercising a pipeline on
  varied inputs; too thin per label to score grouping quality.
- `clustering-eval` — a balanced policy for measuring grouping: a few
  subcategories with equal paper counts, so every reference label carries the
  same weight. No artifact was ever committed, and `docs/clustering_bench.md`
  still names building one as the next thing worth doing. It is described here
  because that document refers to it, not because it is on disk.
