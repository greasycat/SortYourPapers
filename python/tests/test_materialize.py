from __future__ import annotations

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
                    canonical_pdf_url=f"http://127.0.0.1:{server.server_port}/paper.pdf",
                    title="Title",
                    category="CS",
                    subcategory="cs.AI",
                    citations=10,
                    date="2024-01-01",
                    abstract_excerpt="Excerpt",
                    selection_bucket=SelectionBucket.TOP,
                )
            ],
        )

        report = materialize_test_set(test_set, cache_root=tmp_path / "cache")
        exported = export_test_set(report, tmp_path / "out")

        assert report.papers[0].path.exists()
        assert exported[0].exists()
    finally:
        server.shutdown()
        thread.join()
