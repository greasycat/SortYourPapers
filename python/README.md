# Python Test-Set Tooling

This directory contains the `uv`-managed maintainer workflow for building and materializing curated paper test sets.

Common commands:

```bash
uv run --project python paperfetch build-manifest
uv run --project python paperfetch materialize ../assets/testsets/scijudgebench-diverse.toml
uv run --project python paperfetch export ../assets/testsets/scijudgebench-diverse.toml ../tmp/scijudgebench
uv run --project python pytest
```
