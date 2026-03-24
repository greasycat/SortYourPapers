# Python Test-Set Tooling

This directory contains the `uv`-managed maintainer workflow for building and materializing curated paper test sets.

`build-manifest` writes both TOML and JSON artifacts. Each curated sample stores paper metadata, the arXiv abstract page link, and the direct PDF link.

Common commands:

```bash
uv run --project python paperfetch build-manifest
uv run --project python paperfetch materialize ../assets/testsets/scijudgebench-diverse.toml
uv run --project python paperfetch export ../assets/testsets/scijudgebench-diverse.toml ../tmp/scijudgebench
uv run --project python --extra dev python -m pytest python/tests
```

By default, `materialize` and `export` use the shared repo-relative test-set cache from `../dev.toml`.
