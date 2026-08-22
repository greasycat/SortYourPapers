"""Command line entry points for the prototype."""

from __future__ import annotations

import asyncio
import logging
from pathlib import Path

import typer

from .config import ConfigError, resolve_api_key, resolve_settings
from .ingest import ingest_folder
import os
import subprocess

from .library import FilingMode, Library
from .llm import OpenAiClient
from .naming import split_category
from .watch import watch as watch_loop

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
    library_dir: Path = typer.Option(None, "--library", "-o", help="Library folder."),
    recursive: bool = typer.Option(None, "--recursive", "-r"),
    page_cutoff: int = typer.Option(None, "--page-cutoff", "-p"),
    batch_size: int = typer.Option(None, "--batch-size"),
    model: str = typer.Option(None, "--model", "-m"),
    mode: FilingMode = typer.Option(
        FilingMode.PREVIEW.value,
        "--mode",
        help="preview (default), copy to leave the source in place, or move.",
    ),
) -> None:
    """Read every not-yet-known document and file it into the library."""
    settings, client = _build(
        input_dir, library_dir, recursive, page_cutoff, batch_size, model
    )
    with Library(settings.output_dir) as library:
        report = asyncio.run(ingest_folder(settings, client, library, mode=mode))

        verb = "filed" if mode.writes else "would file"
        typer.echo(
            f"{verb} {report.processed} document(s); "
            f"{report.skipped_already_known} already known; "
            f"{len(report.skipped_oversized)} oversized; {len(report.failed)} failed"
        )
        if report.rescan and report.rescan.changed:
            typer.echo(
                f"  ({len(report.rescan.changed)} stored file(s) changed on disk; "
                "hashes refreshed)"
            )
        for filing in report.filed or report.planned:
            typer.echo(f"  {filing.describe()}")
        for path, reason in report.failed:
            typer.echo(f"  ! {path.name}: {reason}", err=True)
        if not mode.writes and report.planned:
            typer.echo("\nnothing was written; re-run with --mode copy or --mode move")

    if report.failed:
        raise typer.Exit(code=1)


@app.command()
def watch(
    input_dir: Path = typer.Option(None, "--input", "-i", help="Folder to watch."),
    library_dir: Path = typer.Option(None, "--library", "-o", help="Library folder."),
    recursive: bool = typer.Option(None, "--recursive", "-r"),
    page_cutoff: int = typer.Option(None, "--page-cutoff", "-p"),
    batch_size: int = typer.Option(None, "--batch-size"),
    model: str = typer.Option(None, "--model", "-m"),
    mode: FilingMode = typer.Option(
        FilingMode.PREVIEW.value,
        "--mode",
        help="preview (default), copy to leave the source in place, or move.",
    ),
) -> None:
    """File new documents into the library whenever they settle in the input folder."""
    settings, client = _build(
        input_dir, library_dir, recursive, page_cutoff, batch_size, model
    )
    try:
        with Library(settings.output_dir) as library:
            asyncio.run(watch_loop(settings, client, library, mode=mode))
    except KeyboardInterrupt:
        typer.echo("stopped")


@app.command()
def tree(
    library_dir: Path = typer.Option(None, "--library", "-o", help="Library folder."),
) -> None:
    """Rebuild the symlink tree from the database."""
    settings = _settings(None, library_dir)
    with Library(settings.output_dir) as library:
        linked = library.rebuild_tree()
        typer.echo(f"linked {linked} document(s) under {library.tree_dir}")
        for paper in library.missing_files():
            typer.echo(
                f"  ! {paper.file_id}: {paper.store_name} is missing from the store",
                err=True,
            )


@app.command()
def retag(
    file_id: str = typer.Argument(..., help="Document id, the part before the first __."),
    category: str = typer.Argument(..., help="New category path, e.g. 'AI/Transformers'."),
    library_dir: Path = typer.Option(None, "--library", "-o", help="Library folder."),
) -> None:
    """Give a document new tags, renaming its file and moving its link."""
    settings = _settings(None, library_dir)
    tags = split_category(category)
    if not tags:
        typer.echo(f"error: {category!r} has no usable tags", err=True)
        raise typer.Exit(code=2)

    with Library(settings.output_dir) as library:
        try:
            paper = library.retag(file_id, tags)
        except Exception as err:
            typer.echo(f"error: {err}", err=True)
            raise typer.Exit(code=1) from err
        typer.echo(f"{paper.store_name}  [{' / '.join(paper.tags)}]")


