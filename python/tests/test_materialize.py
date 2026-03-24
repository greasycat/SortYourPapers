from __future__ import annotations

import subprocess
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path
from threading import Thread

from syp_paperfetch.materialize import export_test_set, materialize_test_set
from syp_paperfetch.models import CuratedPaper, CuratedTestSet, SamplingPolicy, SelectionBucket


class _PdfHandler(BaseHTTPRequestHandler):
    body = b"%PDF-demo"

    def do_GET(self) -> None:  # noqa: N802
        self.send_response(200)
        self.send_header("Content-Type", "application/pdf")
        self.send_header("Content-Length", str(len(self.body)))
        self.end_headers()
        self.wfile.write(self.body)

    def log_message(self, format: str, *args: object) -> None:  # noqa: A003
        return


def test_materialize_and_export(tmp_path: Path) -> None:
    server = HTTPServer(("127.0.0.1", 0), _PdfHandler)
    thread = Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        test_set = CuratedTestSet(
            set_id="demo",
            description="Demo",
            source_dataset="OpenMOSS-Team/SciJudgeBench",
            selection_policy=SamplingPolicy(),
            generated_at_ms=1,
            papers=[
                CuratedPaper(
                    paper_id="arxiv-1234.5678",
                    arxiv_id="1234.5678",
                    title="Title",
                    category="CS",
                    subcategory="cs.AI",
                    citations=10,
                    date="2024-01-01",
                    abstract_excerpt="Excerpt",
                    selection_bucket=SelectionBucket.TOP,
                    paper_url="https://arxiv.org/abs/1234.5678",
                    pdf_url=f"http://127.0.0.1:{server.server_port}/paper.pdf",
                    source_splits=["train"],
                )
            ],
        )

        report = materialize_test_set(test_set, cache_root=tmp_path / "cache")
        exported = export_test_set(report, tmp_path / "out")

        assert report.papers[0].path.exists()
        assert exported[0].exists()
        assert (report.cache_dir / "manifest.toml").exists()
        assert (report.cache_dir / "manifest.json").exists()
    finally:
        server.shutdown()
        thread.join()


def test_materialize_falls_back_to_rendered_html_pdf(
    monkeypatch, tmp_path: Path
) -> None:
    class _LegacyHandler(BaseHTTPRequestHandler):
        def do_GET(self) -> None:  # noqa: N802
            if self.path == "/paper.pdf":
                self.send_response(404)
                self.end_headers()
                return
            if self.path == "/html/paper/" or self.path == "/paper":
                body = b"<html><body><h1>Legacy Paper</h1></body></html>"
                self.send_response(200)
                self.send_header("Content-Type", "text/html; charset=utf-8")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)
                return
            self.send_response(404)
            self.end_headers()

        def log_message(self, format: str, *args: object) -> None:  # noqa: A003
            return

    server = HTTPServer(("127.0.0.1", 0), _LegacyHandler)
    thread = Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        monkeypatch.setattr("syp_paperfetch.materialize._find_pdf_browser", lambda: "chromium")

        def fake_run(args: list[str], **_: object) -> subprocess.CompletedProcess[str]:
            output_arg = next(value for value in args if value.startswith("--print-to-pdf="))
            output_path = Path(output_arg.split("=", 1)[1])
            output_path.write_bytes(b"%PDF-legacy")
            return subprocess.CompletedProcess(args=args, returncode=0)

        monkeypatch.setattr("syp_paperfetch.materialize.subprocess.run", fake_run)

        test_set = CuratedTestSet(
            set_id="legacy",
            description="Legacy",
            source_dataset="OpenMOSS-Team/SciJudgeBench",
            selection_policy=SamplingPolicy(),
            generated_at_ms=1,
            papers=[
                CuratedPaper(
                    paper_id="arxiv-cs-9908001",
                    arxiv_id="cs/9908001",
                    title="Legacy Title",
                    category="CS",
                    subcategory="cs.CL",
                    citations=1,
                    date="1999-08-01",
                    abstract_excerpt="Excerpt",
                    selection_bucket=SelectionBucket.RANDOM,
                    paper_url=f"http://127.0.0.1:{server.server_port}/paper",
                    pdf_url=f"http://127.0.0.1:{server.server_port}/paper.pdf",
                    source_splits=["train"],
                )
            ],
        )

        report = materialize_test_set(test_set, cache_root=tmp_path / "cache")

        assert report.papers[0].path.exists()
        assert report.papers[0].path.read_bytes() == b"%PDF-legacy"
    finally:
        server.shutdown()
        thread.join()
