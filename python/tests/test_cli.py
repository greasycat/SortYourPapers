from pathlib import Path

from typer.testing import CliRunner

from syp_paperfetch.cli import app
from syp_paperfetch.models import CuratedTestSet, SamplingPolicy


def test_build_manifest_cli(monkeypatch, tmp_path: Path) -> None:
    runner = CliRunner()

    monkeypatch.setattr("syp_paperfetch.cli.load_candidates", lambda source: [])
    monkeypatch.setattr(
        "syp_paperfetch.cli.build_curated_test_set",
        lambda candidates, policy: CuratedTestSet(
            set_id="demo",
            description="Demo",
            source_dataset="OpenMOSS-Team/SciJudgeBench",
            selection_policy=SamplingPolicy(),
            generated_at_ms=1,
            papers=[],
        ),
    )

    output = tmp_path / "manifest.toml"
    result = runner.invoke(app, ["build-manifest", "--output", str(output)])

    assert result.exit_code == 0
    assert output.exists()
