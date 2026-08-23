"""Command line entry points for `sypy`."""

from __future__ import annotations

import asyncio
import json
import logging
from logging.handlers import RotatingFileHandler
from pathlib import Path
from typing import Sequence

import typer

from .budget import Budget
from .config import (
    DEFAULT_LABEL_CACHE_DAYS,
    DEFAULT_LLM_MAX_RETRIES,
    DEFAULT_LLM_TIMEOUT_SECONDS,
    ConfigError,
    env_float,
    env_int,
    resolve_api_key,
    resolve_settings,
)
from .db import Paper
from .ingest import ingest_folder
import os
import subprocess

from .library import FilingMode, Library, LibraryError
from .llm import OpenAiClient
from .naming import split_category
from .registry import RegistryError, WatchEntry, load_registry, registry_path
from .watch import watch as watch_loop
from .watchlock import WatchConflict

app = typer.Typer(help="SortYourPapers: LLM ingest and folder watcher.")


# Kept small enough that the whole history stays cheap to keep and to read:
# four files of 2MB is roughly a fortnight of a busy watcher.
LOG_MAX_BYTES = 2 * 1024 * 1024
LOG_BACKUP_COUNT = 3


@app.callback()
def _configure(verbose: bool = typer.Option(False, "--verbose", "-v")) -> None:
    """Set up logging for whichever command follows.

    Lines carry a timestamp because the watcher's log is read long after the
    fact, and an untimed line cannot say whether it is from this run or the one
    that crashed an hour ago — which is exactly the question a restart loop
    raises.

    With `SYPY_LOG_FILE` set — which is how the service runs — the log goes to
    that file through a rotating handler instead of to stderr, so a watcher left
    running for months cannot fill the disk. Instead of, not as well as: under
    launchd stderr is itself redirected to a file that nothing rotates, and
    writing every line to both would leave the unbounded copy growing exactly as
    before. What still reaches stderr is what logging never sees — a traceback
    from a crash — which is the one thing worth keeping unrotated.

    Only the service sets it. A rotating handler is safe for a single writer,
    and two processes rotating one file can lose each other's lines.
    """
    log_file = (os.environ.get("SYPY_LOG_FILE") or "").strip()
    if log_file:
        path = Path(log_file).expanduser()
        path.parent.mkdir(parents=True, exist_ok=True)
        handlers: list[logging.Handler] = [
            RotatingFileHandler(
                path, maxBytes=LOG_MAX_BYTES, backupCount=LOG_BACKUP_COUNT
            )
        ]
    else:
        handlers = [logging.StreamHandler()]

    logging.basicConfig(
        level=logging.DEBUG if verbose else logging.INFO,
        format="%(asctime)s %(levelname)s %(message)s",
        datefmt="%Y-%m-%d %H:%M:%S",
        handlers=handlers,
        force=True,
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
    name: str = typer.Argument(
        None, help="A watch declared in the registry. Omit to use the default."
    ),
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
    entry = _watch_entry(name) if (name or not input_dir) else None
    if entry is not None:
        input_dir = input_dir or entry.input_dir
        library_dir = library_dir or entry.library_dir
        if mode is FilingMode.PREVIEW:
            mode = entry.mode
    elif name:
        typer.echo(f"error: no watch named {name!r}", err=True)
        raise typer.Exit(code=2)

    settings, client = _build(
        input_dir, library_dir, recursive, page_cutoff, batch_size, model
    )
    try:
        with Library(settings.output_dir) as library:
            asyncio.run(watch_loop(settings, client, library, mode=mode))
    except WatchConflict as err:
        typer.echo(f"error: {err}", err=True)
        typer.echo(
            "  two watchers sharing a folder file the same document twice; "
            "stop the other one first.",
            err=True,
        )
        raise typer.Exit(code=2) from err
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
        litter = library.tree_litter()
        if litter:
            typer.echo(
                f"\n  {len(litter)} file(s) live in the tree, which is rebuilt and "
                "is not backed up with the library:",
                err=True,
            )
            for path in litter:
                typer.echo(f"    {path.relative_to(library.tree_dir)}", err=True)
            typer.echo(
                "  move them into the document's folder "
                "(`sypy note <id>` opens one) to keep them.",
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
    path_only: bool = typer.Option(
        False, "--path", help="Print where the notes are instead of opening them."
    ),
) -> None:
    """Open a document's notes, creating them if they do not exist yet.

    Notes live in the document's folder in the store, so they are backed up with
    it, follow it when it is re-tagged, and are reachable through the tree.

    `--path` prints the file and stops. A caller that means to write the notes
    itself has no use for an editor, and one inheriting an `$EDITOR` it cannot
    drive would be left holding a terminal open forever.
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

    editor = None if path_only else (os.environ.get("VISUAL") or os.environ.get("EDITOR"))
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
    """Bring the store's layout and filenames in line with the current rules."""
    settings = _settings(None, library_dir)
    with Library(settings.output_dir) as library:
        moved = library.migrate_store_layout()
        for file_id in moved:
            paper = library.db.get(file_id)
            typer.echo(f"  folder: {file_id} -> {paper.store_name}/{paper.document_name}")

        renamed = library.refresh_document_names()
        for _, old, new in renamed:
            typer.echo(f"  rename: {old}\n       -> {new}")

        if not moved and not renamed:
            typer.echo("nothing to do; the store already matches the current rules")
            return
        typer.echo(
            f"moved {len(moved)} document(s) into folders, renamed {len(renamed)}"
        )
        library.rebuild_tree()
        typer.echo("tree rebuilt")


@app.command()
def fsck(
    library_dir: Path = typer.Option(None, "--library", "-o", help="Library folder."),
    adopt: bool = typer.Option(
        False, "--adopt", help="Give orphaned folders a row instead of only listing them."
    ),
) -> None:
    """Check the store and the database against each other.

    Reports documents whose file is gone, and folders the database does not know
    about — which is what an interrupted pass leaves behind, and what nothing
    else can find.
    """
    settings = _settings(None, library_dir)
    with Library(settings.output_dir) as library:
        missing = library.missing_files()
        orphans = library.orphans()

        for paper in missing:
            typer.echo(
                f"  missing: {paper.file_id} -> {paper.store_name} is not in the store",
                err=True,
            )

        for directory in orphans:
            if not adopt:
                typer.echo(f"  orphan : {directory.name}", err=True)
                continue
            try:
                paper = library.adopt(directory)
            except LibraryError as err:
                typer.echo(f"  orphan : {directory.name} — {err}", err=True)
                continue
            typer.echo(f"  adopted: {paper.file_id}  {paper.document_name}")

        if not missing and not orphans:
            typer.echo(f"{library.db.count()} document(s); store and database agree")
            return

        typer.echo(
            f"\n{library.db.count()} document(s); "
            f"{len(missing)} missing, {len(orphans)} orphaned"
        )
        if orphans and not adopt:
            typer.echo("re-run with --adopt to bring the orphaned folders back in.")
            typer.echo(
                "Their title, authors, and year are not recoverable — those "
                "lived only in the database."
            )
        raise typer.Exit(code=1)


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


@app.command()
def backup(
    destination: Path = typer.Argument(..., help="Empty folder to copy the library into."),
    library_dir: Path = typer.Option(None, "--library", "-o", help="Library folder."),
) -> None:
    """Copy the store and the database somewhere safe, together.

    They are only useful as a pair, and the tree is not copied because `sypy
    tree` rebuilds it. Run it from cron or a launchd agent for a nightly copy.
    """
    settings = _settings(None, library_dir)
    with Library(settings.output_dir) as library:
        try:
            report = library.backup(destination)
        except LibraryError as err:
            typer.echo(f"error: {err}", err=True)
            raise typer.Exit(code=1) from err

    typer.echo(f"backed up {report.documents} document(s) to {report.destination}")
    typer.echo(f"  database  {report.database.name}")
    typer.echo(f"  store     {report.bytes_copied / 1e6:.1f} MB")
    typer.echo("  the tree is not copied; `sypy tree` rebuilds it")


@app.command()
def cache(
    forget: bool = typer.Option(
        False, "--forget", help="Throw the banked answers away."
    ),
    library_dir: Path = typer.Option(None, "--library", "-o", help="Library folder."),
) -> None:
    """Show the model answers this library has already paid for.

    An answer is kept against the document's contents, so a pass that dies
    between paying and filing does not buy the same one again when it restarts.
    `--forget` clears them, which costs money rather than losing anything: the
    next pass asks the model again.
    """
    settings = _settings(None, library_dir)
    with Library(settings.output_dir) as library:
        if forget:
            typer.echo(f"forgot {library.db.forget_model_answers()} banked answer(s)")
            return
        typer.echo(f"{library.db.count_model_answers()} banked answer(s)")

    days = env_int("SYP_LABEL_CACHE_DAYS", DEFAULT_LABEL_CACHE_DAYS)
    if days == 0:
        typer.echo("reuse is off (SYP_LABEL_CACHE_DAYS=0); every pass asks again")
    elif days < 0:
        typer.echo("kept indefinitely (SYP_LABEL_CACHE_DAYS is negative)")
    else:
        typer.echo(
            f"reused for {days} day(s), then asked again — a label is a choice "
            "made against the categories the library had at the time"
        )


@app.command()
def budget(
    reset: bool = typer.Option(False, "--reset", help="Forget the recorded spend."),
) -> None:
    """Show what the last 24 hours have cost at the API, against the ceiling."""
    ledger = Budget()
    if reset:
        ledger.reset()
        typer.echo("spend record cleared")
        return

    used = ledger.usage()
    limits = ledger.limits
    typer.echo(f"last 24 hours, from {ledger.path}")
    typer.echo(f"  requests  {used.requests:>8} / {_ceiling(limits.requests_per_day)}")
    typer.echo(f"  tokens    {used.tokens:>8} / {_ceiling(limits.tokens_per_day)}")
    if limits.unlimited:
        typer.echo(
            "\nboth ceilings are off. The watcher restarts on failure, so "
            "nothing here would stop a loop paying for the same folder."
        )
    else:
        typer.echo(
            "\nraise with SYP_MAX_REQUESTS_PER_DAY / SYP_MAX_TOKENS_PER_DAY; "
            "0 turns a ceiling off."
        )


def _ceiling(limit: int) -> str:
    return "unlimited" if limit <= 0 else str(limit)


@app.command("watch-target", hidden=True)
def watch_target(
    name: str = typer.Argument(None, help="Watch name; omit for the default."),
) -> None:
    """Print a watch's input and library, one per line.

    For the service script, so it reads the registry through the same code the
    rest of the tool does instead of parsing TOML in shell.
    """
    entry = _watch_entry(name)
    if entry is None:
        typer.echo(
            "no watch to install: declare one in "
            f"{registry_path()} (see `sypy watches`), or pass the folders",
            err=True,
        )
        raise typer.Exit(code=2)
    typer.echo(entry.input_dir)
    typer.echo(entry.library_dir)


@app.command()
def watches() -> None:
    """Show the declared watches, and which are running."""
    from .watchlock import locks_dir

    try:
        registry = load_registry()
    except RegistryError as err:
        typer.echo(f"error: {err}", err=True)
        raise typer.Exit(code=2) from err

    if not registry.watches:
        typer.echo(f"no watches declared in {registry.path}\n")
        typer.echo("Declare one:\n")
        typer.echo('  [watch.downloads]')
        typer.echo('  input   = "~/Downloads"')
        typer.echo('  library = "~/Documents/sypy-library"')
        typer.echo('  mode    = "copy"')
        return

    claimed = _claimed_folders(locks_dir())
    default = registry.default()
    for entry in registry.watches.values():
        marks = []
        if default is not None and entry.name == default.name:
            marks.append("default")
        holder = claimed.get(entry.input_dir) or claimed.get(entry.library_dir)
        marks.append(f"running pid {holder}" if holder else "stopped")
        typer.echo(f"{entry.name}  [{', '.join(marks)}]")
        typer.echo(f"    input   {entry.input_dir}")
        typer.echo(f"    library {entry.library_dir}")
        typer.echo(f"    mode    {entry.mode.value}")
    typer.echo(f"\n{registry.path}")


def _claimed_folders(directory: Path) -> dict[Path, int]:
    """Folders claimed by a watcher that is still running, by path."""
    from .watchlock import live_claims

    return live_claims(directory)


@app.command("list")
def list_papers(
    library_dir: Path = typer.Option(None, "--library", "-o", help="Library folder."),
    as_json: bool = typer.Option(
        False, "--json", help="Print records instead of a table, for a program to read."
    ),
) -> None:
    """List what the library holds."""
    settings = _settings(None, library_dir)
    with Library(settings.output_dir) as library:
        _report(library, library.db.all_papers(), as_json=as_json)


@app.command()
def find(
    query: str = typer.Argument(
        ..., help="Words to match against titles, authors, keywords, tags, and ids."
    ),
    library_dir: Path = typer.Option(None, "--library", "-o", help="Library folder."),
    limit: int = typer.Option(
        20, "--limit", "-n", min=0, help="Most to show; 0 for all."
    ),
    as_json: bool = typer.Option(
        False, "--json", help="Print records instead of a table, for a program to read."
    ),
) -> None:
    """Find documents by title, author, keyword, tag, year, or id.

    Every word has to match somewhere, so adding one narrows the result. The
    records `--json` prints carry the path to each document, which is how a
    reader gets from a search to the file itself.
    """
    settings = _settings(None, library_dir)
    with Library(settings.output_dir) as library:
        # One more than will be shown, which is how the cap is noticed. A
        # capped result that says nothing about it reads as the whole answer,
        # and the reader stops looking for the document that was cut off.
        papers = library.db.search(query, limit=limit + 1 if limit else None)
        capped = bool(limit) and len(papers) > limit
        _report(library, papers[:limit] if capped else papers, as_json=as_json)
        if capped:
            typer.echo(
                f"more than {limit} matched; narrow the search, "
                "or pass --limit 0 for all of them",
                err=True,
            )


def _report(library: Library, papers: Sequence[Paper], *, as_json: bool) -> None:
    """Show what was found, either to a person or to whatever called this."""
    if as_json:
        typer.echo(
            json.dumps([_describe(library, paper) for paper in papers], indent=2)
        )
        return
    for paper in papers:
        tags = " / ".join(paper.tags) or "-"
        typer.echo(f"{paper.file_id}  {tags:<40}  {paper.title or paper.original_name}")
    typer.echo(f"\n{len(papers)} document(s) in {library.root}")


def _describe(library: Library, paper: Paper) -> dict:
    """One document as a record, with the paths that lead to it.

    Paths are absolute because the caller is not standing anywhere in
    particular, and a record whose file cannot be opened from it is only half
    an answer.
    """
    return {
        "id": paper.file_id,
        "title": paper.title,
        "authors": paper.authors,
        "year": paper.year,
        "category": "/".join(paper.tags),
        "tags": paper.tags,
        "keywords": paper.keywords,
        "document": str(library.store_path(paper).resolve()),
        "folder": str(library.document_dir(paper).resolve()),
        "notes": str(library.note_path(paper).resolve()),
        "original_name": paper.original_name,
        "from_page_images": paper.from_page_images,
    }


def _watch_entry(name: str | None) -> WatchEntry | None:
    """The declared watch a command should act on, if the registry names one."""
    try:
        registry = load_registry()
    except RegistryError as err:
        typer.echo(f"error: {err}", err=True)
        raise typer.Exit(code=2) from err
    return registry.get(name) if name else registry.default()


def _resolved_library(library_dir: Path | None, watch_name: str | None = None) -> Path | None:
    """Where a command should look, when it was not told.

    CLI beats the environment beats the registry, which is the order the rest of
    the settings already resolve in.
    """
    if library_dir is not None or os.environ.get("SYP_OUTPUT"):
        return library_dir
    try:
        entry = _watch_entry(watch_name)
    except typer.Exit:
        raise
    return entry.library_dir if entry else None


def _settings(input_dir: Path | None, library_dir: Path | None, watch_name: str | None = None):
    try:
        return resolve_settings(input_dir, _resolved_library(library_dir, watch_name))
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
            _resolved_library(library_dir),
            recursive=recursive,
            page_cutoff=page_cutoff,
            keyword_batch_size=batch_size,
            model=model,
        )
        return settings, OpenAiClient(
            resolve_api_key(),
            settings.model,
            budget=Budget(),
            max_retries=env_int("SYP_LLM_MAX_RETRIES", DEFAULT_LLM_MAX_RETRIES),
            timeout_seconds=env_float(
                "SYP_LLM_TIMEOUT_SECONDS", DEFAULT_LLM_TIMEOUT_SECONDS
            ),
        )
    except ConfigError as err:
        typer.echo(f"error: {err}", err=True)
        raise typer.Exit(code=2) from err


def main() -> None:
    app()


if __name__ == "__main__":
    main()
