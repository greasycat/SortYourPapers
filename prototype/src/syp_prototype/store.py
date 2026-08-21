"""The ingest index: what has been read, so it is never paid for twice.

Kept as JSON Lines in the library folder. Appending one line per paper means an
interrupted run keeps everything it had already finished, which is what lets the
watcher recover by simply running again.
"""

from __future__ import annotations

import json
from dataclasses import asdict, dataclass
from pathlib import Path

INGEST_INDEX_FILE = "ingest.jsonl"


@dataclass(frozen=True)
class IngestRecord:
    """One paper, as read."""

    file_id: str
    path: str
    keywords: list[str]
    preliminary_category: str
    pages_read: int
    chars_extracted: int


class IngestIndex:
    """Append-only record of ingested papers, keyed by content id."""

    def __init__(self, path: Path) -> None:
        self.path = path
        self._records: dict[str, IngestRecord] = {}
        self._load()

    def _load(self) -> None:
        if not self.path.is_file():
            return
        for line in self.path.read_text(encoding="utf-8").splitlines():
            line = line.strip()
            if not line:
                continue
            try:
                payload = json.loads(line)
                record = IngestRecord(**payload)
            except (json.JSONDecodeError, TypeError):
                # A half-written final line is the expected shape of a crash;
                # skip it rather than refusing to start.
                continue
            self._records[record.file_id] = record

    def __contains__(self, file_id: object) -> bool:
        return file_id in self._records

    def __len__(self) -> int:
        return len(self._records)

    @property
    def records(self) -> list[IngestRecord]:
        return list(self._records.values())

    def add(self, record: IngestRecord) -> None:
        """Record one paper and flush it, so progress survives an interrupt."""
        self._records[record.file_id] = record
        self.path.parent.mkdir(parents=True, exist_ok=True)
        with self.path.open("a", encoding="utf-8") as handle:
            handle.write(json.dumps(asdict(record), ensure_ascii=False) + "\n")