@app.command()
def note(
    file_id: str = typer.Argument(..., help="Document id, from `sypy list`."),
    library_dir: Path = typer.Option(None, "--library", "-o", help="Library folder."),
) -> None:
    """Open a document's notes, creating them if they do not exist yet.

    Notes live in the document's folder in the store, so they are backed up with
    it, follow it when it is re-tagged, and are reachable through the tree.
    """
    settings = _settings(None, library_dir)
    with Library(settings.output_dir) as library:
        paper = library.db.get(file_id)
        if paper is None:
            typer.echo(f"error: no document with id {file_id}", err=True)
            raise typer.Exit(code=1)
        if not library.document_dir(paper).is_dir():
            typer.echo(f"error: {paper.store_name} is missing from the store", err=True)
            raise typer.Exit(code=1)

        path = library.note_path(paper)
        if not path.exists():
            title = paper.title or paper.original_name or paper.store_name
            path.write_text(f"# {title}\n\n", encoding="utf-8")

    editor = os.environ.get("VISUAL") or os.environ.get("EDITOR")
    if editor:
        # Split so EDITOR="code -w" works, and let the editor own the terminal.
        subprocess.call([*editor.split(), str(path)])
    else:
        # No editor configured, so print the path for the caller to use.
        typer.echo(path)


@app.command("migrate-store")
def migrate_store(
    library_dir: Path = typer.Option(None, "--library", "-o", help="Library folder."),
) -> None:
    """Move a pre-folder library into a folder per document."""
    settings = _settings(None, library_dir)
    with Library(settings.output_dir) as library:
        moved = library.migrate_store_layout()
        if not moved:
            typer.echo("nothing to migrate; every document already has a folder")
            return
        for file_id in moved:
            paper = library.db.get(file_id)
            typer.echo(f"  {file_id} -> {paper.store_name}/{paper.document_name}")
        typer.echo(f"moved {len(moved)} document(s) into folders")
        library.rebuild_tree()
        typer.echo("tree rebuilt")


@app.command()
def scan(
    library_dir: Path = typer.Option(None, "--library", "-o", help="Library folder."),
) -> None:
    """Re-check stored files and refresh the hashes of any edited in place."""
    settings = _settings(None, library_dir)
    with Library(settings.output_dir) as library:
        report = library.rescan()
        typer.echo(
            f"checked {report.checked} document(s); "
            f"{report.rehashed} restatted; {len(report.changed)} changed; "
            f"{len(report.missing)} missing"
        )
        for file_id, old, new in report.changed:
            typer.echo(f"  {file_id}: {old[:12]} -> {new[:12]}")
        for paper in report.missing:
            typer.echo(
                f"  ! {paper.file_id}: {paper.store_name} is missing from the store",
                err=True,
            )


@app.command()
def remove(
    file_id: str = typer.Argument(..., help="Document id, the part before the first __."),
    library_dir: Path = typer.Option(None, "--library", "-o", help="Library folder."),
    yes: bool = typer.Option(False, "--yes", "-y", help="Skip the confirmation."),
) -> None:
    """Delete a document from the library: its link, its file, and its record."""
    settings = _settings(None, library_dir)
    with Library(settings.output_dir) as library:
        paper = library.db.get(file_id)
        if paper is None:
            typer.echo(f"error: no document with id {file_id}", err=True)
            raise typer.Exit(code=1)

        label = paper.title or paper.original_name or paper.store_name
        if not yes:
            typer.echo(f"{paper.store_name}\n  {label}")
            # The stored file is the only copy when it arrived by move, so this
            # is not undoable.
            typer.confirm("permanently delete this document?", abort=True)

        library.remove(file_id)
        typer.echo(f"removed {file_id}")


@app.command("list")
def list_papers(
    library_dir: Path = typer.Option(None, "--library", "-o", help="Library folder."),
) -> None:
    """List what the library holds."""
    settings = _settings(None, library_dir)
    with Library(settings.output_dir) as library:
        papers = library.db.all_papers()
        for paper in papers:
            tags = " / ".join(paper.tags) or "-"
            typer.echo(f"{paper.file_id}  {tags:<40}  {paper.title or paper.original_name}")
        typer.echo(f"\n{len(papers)} document(s) in {library.root}")


def _settings(input_dir: Path | None, library_dir: Path | None):
    try:
        return resolve_settings(input_dir, library_dir)
    except ConfigError as err:
        typer.echo(f"error: {err}", err=True)
        raise typer.Exit(code=2) from err


def _build(
    input_dir: Path | None,
    library_dir: Path | None,
    recursive: bool | None,
    page_cutoff: int | None,
    batch_size: int | None,
    model: str | None,
):
    try:
        settings = resolve_settings(
            input_dir,
            library_dir,
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
