"""Command line entry points for the prototype."""

from __future__ import annotations

import asyncio
import logging
from pathlib import Path

import typer

from .config import ConfigError, resolve_api_key, resolve_settings
from .ingest import ingest_folder
from .llm import OpenAiClient
from .store import INGEST_INDEX_FILE, IngestIndex
from .watch import watch as watch_folder

app = typer.Typer(help="SortYourPapers Python prototype: LLM ingest and folder watcher.")


@app.callback()
def _configure(verbose: bool = typer.Option(False, "--verbose", "-v")) -> None:
    logging.basicConfig(
        level=logging.DEBUG if verbose else logging.INFO,
        format="%(levelname)s %(message)s",
    )


@app.command()
def ingest(
    input_dir: Path = typer.Option(None, "--input", "-i", help="Folder of PDFs."),
    output_dir: Path = typer.Option(None, "--output", "-o", help="Library folder."),
    recursive: bool = typer.Option(None, "--recursive", "-r"),
    page_cutoff: int = typer.Option(None, "--page-cutoff", "-p"),
    batch_size: int = typer.Option(None, "--batch-size"),
    model: str = typer.Option(None, "--model", "-m"),
) -> None:
    """Read every not-yet-ingested PDF in the input folder."""
    settings, client = _build(
        input_dir, output_dir, recursive, page_cutoff, batch_size, model
    )
    index = IngestIndex(settings.output_dir / INGEST_INDEX_FILE)
    report = asyncio.run(ingest_folder(settings, client, index))

    typer.echo(
        f"ingested {report.processed} paper(s); "
        f"{report.skipped_already_ingested} already known; "
        f"{len(report.skipped_oversized)} oversized; {len(report.failed)} failed"
    )
    for record in report.ingested:
        typer.echo(f"  {Path(record.path).name} -> {record.preliminary_category}")
    for path, reason in report.failed:
        typer.echo(f"  ! {path.name}: {reason}", err=True)
    if report.failed:
        raise typer.Exit(code=1)


@app.command()
def watch(
    input_dir: Path = typer.Option(None, "--input", "-i", help="Folder to watch."),
    output_dir: Path = typer.Option(None, "--output", "-o", help="Library folder."),
    recursive: bool = typer.Option(None, "--recursive", "-r"),
    page_cutoff: int = typer.Option(None, "--page-cutoff", "-p"),
    batch_size: int = typer.Option(None, "--batch-size"),
    model: str = typer.Option(None, "--model", "-m"),
) -> None:
    """Ingest the input folder whenever new PDFs settle in it."""
    settings, client = _build(
        input_dir, output_dir, recursive, page_cutoff, batch_size, model
    )
    try:
        asyncio.run(watch_folder(settings, client))
    except KeyboardInterrupt:
        typer.echo("stopped")


def _build(
    input_dir: Path | None,
    output_dir: Path | None,
    recursive: bool | None,
    page_cutoff: int | None,
    batch_size: int | None,
    model: str | None,
):
    try:
        settings = resolve_settings(
            input_dir,
            output_dir,
            recursive=recursive,
            page_cutoff=page_cutoff,
            keyword_batch_size=batch_size,
            model=model,
        )
        return settings, OpenAiClient(resolve_api_key(), settings.model)
    except ConfigError as err:
        typer.echo(f"error: {err}", err=True)
        raise typer.Exit(code=2) from err


def main() -> None:
    app()


if __name__ == "__main__":
    main()
