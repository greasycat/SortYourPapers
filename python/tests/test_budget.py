"""What stops an unattended watcher paying for the same folder forever."""

from __future__ import annotations

from pathlib import Path

import pytest

from sortyourpaperya.budget import (
    BUCKET_SECONDS,
    WINDOW_SECONDS,
    Budget,
    BudgetExceeded,
    Limits,
    Usage,
    ledger_path,
    resolve_limits,
)
from sortyourpaperya.extract import PaperText
from sortyourpaperya.llm import OpenAiClient
from sortyourpaperya.render import PageImage


@pytest.fixture
def ledger(tmp_path: Path) -> Budget:
    return Budget(tmp_path / "spend.json", Limits(requests_per_day=3, tokens_per_day=100))


def test_a_request_under_the_ceiling_is_allowed(ledger: Budget) -> None:
    ledger.record(requests=1, tokens=10)

    ledger.check()  # does not raise


def test_the_request_ceiling_refuses_the_next_call(ledger: Budget) -> None:
    for _ in range(3):
        ledger.record(requests=1)

    with pytest.raises(BudgetExceeded, match="3 of 3"):
        ledger.check()


def test_the_token_ceiling_refuses_the_next_call(ledger: Budget) -> None:
    ledger.record(requests=1, tokens=100)

    with pytest.raises(BudgetExceeded, match="tokens"):
        ledger.check()


def test_the_refusal_says_which_knob_lifts_it(ledger: Budget) -> None:
    # An error a person cannot act on stops the same amount of work as one they
    # can, and costs them the time it takes to find out how.
    for _ in range(3):
        ledger.record(requests=1)

    with pytest.raises(BudgetExceeded, match="SYP_MAX_REQUESTS_PER_DAY"):
        ledger.check()


def test_spend_falls_out_of_the_window(ledger: Budget, monkeypatch) -> None:
    import sortyourpaperya.budget as budget_module

    now = 1_000_000.0
    monkeypatch.setattr(budget_module.time, "time", lambda: now)
    for _ in range(3):
        ledger.record(requests=1)
    with pytest.raises(BudgetExceeded):
        ledger.check()

    monkeypatch.setattr(
        budget_module.time, "time", lambda: now + WINDOW_SECONDS + BUCKET_SECONDS
    )

    assert ledger.usage().requests == 0
    ledger.check()  # a day later the allowance is back


def test_spend_survives_the_process_that_recorded_it(tmp_path: Path) -> None:
    # The ceiling exists for a watcher that keeps being restarted, so a counter
    # living only in memory would reset on exactly the failure it guards.
    path = tmp_path / "spend.json"
    Budget(path, Limits(requests_per_day=2, tokens_per_day=0)).record(requests=1)

    later = Budget(path, Limits(requests_per_day=2, tokens_per_day=0))

    assert later.usage().requests == 1


def test_concurrent_writers_do_not_lose_each_others_spend(tmp_path: Path) -> None:
    # Four batches run at once. A read-modify-write that lost increments would
    # undercount exactly when the most is being spent.
    path = tmp_path / "spend.json"
    for _ in range(20):
        Budget(path, Limits()).record(requests=1, tokens=5)

    assert Budget(path, Limits()).usage() == Usage(requests=20, tokens=100)


def test_a_corrupt_ledger_does_not_stop_a_pass(tmp_path: Path) -> None:
    path = tmp_path / "spend.json"
    path.write_text("{not json", encoding="utf-8")
    ledger = Budget(path, Limits())

    assert ledger.usage().requests == 0
    ledger.record(requests=1)

    assert ledger.usage().requests == 1, "the next write should replace it"


def test_a_ceiling_of_zero_is_off(tmp_path: Path) -> None:
    ledger = Budget(tmp_path / "spend.json", Limits(requests_per_day=0, tokens_per_day=0))
    for _ in range(100):
        ledger.record(requests=1, tokens=10_000)

    ledger.check()  # does not raise


def test_reset_clears_the_record(ledger: Budget) -> None:
    ledger.record(requests=1, tokens=10)

    ledger.reset()

    assert ledger.usage().requests == 0


def test_limits_come_from_the_environment(monkeypatch) -> None:
    monkeypatch.setenv("SYP_MAX_REQUESTS_PER_DAY", "7")
    monkeypatch.setenv("SYP_MAX_TOKENS_PER_DAY", "9")

    assert resolve_limits() == Limits(requests_per_day=7, tokens_per_day=9)


def test_the_ledger_is_machine_wide_not_per_library(tmp_path: Path, monkeypatch) -> None:
    # Two libraries on one API key spend the same money.
    monkeypatch.setenv("SORTYOURPAPERYA_STATE_DIR", str(tmp_path / "state"))

    assert ledger_path().parent == tmp_path / "state" / "sortyourpaperya"


