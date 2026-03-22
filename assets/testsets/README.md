# Test Sets

Curated paper test sets live here as TOML manifests.

- Commit manifests only.
- Materialized PDFs are downloaded on demand into the XDG cache tree.
- The maintainer workflow lives under `python/` and is run via `uv`.
- `uv run --project python paperfetch materialize assets/testsets/scijudgebench-diverse.toml` downloads the referenced arXiv PDFs into the cache.

The initial scaffold manifest is `scijudgebench-diverse.toml`.
