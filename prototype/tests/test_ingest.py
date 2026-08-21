from __future__ import annotations

from pathlib import Path

from conftest import FailingLlmClient, FakeLlmClient, write_pdf, write_scanned_pdf

from syp_prototype.config import Settings
from syp_prototype.ingest import ingest_folder
from syp_prototype.library import Library


async def test_ingests_every_pending_pdf(settings: Settings, library: Library) -> None:
    write_pdf(settings.input_dir / "a.pdf", "attention transformers")
    write_pdf(settings.input_dir / "b.pdf", "genome sequencing")
    client = FakeLlmClient()

    report = await ingest_folder(settings, client, library, apply=True)

    assert report.processed == 2
    assert {filing.source.name for filing in report.filed} == {"a.pdf", "b.pdf"}
    assert all(filing.store_path.is_file() for filing in report.filed)


async def test_batches_respect_the_configured_size(settings: Settings, library: Library) -> None:
    for index in range(5):
        write_pdf(settings.input_dir / f"p{index}.pdf", f"paper {index}")
    client = FakeLlmClient()

    await ingest_folder(settings, client, library, apply=True)

    # batch size 2 over 5 papers
    assert [len(batch) for batch in client.batches] == [2, 2, 1]


async def test_filing_empties_the_inbox_so_a_second_pass_has_nothing_to_do(
    settings: Settings, library: Library
) -> None:
    write_pdf(settings.input_dir / "a.pdf", "attention")
    await ingest_folder(settings, FakeLlmClient(), library, apply=True)

    second_client = FakeLlmClient()
    report = await ingest_folder(settings, second_client, library, apply=True)

    assert second_client.batches == []
    assert report.processed == 0


async def test_the_same_content_arriving_again_costs_no_model_call(
    settings: Settings, library: Library
) -> None:
    source = write_pdf(settings.input_dir / "a.pdf", "attention")
    content = source.read_bytes()
    await ingest_folder(settings, FakeLlmClient(), library, apply=True)

    # The same paper downloaded a second time, under a different name.
    (settings.input_dir / "a-copy.pdf").write_bytes(content)
    client = FakeLlmClient()
    report = await ingest_folder(settings, client, library, apply=True)

    assert client.batches == []
    assert report.skipped_already_known == 1
    assert library.db.count() == 1


async def test_identity_survives_renaming_because_it_is_content_addressed(
    settings: Settings, library: Library
) -> None:
    source = write_pdf(settings.input_dir / "a.pdf", "attention")
    content = source.read_bytes()
    await ingest_folder(settings, FakeLlmClient(), library, apply=True)
    stored_id = library.db.all_papers()[0].file_id

    (settings.input_dir / "totally-different-name.pdf").write_bytes(content)
    await ingest_folder(settings, FakeLlmClient(), library, apply=True)

    assert [paper.file_id for paper in library.db.all_papers()] == [stored_id]


async def test_the_library_is_never_reingested(settings: Settings, library: Library) -> None:
    write_pdf(settings.output_dir / "AI" / "filed.pdf", "already filed")
    write_pdf(settings.input_dir / "pending.pdf", "pending")
    settings = Settings(
        input_dir=settings.input_dir,
        output_dir=settings.output_dir,
        recursive=True,
        keyword_batch_size=settings.keyword_batch_size,
    )
    client = FakeLlmClient()

    report = await ingest_folder(settings, client, library, apply=True)

    assert [filing.source.name for filing in report.filed] == ["pending.pdf"]


async def test_a_scanned_pdf_is_reported_not_sent_to_the_model(
    settings: Settings, library: Library
) -> None:
    write_scanned_pdf(settings.input_dir / "scan.pdf")
    client = FakeLlmClient()

    report = await ingest_folder(settings, client, library, apply=True)

    assert client.batches == []
    assert len(report.failed) == 1
    assert "no text layer" in report.failed[0][1]


async def test_oversized_files_are_skipped_not_failed(settings: Settings, library: Library) -> None:
    big = settings.input_dir / "big.pdf"
    write_pdf(big, "big")
    big.write_bytes(big.read_bytes() + b"\x00" * (17 * 1024 * 1024))

    report = await ingest_folder(settings, FakeLlmClient(), library, apply=True)

    assert [path.name for path in report.skipped_oversized] == ["big.pdf"]
    assert report.failed == []


async def test_a_failed_batch_does_not_discard_the_batches_that_worked(
    settings: Settings, library: Library
) -> None:
    write_pdf(settings.input_dir / "a.pdf", "a")
    client = FailingLlmClient()

    report = await ingest_folder(settings, client, library, apply=True)

    assert report.processed == 0
    assert len(report.failed) == 1
    assert "rate limited" in report.failed[0][1]
    # Nothing was recorded, so the next pass retries rather than skipping.
    assert library.db.count() == 0


async def test_everything_known_about_a_paper_lands_in_the_database(
    settings: Settings, library: Library
) -> None:
    write_pdf(settings.input_dir / "a.pdf", "a")

    await ingest_folder(settings, FakeLlmClient(), library, apply=True)

    papers = library.db.all_papers()
    assert len(papers) == 1
    paper = papers[0]
    assert paper.tags == ["AI", "Transformers"]
    assert paper.authors == ["Ashish Vaswani"]
    assert paper.year == 2017
    assert paper.title == "Attention Is All You Need"
    assert sorted(paper.keywords) == ["alpha", "beta"]
    assert paper.original_name == "a.pdf"


async def test_a_preview_run_moves_nothing(settings: Settings, library: Library) -> None:
    source = write_pdf(settings.input_dir / "a.pdf", "a")

    report = await ingest_folder(settings, FakeLlmClient(), library, apply=False)

    assert len(report.planned) == 1
    assert report.filed == []
    assert source.exists(), "preview must leave the input folder untouched"
    assert library.db.count() == 0
    assert not library.store_dir.exists()


async def test_filed_papers_land_in_the_store_and_the_tree(
    settings: Settings, library: Library
) -> None:
    write_pdf(settings.input_dir / "a.pdf", "a")

    report = await ingest_folder(settings, FakeLlmClient(), library, apply=True)

    filing = report.filed[0]
    assert filing.store_path.parent == library.store_dir
    assert "__AI__Transformers.pdf" in filing.store_path.name
    assert filing.link_path.is_symlink()
    assert filing.link_path.resolve() == filing.store_path.resolve()
