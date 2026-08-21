from __future__ import annotations

import pytest

from syp_prototype.naming import (
    MAX_NAME_CHARS,
    disambiguate,
    link_name,
    new_paper_id,
    parse_store_name,
    sanitize_tag,
    split_category,
    store_name,
)


def test_store_name_round_trips_id_and_tags() -> None:
    paper_id = new_paper_id()

    name = store_name(paper_id, ["Machine Learning", "Transformers"])

    assert name == f"{paper_id}__Machine Learning__Transformers.pdf"
    assert parse_store_name(name) == (paper_id, ["Machine Learning", "Transformers"])


def test_a_tag_can_never_contain_the_separator() -> None:
    # Underscores are what would break parsing, so they must not survive.
    assert "_" not in sanitize_tag("deep__learning")
    assert "_" not in sanitize_tag("a_b_c")

    paper_id = new_paper_id()
    name = store_name(paper_id, [sanitize_tag("deep__learning"), "Vision"])

    assert parse_store_name(name) == (paper_id, ["deep-learning", "Vision"])


def test_split_category_turns_a_model_path_into_ordered_tags() -> None:
    assert split_category("Machine Learning/Transformers") == [
        "Machine Learning",
        "Transformers",
    ]
    assert split_category("AI//  /Vision") == ["AI", "Vision"]


def test_tags_are_dropped_to_fit_the_filesystem_limit() -> None:
    paper_id = new_paper_id()
    tags = [f"Category number {index}" for index in range(40)]

    name = store_name(paper_id, tags)

    assert len(name) <= MAX_NAME_CHARS
    recovered_id, recovered = parse_store_name(name)
    assert recovered_id == paper_id
    # A prefix of the tags survives; the database holds the rest.
    assert recovered == tags[: len(recovered)]


def test_a_name_without_a_paper_id_is_rejected() -> None:
    with pytest.raises(ValueError):
        parse_store_name("not-an-id__AI.pdf")


def test_link_name_is_author_year_title() -> None:
    name = link_name(
        fallback="x.pdf",
        authors=["Ashish Vaswani", "Noam Shazeer"],
        year=2017,
        title="Attention Is All You Need",
    )

    assert name == "vaswani_2017_attention-is-all-you-need.pdf"


def test_link_name_handles_surname_first_authors_and_accents() -> None:
    name = link_name(
        fallback="x.pdf", authors=["Bengio, Yoshua"], year=2003, title="Neural Modèles"
    )

    assert name == "bengio_2003_neural-modeles.pdf"


@pytest.mark.parametrize(
    "authors,year,title",
    [([], 2017, "A Title"), (["A Author"], None, "A Title"), (["A Author"], 2017, "")],
)
def test_link_name_falls_back_to_the_store_name_when_a_piece_is_missing(
    authors, year, title
) -> None:
    assert (
        link_name(fallback="abc123__AI.pdf", authors=authors, year=year, title=title)
        == "abc123__AI.pdf"
    )


def test_disambiguate_keeps_the_suffix() -> None:
    assert disambiguate("a_2017_b.pdf", "deadbeef") == "a_2017_b_deadbeef.pdf"
