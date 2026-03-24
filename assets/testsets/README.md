# Test Sets

Curated paper test sets live here as committed TOML and JSON artifacts.

- Commit metadata artifacts only.
- Materialized PDFs plus manifest/state metadata are downloaded on demand into the shared repo-relative cache declared in `dev.toml`.
- The maintainer workflow lives under `python/` and is run via `uv`.
- `scijudgebench-diverse.{toml,json}` is built from SciJudgeBench using a `5 top + 5 bottom + 5 deterministic random` policy per category.
- Every sample stores paper metadata plus both the arXiv abstract URL and direct PDF URL.
- `uv run --project python paperfetch build-manifest --output assets/testsets/scijudgebench-diverse.toml` refreshes both committed artifacts.
- `uv run --project python paperfetch materialize assets/testsets/scijudgebench-diverse.toml` downloads the referenced arXiv PDFs into the cache.
