---
name: sortyourpapers
description: Search and read a SortYourPapers library — the documents `sypy` has filed, with their categories, authors, years, and notes. Use when asked to find, look up, read, cite, summarise, compare, re-tag, or take notes on documents the user has filed, or when a question is about "my papers", "my library", "that bill", "the manual", or a document they say they already have.
---

# Reading a SortYourPapers library

A library is a folder of filed documents. Every document lives in exactly one
folder in `store/`, holding the document itself and anything kept beside it;
`tree/` is a browsable view of symlinks over that, rebuilt on demand. A DuckDB
database is the source of truth for what is filed and how it is labelled.

Reach it through `sypy`, never by walking the folders. The database knows
titles, authors, years, keywords, and categories that the filenames do not.

## Find the document

```bash
sypy find "attention transformers" --json
```

Every word has to match, somewhere in the id, title, original filename, year,
tag, author, or keyword — so adding a word narrows and removing one widens. If
a search comes back empty, drop the most specific word and try again rather
than concluding the library does not have it.

Twenty results are shown by default. When more matched, `find` says so on
stderr — read it, because the rest are documents the library holds and this
search did not show you. Add a word to narrow, or pass `--limit 0` for all of
them.

Each record looks like this:

```json
{
  "id": "5112ee75ddcf",
  "title": "Attention Is All You Need",
  "authors": ["Ashish Vaswani"],
  "year": 2017,
  "category": "Machine Learning/Deep Learning/Transformers",
  "tags": ["Machine Learning", "Deep Learning", "Transformers"],
  "keywords": ["attention", "sequence modelling"],
  "document": "/…/store/5112ee75ddcf__…/vaswani_2017_attention-is-all-you-need.pdf",
  "folder": "/…/store/5112ee75ddcf__…",
  "notes": "/…/store/5112ee75ddcf__…/notes.md",
  "original_name": "1706.03762v7.pdf",
  "from_page_images": false
}
```

`document` is the file — open it to read the document. `id` is what every
other command takes. `from_page_images: true` means the title, authors and year
were a model's reading of a scan rather than the document's own words, so treat
them as approximate and say so if they matter.

`sypy list --json` gives every document the same way. Prefer `find`: a large
library is a lot to read, and the whole point of the labels is not having to.

Both commands, and the ones below, take `--library <path>` when the user names a
library. Without it `sypy` resolves the one this machine watches, which is
usually right.

## Take notes on it

```bash
sypy note 5112ee75ddcf --path
```

Prints the path to the document's `notes.md`, creating it with a heading if it
does not exist, and prints nothing else. Write or append to that file directly.

Always pass `--path`. Without it the command opens `$EDITOR`, which will hang.

Notes belong in the document's `folder`, beside the document — anything written
there is backed up with it, follows it when it is re-tagged, and is deleted with
it. Nothing written into `tree/` is durable: it is rebuilt from the database and
not backed up.

## Re-file a document

```bash
sypy retag 5112ee75ddcf "Medicine/Radiology"
```

Only when the user asks, or when they agree a document is filed wrongly. It
renames the document's folder and moves its link; nothing is copied and no
model is called. Look at `sypy list --json` first to see what categories the
library already uses, so a re-tag joins an existing branch instead of opening a
near-duplicate of one.

## What not to run

- **`sypy ingest` and `sypy watch` file new documents, and both send every
  document's text to OpenAI and cost money.** Never run either to answer a
  question. Run them only when the user asks for documents to be filed, and use
  `--mode copy` unless they ask for `move`, which drains the source folder.
- **`sypy remove` deletes the document, its notes, and its record.** When the
  document arrived by move, that is the only copy. Ask first, every time; pass
  `--yes` only after the user has said yes to that document.
- `sypy fsck`, `scan`, `tree`, `migrate-store`, and `backup` are maintenance.
  They are safe, but run them when asked, not speculatively.

## When something goes wrong

- A command that hangs for a few seconds is waiting on the database lock — a
  watcher may be mid-pass. It waits up to 30 seconds, then fails.
- `sypy fsck` reports a document whose file is gone, or a folder the database
  does not know about. `--adopt` brings such a folder back in, losing the
  title, authors, and year, which only ever lived in the database.
- A document the user is sure they filed, that `find` cannot see, is worth one
  `sypy fsck` before saying it is not there.
