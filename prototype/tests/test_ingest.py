from __future__ import annotations

from pathlib import Path

from conftest import FailingLlmClient, FakeLlmClient, write_pdf, write_scanned_pdf

from syp_prototype.config import Settings
from syp_prototype.ingest import ingest_folder
from syp_prototype.library import FilingMode, Library


async def test_ingests_every_pending_pdf(settings: Settings, library: Library) -> None:
    write_pdf(settings.input_dir / "a.pdf", "attention transformers")
    write_pdf(settings.input_dir / "b.pdf", "genome sequencing")
    client = FakeLlmClient()

    report = await ingest_folder(settings, client, library, mode=FilingMode.MOVE)

    assert report.processed == 2
    assert {filing.source.name for filing in report.filed} == {"a.pdf", "b.pdf"}
    assert all(filing.store_path.is_file() for filing in report.filed)


async def test_batches_respect_the_configured_size(settings: Settings, library: Library) -> None:
    for index in range(5):
        write_pdf(settings.input_dir / f"p{index}.pdf", f"paper {index}")
    client = FakeLlmClient()

    await ingest_folder(settings, client, library, mode=FilingMode.MOVE)

    # batch size 2 over 5 papers
    assert [len(batch) for batch in client.batches] == [2, 2, 1]


async def test_filing_empties_the_inbox_so_a_second_pass_has_nothing_to_do(
    settings: Settings, library: Library
) -> None:
    write_pdf(settings.input_dir / "a.pdf", "attention")
    await ingest_folder(settings, FakeLlmClient(), library, mode=FilingMode.MOVE)

    second_client = FakeLlmClient()
    report = await ingest_folder(settings, second_client, library, mode=FilingMode.MOVE)

    assert second_client.batches == []
    assert report.processed == 0


async def test_the_same_content_arriving_again_costs_no_model_call(
    settings: Settings, library: Library
) -> None:
    source = write_pdf(settings.input_dir / "a.pdf", "attention")
    content = source.read_bytes()
    await ingest_folder(settings, FakeLlmClient(), library, mode=FilingMode.MOVE)

    # The same paper downloaded a second time, under a different name.
    (settings.input_dir / "a-copy.pdf").write_bytes(content)
    client = FakeLlmClient()
    report = await ingest_folder(settings, client, library, mode=FilingMode.MOVE)

    assert client.batches == []
    assert report.skipped_already_known == 1
    assert library.db.count() == 1


async def test_identity_survives_renaming_because_it_is_content_addressed(
    settings: Settings, library: Library
) -> None:
    source = write_pdf(settings.input_dir / "a.pdf", "attention")
    content = source.read_bytes()
    await ingest_folder(settings, FakeLlmClient(), library, mode=FilingMode.MOVE)
    stored_id = library.db.all_papers()[0].file_id

    (settings.input_dir / "totally-different-name.pdf").write_bytes(content)
    await ingest_folder(settings, FakeLlmClient(), library, mode=FilingMode.MOVE)

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

    report = await ingest_folder(settings, client, library, mode=FilingMode.MOVE)

    assert [filing.source.name for filing in report.filed] == ["pending.pdf"]


async def test_a_scanned_pdf_is_reported_not_sent_to_the_model(
    settings: Settings, library: Library
) -> None:
    write_scanned_pdf(settings.input_dir / "scan.pdf")
    client = FakeLlmClient()

    report = await ingest_folder(settings, client, library, mode=FilingMode.MOVE)

    assert client.batches == []
    assert len(report.failed) == 1
    assert "no text layer" in report.failed[0][1]


async def test_oversized_files_are_skipped_not_failed(settings: Settings, library: Library) -> None:
    big = settings.input_dir / "big.pdf"
    write_pdf(big, "big")
    big.write_bytes(big.read_bytes() + b"\x00" * (17 * 1024 * 1024))

    report = await ingest_folder(settings, FakeLlmClient(), library, mode=FilingMode.MOVE)

    assert [path.name for path in report.skipped_oversized] == ["big.pdf"]
    assert report.failed == []


