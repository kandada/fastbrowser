"""Regression tests for the 0.1.4 additions through the Python binding.

Covers the new `download` tool (binary-safe fetch to a host path) and the
Playwright MCP / browser-use alias compatibility resolved by the kernel.
Mock engine, no browser required.
"""

import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

import pytest

import fastbrowser as fb

PNG = bytes([0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0xFF, 0x80])


class _Handler(BaseHTTPRequestHandler):
    def do_GET(self):  # noqa: N802
        if self.path == "/pic.png":
            body, ctype, status = PNG, "image/png", 200
        elif self.path == "/missing":
            body, ctype, status = b"nf", "text/plain", 404
        else:
            body, ctype, status = b"ok", "text/plain", 200
        self.send_response(status)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *args):  # keep the test output quiet
        pass


@pytest.fixture()
def base_url():
    server = ThreadingHTTPServer(("127.0.0.1", 0), _Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield f"http://127.0.0.1:{server.server_address[1]}"
    finally:
        server.shutdown()
        server.server_close()


def make():
    b = fb.FastBrowser()
    b.init()
    return b


def test_download_writes_binary_verbatim(base_url, tmp_path):
    b = make()
    dest = tmp_path / "pic.png"
    out = b.tool_call("download", {"url": f"{base_url}/pic.png", "path": str(dest)})
    assert out["ok"] is True, out
    assert out["bytes"] == len(PNG)
    assert out["content_type"] == "image/png"
    assert dest.read_bytes() == PNG, "file must be byte-identical"
    b.shutdown()


def test_download_http_error_reports_status(base_url, tmp_path):
    b = make()
    dest = tmp_path / "missing.bin"
    out = b.tool_call("download", {"url": f"{base_url}/missing", "path": str(dest)})
    assert out["ok"] is False, out
    assert out["status"] == 404
    assert not dest.exists(), "a failed download must not create the file"
    b.shutdown()


def test_download_is_registered():
    b = make()
    names = {t["name"] for t in b.tool_list()}
    assert "download" in names
    b.shutdown()


def test_playwright_and_browser_use_aliases():
    b = make()
    b.open("https://example.com")

    # Playwright MCP tool names.
    assert b.tool_call("browser_navigate", {"url": "https://example.com/pw"})[
        "url"
    ].endswith("/pw")
    assert b.tool_call("browser_snapshot")["count"] >= 1
    assert b.tool_call("browser_click", {"id": "a"})["clicked"] == "a"

    # browser-use tool names.
    assert b.tool_call("go_to_url", {"url": "https://example.com/bu"})[
        "url"
    ].endswith("/bu")
    assert b.tool_call("extract_content")["text"]

    # Parameter alias: `expression` → `script`.
    out = b.tool_call("execute_js", {"expression": "document.title"})
    assert "Example" in out["result"]
    b.shutdown()
