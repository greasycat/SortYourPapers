from __future__ import annotations

from pathlib import Path

from conftest import FailingLlmClient, FakeLlmClient, write_pdf, write_scanned_pdf

from syp_prototype.config import Settings
from syp_prototype.ingest import ingest_folder
from syp_prototype.store import INGEST_INDEX_FILE, IngestIndex


def _index(settings: Settings) -> IngestIndex:
    return IngestIndex(settings.output_dir / INGEST_INDEX_FILE)


async def test_ingests_every_pending_pdf(settings: Settings) -> None:
    write_pdf(settings.input_dir / "a.pdf", "attention transformers")
    write_pdf(settings.input_dir / "b.pdf", "genome sequencing")
    client = FakeLlmClient()

    report = await ingest_folder(settings, client, _index(settings))

    assert report.processed == 2
    assert {Path(record.path).name for record in report.ingested} == {"a.pdf", "b.pdf"}
    assert all(record.preliminary_category for record in report.ingested)


async def test_batches_respect_the_configured_size(settings: Settings) -> None:
    for index in range(5):
        write_pdf(settings.input_dir / f"p{index}.pdf", f"paper {index}")
    client = FakeLlmClient()

    await ingest_folder(settings, client, _index(settings))

    # batch size 2 over 5 papers
    assert [len(batch) for batch in client.batches] == [2, 2, 1]


async def test_a_second_pass_costs_no_model_calls(settings: Settings) -> None:
    write_pdf(settings.input_dir / "a.pdf", "attention")
    first_client = FakeLlmClient()
    await ingest_folder(settings, first_client, _index(settings))

    second_client = FakeLlmClient()
    report = await ingest_folder(settings, second_client, _index(settings))

    assert second_client.batches == []
    assert report.processed == 0
    assert report.skipped_already_ingested == 1


async def test_a_renamed_paper_is_recognised_as_already_ingested(
    settings: Settings,
) -> None:
    original = write_pdf(settings.input_dir / "a.pdf", "attention")
    await ingest_folder(settings, FakeLlmClient(), _index(settings))
    original.rename(settings.input_dir / "renamed.pdf")

    client = FakeLlmClient()
    report = await ingest_folder(settings, client, _index(settings))

    assert client.batches == []
    assert report.skipped_already_ingested == 1


async def test_the_library_is_never_reingested(settings: Settings) -> None:
    write_pdf(settings.output_dir / "AI" / "filed.pdf", "already filed")
    write_pdf(settings.input_dir / "pending.pdf", "pending")
    settings = Settings(
        input_dir=settings.input_dir,
        output_dir=settings.output_dir,
        recursive=True,
        keyword_batch_size=settings.keyword_batch_size,
    )
    client = FakeLlmClient()

    report = await ingest_folder(settings, client, _index(settings))

    assert [Path(record.path).name for record in report.ingested] == ["pending.pdf"]


async def test_a_scanned_pdf_is_reported_not_sent_to_the_model(
    settings: Settings,
) -> None:
    write_scanned_pdf(settings.input_dir / "scan.pdf")
    client = FakeLlmClient()

    report = await ingest_folder(settings, client, _index(settings))

    assert client.batches == []
    assert len(report.failed) == 1
    assert "no text layer" in report.failed[0][1]


async def test_oversized_files_are_skipped_not_failed(settings: Settings) -> None:
    big = settings.input_dir / "big.pdf"
    write_pdf(big, "big")
    big.write_bytes(big.read_bytes() + b"\x00" * (17 * 1024 * 1024))

    report = await ingest_folder(settings, FakeLlmClient(), _index(settings))

    assert [path.name for path in report.skipped_oversized] == ["big.pdf"]
    assert report.failed == []


async def test_a_failed_batch_does_not_discard_the_batches_that_worked(
    settings: Settings,
) -> None:
    write_pdf(settings.input_dir / "a.pdf", "a")
    client = FailingLlmClient()

    report = await ingest_folder(settings, client, _index(settings))

    assert report.processed == 0
    assert len(report.failed) == 1
    assert "rate limited" in report.failed[0][1]
    # Nothing was recorded, so the next pass retries rather than skipping.
    assert len(_index(settings)) == 0


async def test_progress_is_flushed_per_paper_so_an_interrupt_keeps_it(
    settings: Settings,
) -> None:
    write_pdf(settings.input_dir / "a.pdf", "a")
    await ingest_folder(settings, FakeLlmClient(), _index(settings))

    reloaded = _index(settings)

    assert len(reloaded) == 1
    assert reloaded.records[0].keywords == ["alpha", "beta"]