async def test_a_failed_batch_does_not_discard_the_batches_that_worked(
    settings: Settings, library: Library
) -> None:
    write_pdf(settings.input_dir / "a.pdf", "a")
    client = FailingLlmClient()

    report = await ingest_folder(settings, client, library, mode=FilingMode.MOVE)

    assert report.processed == 0
    assert len(report.failed) == 1
    assert "rate limited" in report.failed[0][1]
    # Nothing was recorded, so the next pass retries rather than skipping.
    assert library.db.count() == 0


async def test_everything_known_about_a_paper_lands_in_the_database(
    settings: Settings, library: Library
) -> None:
    write_pdf(settings.input_dir / "a.pdf", "a")

    await ingest_folder(settings, FakeLlmClient(), library, mode=FilingMode.MOVE)

    papers = library.db.all_papers()
    assert len(papers) == 1
    paper = papers[0]
    assert paper.tags == ["AI", "Transformers"]
    assert paper.authors == ["Ashish Vaswani"]
    assert paper.year == 2017
    assert paper.title == "Attention Is All You Need"
    assert sorted(paper.keywords) == ["alpha", "beta"]
    assert paper.original_name == "a.pdf"


async def test_a_preview_run_writes_nothing(settings: Settings, library: Library) -> None:
    source = write_pdf(settings.input_dir / "a.pdf", "a")

    report = await ingest_folder(settings, FakeLlmClient(), library, mode=FilingMode.PREVIEW)

    assert len(report.planned) == 1
    assert report.filed == []
    assert source.exists(), "preview must leave the input folder untouched"
    assert library.db.count() == 0
    assert not library.store_dir.exists()


async def test_filed_papers_land_in_the_store_and_the_tree(
    settings: Settings, library: Library
) -> None:
    write_pdf(settings.input_dir / "a.pdf", "a")

    report = await ingest_folder(settings, FakeLlmClient(), library, mode=FilingMode.MOVE)

    filing = report.filed[0]
    assert filing.store_path.parent == library.store_dir
    assert "__AI__Transformers.pdf" in filing.store_path.name
    assert filing.link_path.is_symlink()
    assert filing.link_path.resolve() == filing.store_path.resolve()


async def test_copy_mode_leaves_the_source_folder_untouched(
    settings: Settings, library: Library
) -> None:
    # The point of copy mode: index a folder someone else owns without
    # rearranging it underneath them.
    source = write_pdf(settings.input_dir / "a.pdf", "attention")
    before = source.read_bytes()

    report = await ingest_folder(
        settings, FakeLlmClient(), library, mode=FilingMode.COPY
    )

    filing = report.filed[0]
    assert source.exists(), "copy mode must not remove the source"
    assert source.read_bytes() == before
    assert filing.store_path.is_file()
    assert filing.store_path.read_bytes() == before
    assert filing.link_path.is_symlink()


async def test_copying_the_same_folder_twice_costs_one_model_call(
    settings: Settings, library: Library
) -> None:
    # In copy mode the source never drains, so re-scanning must be free or a
    # watched folder would pay for the same documents on every pass.
    write_pdf(settings.input_dir / "a.pdf", "attention")
    first = FakeLlmClient()
    await ingest_folder(settings, first, library, mode=FilingMode.COPY)

    second = FakeLlmClient()
    report = await ingest_folder(settings, second, library, mode=FilingMode.COPY)

    assert len(first.batches) == 1
    assert second.batches == [], "the second pass must not call the model"
    assert report.skipped_already_known == 1
    assert library.db.count() == 1


async def test_move_mode_works_across_filesystems(
    settings: Settings, library: Library, monkeypatch
) -> None:
    # os.replace refuses a cross-device move; shutil.move falls back to a copy.
    import shutil as _shutil

    write_pdf(settings.input_dir / "a.pdf", "attention")
    real_move = _shutil.move
    calls: list[str] = []

    def tracking_move(src, dst, *args, **kwargs):
        calls.append(str(src))
        return real_move(src, dst, *args, **kwargs)

    monkeypatch.setattr("syp_prototype.library.shutil.move", tracking_move)
    report = await ingest_folder(
        settings, FakeLlmClient(), library, mode=FilingMode.MOVE
    )

    assert calls, "move mode must go through shutil.move, not os.replace"
    assert report.filed[0].store_path.is_file()
