from __future__ import annotations

from pathlib import Path

import typer

from .catalog import DatasetSource, load_candidates
from .curate import build_curated_test_set
from .manifest import load_test_set, save_test_set
from .materialize import export_test_set, materialize_test_set
from .models import SamplingPolicy

app = typer.Typer(help="Curate and materialize SciJudgeBench-backed arXiv test sets.")


@app.command("build-manifest")
def build_manifest(
    output: Path = typer.Option(
        Path("assets/testsets/scijudgebench-diverse.toml"),
        "--output",
        help="Base manifest path to write. Writes both .toml and .json artifacts.",
    ),
    top_n: int = typer.Option(5, "--top-n"),
    bottom_n: int = typer.Option(5, "--bottom-n"),
    random_n: int = typer.Option(5, "--random-n"),
    per_subcategory_cap: int = typer.Option(2, "--per-subcategory-cap"),
    random_seed: int = typer.Option(1_511_510_650, "--random-seed"),
    repo_id: str = typer.Option("OpenMOSS-Team/SciJudgeBench", "--repo-id"),
    revision: str = typer.Option("main", "--revision"),
) -> None:
    source = DatasetSource(repo_id=repo_id, revision=revision)
    candidates = load_candidates(source)
    test_set = build_curated_test_set(
        candidates,
        SamplingPolicy(
            top_n_per_category=top_n,
            bottom_n_per_category=bottom_n,
            random_n_per_category=random_n,
            random_seed=random_seed,
            per_subcategory_cap=per_subcategory_cap,
        ),
    )
    toml_path, json_path = _artifact_paths(output)
    save_test_set(toml_path, test_set)
    save_test_set(json_path, test_set)
    typer.echo(f"Wrote manifests to {toml_path} and {json_path}")


@app.command("materialize")
def materialize(
    manifest_path: Path = typer.Argument(..., exists=True, readable=True),
    cache_dir: Path | None = typer.Option(None, "--cache-dir"),
    force: bool = typer.Option(False, "--force", help="Re-download all PDFs."),
) -> None:
    test_set = load_test_set(manifest_path)
    report = materialize_test_set(test_set, cache_root=cache_dir, force_download=force)
    typer.echo(f"Materialized {len(report.papers)} papers into {report.cache_dir}")


@app.command("export")
def export(
    manifest_path: Path = typer.Argument(..., exists=True, readable=True),
    output_dir: Path = typer.Argument(...),
    cache_dir: Path | None = typer.Option(None, "--cache-dir"),
    force: bool = typer.Option(False, "--force", help="Re-download all PDFs before export."),
) -> None:
    test_set = load_test_set(manifest_path)
    report = materialize_test_set(test_set, cache_root=cache_dir, force_download=force)
    exported = export_test_set(report, output_dir)
    typer.echo(f"Exported {len(exported)} PDFs into {output_dir}")


def main() -> None:
    app()


def _artifact_paths(output: Path) -> tuple[Path, Path]:
    base = output.with_suffix("") if output.suffix.lower() in {".toml", ".json"} else output
    return base.with_suffix(".toml"), base.with_suffix(".json")


if __name__ == "__main__":
    main()