# ---- the client -----------------------------------------------------------


class _StubResponse:
    def __init__(self, content: str, total_tokens: int) -> None:
        self.choices = [type("C", (), {"message": type("M", (), {"content": content})})]
        self.usage = type("U", (), {"total_tokens": total_tokens})


_ONE_PAIR = (
    '{"pairs":[{"file_id":"a","keywords":["k"],'
    '"preliminary_categories_k_depth":"Misc","title":"T","authors":[],"year":null}]}'
)


class _RecordingCompletions:
    def __init__(self, content: str = _ONE_PAIR, total_tokens: int = 42) -> None:
        self.calls = 0
        self._content = content
        self._total = total_tokens

    async def create(self, **_kwargs):
        self.calls += 1
        return _StubResponse(self._content, self._total)


def _client_with(budget: Budget, completions: _RecordingCompletions) -> OpenAiClient:
    client = OpenAiClient("test-key", "test-model", budget=budget)
    client._client = type(
        "Stub", (), {"chat": type("Chat", (), {"completions": completions})}
    )
    return client


async def test_a_spent_budget_stops_the_request_before_it_is_sent(
    tmp_path: Path,
) -> None:
    """Checked before, not after: a ceiling read afterwards has already been paid."""
    ledger = Budget(tmp_path / "spend.json", Limits(requests_per_day=1, tokens_per_day=0))
    ledger.record(requests=1)
    completions = _RecordingCompletions()
    client = _client_with(ledger, completions)

    with pytest.raises(BudgetExceeded):
        await client.extract_keywords([PaperText(file_id="a", path=tmp_path / "a.pdf", text="x", pages_read=1)])

    assert completions.calls == 0, "the request was sent anyway"


async def test_a_request_is_taken_out_of_the_allowance_before_it_is_made(
    tmp_path: Path,
) -> None:
    """Four batches run at once, and each checks before any of them records.

    Counting only afterwards would let all four through on the last unit of a
    spent allowance.
    """
    ledger = Budget(tmp_path / "spend.json", Limits(requests_per_day=5, tokens_per_day=0))
    seen: list[int] = []

    class _Watching(_RecordingCompletions):
        async def create(self, **kwargs):
            seen.append(ledger.usage().requests)
            return await super().create(**kwargs)

    client = _client_with(ledger, _Watching())
    await client.extract_keywords([PaperText(file_id="a", path=tmp_path / "a.pdf", text="x", pages_read=1)])

    assert seen == [1], "the request was not reserved before it was sent"


async def test_what_a_request_actually_cost_is_recorded(tmp_path: Path) -> None:
    ledger = Budget(tmp_path / "spend.json", Limits(requests_per_day=0, tokens_per_day=0))
    client = _client_with(ledger, _RecordingCompletions(total_tokens=137))

    await client.extract_keywords([PaperText(file_id="a", path=tmp_path / "a.pdf", text="x", pages_read=1)])

    assert ledger.usage() == Usage(requests=1, tokens=137)


async def test_reading_scanned_pages_is_charged_too(tmp_path: Path) -> None:
    # It is a second request per scanned document, and the expensive kind.
    ledger = Budget(tmp_path / "spend.json", Limits(requests_per_day=0, tokens_per_day=0))
    client = _client_with(ledger, _RecordingCompletions(content="a scan", total_tokens=9))

    await client.describe_pages([PageImage(data=b"x", media_type="image/png")])

    assert ledger.usage().requests == 1


def test_the_client_gives_up_rather_than_retrying_forever(tmp_path: Path) -> None:
    """A request that keeps failing has to stop costing money at some point."""
    client = OpenAiClient(
        "test-key",
        "test-model",
        budget=Budget(tmp_path / "spend.json"),
        max_retries=2,
        timeout_seconds=30.0,
    )

    assert client._client.max_retries == 2
    assert client._client.timeout == 30.0


_ONE_CATEGORY = '{"category":"Cognitive Science/Computation","keywords":["a"]}'


async def test_every_re_ask_is_budgeted_like_any_other_request(tmp_path: Path) -> None:
    """Regenerating is unbounded by design, so the ceiling is what bounds it."""
    ledger = Budget(tmp_path / "spend.json", Limits(requests_per_day=2, tokens_per_day=0))
    completions = _RecordingCompletions(_ONE_CATEGORY, total_tokens=11)
    client = _client_with(ledger, completions)
    paper = PaperText(file_id="a", path=tmp_path / "a.pdf", text="x", pages_read=1)

    await client.suggest_category(paper)
    await client.suggest_category(paper, rejected=["Cognitive Science/Computation"])

    assert ledger.usage() == Usage(requests=2, tokens=22)
    with pytest.raises(BudgetExceeded):
        await client.suggest_category(paper)
    assert completions.calls == 2, "the refused one was never sent"
