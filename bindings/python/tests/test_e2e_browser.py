"""End-to-end tests for the Python bindings against a real bundled Chromium.

Covers the full browser surface (navigation / perception / actions / extract /
cookies+storage / tabs / screenshots / PDF / download / request interception)
**and** accessibility (the engine AX tree, plus the feature-gated
accessibility *surface* tools `ax_*` when the wheel was built with `surface`).

A local HTTP server provides deterministic, offline pages on a real origin (so
cookies and localStorage behave normally). Skips cleanly when no bundled
Chromium is available, so it never blocks browser-less environments.
"""

import base64
import http.server
import json
import os
import socketserver
import threading
import time

import pytest

import fastbrowser as fb

REPO_ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "..", "..", ".."))

PAGE = b"""<!doctype html><html lang="en"><head><meta charset="utf-8"><title>AX E2E</title></head><body>
<h1>Accessibility Test</h1>
<nav aria-label="Main navigation"><a id="home" href="#one">Home Link</a></nav>
<form>
  <label for="name">Name</label><input id="name" type="text" placeholder="Enter name">
  <input id="search" type="search" aria-label="Site search" placeholder="Search site">
  <input id="agree" type="checkbox"><label for="agree">I agree</label>
  <button id="submit" type="button" onclick="document.title='Clicked'">Submit Form</button>
  <select id="color"><option value="r">Red</option><option value="g">Green</option></select>
  <textarea id="notes"></textarea>
</form>
<img id="logo" alt="Company logo" src="data:image/gif;base64,R0lGODlhAQABAAAAACwAAAAAAQABAAA=">
<table id="t"><tr><th>Name</th><th>Age</th></tr><tr><td>Alice</td><td>30</td></tr></table>
<div style="height:2000px">Tall block</div>
</body></html>"""

BLOCKED_JS = b"window.__blockedLoaded = true;"
PAGE_BLOCKED = b"""<!doctype html><title>Blocked</title><script src="/blocked.js?cb=1"></script><p>x</p>"""
API = b'{"real": 1}'
PAGE_API = b"""<!doctype html><title>Api</title><script>
window.__api = 'pending';
fetch('/api.json?cb=' + Date.now()).then(r => r.json()).then(d => { window.__api = d; })
  .catch(e => { window.__api = 'err:' + e.name; });
</script>"""
ABORT_JSON = b'{"abort": 1}'
PAGE_ABORT = b"""<!doctype html><title>Abort</title><script>
window.__abort = 'pending';
fetch('/abort.json?cb=' + Date.now()).then(() => { window.__abort = 'ok'; })
  .catch(e => { window.__abort = 'err:' + e.name; });
</script>"""
BIN = bytes(range(256)) * 8

ROUTES = {
    "/": (PAGE, "text/html"),
    "/blocked.js": (BLOCKED_JS, "application/javascript"),
    "/page_blocked.html": (PAGE_BLOCKED, "text/html"),
    "/api.json": (API, "application/json"),
    "/page_api.html": (PAGE_API, "text/html"),
    "/abort.json": (ABORT_JSON, "application/json"),
    "/page_abort.html": (PAGE_ABORT, "text/html"),
    "/data.bin": (BIN, "application/octet-stream"),
}


class _Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        path = self.path.split("?")[0]
        body, ctype = ROUTES.get(path, (b"<title>404</title>", "text/html"))
        self.send_response(200)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *args):
        pass


@pytest.fixture(scope="module")
def base_url():
    httpd = socketserver.TCPServer(("127.0.0.1", 0), _Handler)
    port = httpd.server_address[1]
    threading.Thread(target=httpd.serve_forever, daemon=True).start()
    try:
        yield f"http://127.0.0.1:{port}"
    finally:
        httpd.shutdown()


@pytest.fixture(scope="module")
def browser(base_url):
    if os.path.exists(os.path.join(REPO_ROOT, "vendor/chromium")):
        os.environ.setdefault(
            "CHROME_PATH",
            os.path.join(
                REPO_ROOT,
                "vendor/chromium/chrome-mac-arm64/Google Chrome for Testing.app/"
                "Contents/MacOS/Google Chrome for Testing",
            ),
        )
    b = fb.FastBrowser()
    try:
        b.init({"engine": "bundled", "command_timeout_ms": 20000})
        b.open(base_url + "/")  # establish the active tab
    except Exception as e:  # no bundled chromium → skip the whole module
        try:
            b.shutdown()
        except Exception:
            pass
        pytest.skip(f"bundled chromium unavailable: {e}")
    try:
        yield b
    finally:
        b.shutdown()


def _has_tool(b, name):
    return any(t.get("name") == name for t in b.tool_list())


def _js(b, script):
    return b.tool_call("execute_js", {"script": script}).get("result")


def _walk(node, fn):
    if not isinstance(node, dict):
        return
    fn(node)
    for c in node.get("children") or []:
        _walk(c, fn)


def _find(tree, role):
    hits = []
    _walk(tree, lambda n: hits.append(n) if n.get("role") == role and n.get("ref") else None)
    return hits[0] if hits else None


# ── navigation / perception ──────────────────────────────────────


def test_navigation_and_text(browser, base_url):
    browser.navigate(base_url + "/")
    assert browser.tool_call("get_page_title")["title"] == "AX E2E"
    assert browser.tool_call("get_current_url")["url"].startswith(base_url)
    assert "Accessibility Test" in browser.tool_call("get_page_text")["text"]


def test_snapshot_and_find(browser, base_url):
    browser.navigate(base_url + "/")
    snap = browser.snapshot()
    tags = {e.get("tag") for e in snap["interactive"]}
    assert {"button", "input"} <= tags, tags
    hit = browser.tool_call("find_elements", {"role": "button", "name": "Submit Form"})
    assert hit["count"] >= 1


# ── selector dialects (Playwright / Testing Library / ARIA) ──────


def test_selector_dialects(browser, base_url):
    browser.navigate(base_url + "/")

    def count(sel):
        return browser.tool_call("find_elements", {"selector": sel})["count"]

    # Implicit ARIA roles (were previously only explicit role= / a tiny map).
    assert count("role=checkbox") >= 1, "role=checkbox (implicit input[type=checkbox])"
    assert count("role=searchbox") >= 1, "role=searchbox (implicit input[type=search])"
    assert count("role=heading") >= 1, "role=heading (implicit h1)"
    # getByRole-style role + accessible-name filter.
    assert count("role=button[name='Submit Form']") >= 1, "role=button[name=...]"
    # label= / placeholder= dialects (Testing Library getByLabel/Placeholder).
    assert count("label=Name") >= 1, "label="
    assert count("placeholder=Enter name") >= 1, "placeholder="


def test_find_elements_never_exposes_sentinel_id(browser, base_url):
    browser.navigate(base_url + "/")
    els = browser.tool_call("find_elements", {"selector": "button"})["elements"]
    assert els, "expected at least one button"
    for e in els:
        assert e.get("id") != "\u0001" and e.get("ref") != "\u0001", e


# ── accessibility: engine AX tree ────────────────────────────────


def test_accessibility_tree(browser, base_url):
    browser.navigate(base_url + "/")
    tree = browser.tool_call("get_accessibility_tree", {"max_nodes": 300})
    roles = []
    _walk(tree.get("root"), lambda n: roles.append(n.get("role")))
    assert "heading" in roles, roles
    assert any(r in roles for r in ("button", "textbox", "checkbox")), roles


# ── actions ──────────────────────────────────────────────────────


def test_actions(browser, base_url):
    browser.navigate(base_url + "/")
    snap = browser.snapshot()
    btn = next(e for e in snap["interactive"] if e.get("tag") == "button")
    browser.tool_call("click", {"id": btn["id"]})
    time.sleep(0.3)
    assert browser.tool_call("get_page_title")["title"] == "Clicked"

    inp = next(e for e in snap["interactive"] if e.get("tag") == "input")
    browser.tool_call("type", {"id": inp["id"], "text": "Alice Chen"})
    assert _js(browser, "document.getElementById('name').value") == "Alice Chen"

    browser.tool_call("press", {"key": "Tab"})
    assert browser.tool_call("scroll", {"dy": 300})["ok"] is True
    assert browser.tool_call("hover", {"selector": "#submit"})["ok"] is True


# ── extract ──────────────────────────────────────────────────────


def test_extract(browser, base_url):
    browser.navigate(base_url + "/")
    assert "Accessibility Test" in browser.tool_call("extract_text", {})["text"]
    links = browser.tool_call("extract_links", {})["links"]
    assert any(lk.get("text") == "Home Link" for lk in links), links
    assert browser.tool_call("extract_table", {})["table"] == [["Name", "Age"], ["Alice", "30"]]
    assert browser.tool_call("extract_forms", {})["count"] >= 1
    assert browser.tool_call("extract_images", {"all": True})["count"] >= 1


# ── cookies / storage ────────────────────────────────────────────


def test_cookies_and_storage(browser, base_url):
    browser.navigate(base_url + "/")
    browser.tool_call("cookie_set", {"name": "e2e", "value": "1"})
    assert "e2e" in json.dumps(browser.tool_call("cookie_get", {}))
    browser.tool_call("storage_set", {"key": "k", "value": "v"})
    assert "v" in json.dumps(browser.tool_call("storage_get", {"key": "k"}))


# ── wait / viewport / screenshot ─────────────────────────────────


def test_wait_viewport_screenshot(browser, base_url):
    browser.navigate(base_url + "/")
    assert browser.tool_call("wait_for_element", {"selector": "#submit", "timeout_ms": 3000})["found"]
    browser.set_viewport(1024, 768)
    png = browser.screenshot_png()
    assert png[:8] == b"\x89PNG\r\n\x1a\n"


# ── tabs ─────────────────────────────────────────────────────────


def test_tabs(browser, base_url):
    first = browser.tool_call("list_tabs", {})["tabs"][0]["id"]
    browser.tool_call("new_tab", {"url": base_url + "/?2"})
    ids = [t["id"] for t in browser.tool_call("list_tabs", {})["tabs"]]
    assert len(ids) >= 2, ids
    browser.tool_call("switch_tab", {"tab": first})
    assert browser.tool_call("get_active_tab", {})["tab"] == first


# ── download ─────────────────────────────────────────────────────


def test_download(browser, base_url, tmp_path):
    dest = tmp_path / "data.bin"
    out = browser.tool_call("download", {"url": base_url + "/data.bin", "path": str(dest)})
    assert out["ok"] is True
    assert dest.read_bytes() == BIN


# ── PDF export ───────────────────────────────────────────────────


def test_save_as_pdf(browser, base_url, tmp_path):
    browser.navigate(base_url + "/")
    dest = tmp_path / "page.pdf"
    out = browser.tool_call("save_as_pdf", {"path": str(dest)})
    assert out["ok"] is True
    assert dest.read_bytes()[:5] == b"%PDF-"


# ── request interception ─────────────────────────────────────────


def test_block_request(browser, base_url):
    browser.navigate(base_url + "/")
    try:
        browser.tool_call("block_request", {"patterns": ["*blocked.js*"], "enabled": True})
        # navigate the *same* tab (open() would create a new one)
        browser.navigate(base_url + f"/page_blocked.html?t={time.time()}")
        assert _js(browser, "typeof window.__blockedLoaded") == "undefined"
    finally:
        browser.tool_call("block_request", {"patterns": ["*blocked.js*"], "enabled": False})
    browser.navigate(base_url + f"/page_blocked.html?t={time.time()}")
    assert _js(browser, "typeof window.__blockedLoaded") == "boolean"


def test_intercept_and_fulfill(browser, base_url):
    browser.navigate(base_url + "/")
    try:
        browser.tool_call("intercept_request", {"patterns": ["*api.json*"], "enabled": True})
        browser.navigate(base_url + f"/page_api.html?t={time.time()}")
        rid = None
        for _ in range(50):
            reqs = browser.tool_call("list_pending_requests", {})["requests"]
            hit = [r for r in reqs if "api.json" in str(r.get("url", ""))]
            if hit:
                rid = hit[0]["request_id"]
                break
            time.sleep(0.1)
        assert rid, "intercepted request was not paused/listed"
        body = base64.b64encode(b'{"mocked": true}').decode()
        browser.tool_call("fulfill_request", {"request_id": rid, "status": 200, "body_b64": body})
        for _ in range(50):
            v = _js(browser, "JSON.stringify(window.__api || null)")
            if v not in (None, "null", '"pending"'):
                break
            time.sleep(0.1)
        assert v == '{"mocked":true}', v
    finally:
        browser.tool_call("intercept_request", {"patterns": ["*api.json*"], "enabled": False})


def test_intercept_and_abort(browser, base_url):
    browser.navigate(base_url + "/")
    try:
        browser.tool_call("intercept_request", {"patterns": ["*abort.json*"], "enabled": True})
        browser.navigate(base_url + f"/page_abort.html?t={time.time()}")
        rid = None
        for _ in range(50):
            reqs = browser.tool_call("list_pending_requests", {})["requests"]
            hit = [r for r in reqs if "abort.json" in str(r.get("url", ""))]
            if hit:
                rid = hit[0]["request_id"]
                break
            time.sleep(0.1)
        assert rid, "intercepted request was not paused/listed"
        browser.tool_call("abort_request", {"request_id": rid})
        for _ in range(50):
            v = _js(browser, "window.__abort || null")
            if v and v != "pending":
                break
            time.sleep(0.1)
        assert str(v).startswith("err:"), v
    finally:
        browser.tool_call("intercept_request", {"patterns": ["*abort.json*"], "enabled": False})


# ── accessibility SURFACE (ax_*), feature-gated ──────────────────


def test_surface_accessibility(browser, base_url):
    if not _has_tool(browser, "ax_list"):
        pytest.skip("wheel built without the `surface` feature (no ax_* tools)")
    browser.navigate(base_url + "/")

    listing = browser.tool_call("ax_list", {})
    ids = [s.get("id") for s in listing["surfaces"]]
    assert any(str(i).startswith("web:") for i in ids), ids

    snap = browser.tool_call("ax_snapshot", {})
    assert str((snap.get("surface") or {}).get("id", "")).startswith("web:")
    roles = []
    _walk(snap.get("tree"), lambda n: roles.append(n.get("role")))
    assert "button" in roles and "textbox" in roles, roles

    box = _find(snap.get("tree"), "checkbox")
    if box:
        # Playwright-MCP style `@ref` prefix must be normalized away (B1).
        assert browser.tool_call(
            "ax_act", {"ref": "@" + box["ref"], "action": "check", "checked": True}
        )["ok"]
    btn = _find(snap.get("tree"), "button")
    if btn:
        assert browser.tool_call("ax_click", {"ref": btn["ref"]})["ok"]

    # Surface-level scroll must actually move the document (B5).
    before = int(_js(browser, "window.scrollY || 0") or 0)
    assert browser.tool_call("ax_scroll", {"dy": 400})["ok"] is True
    after = int(_js(browser, "window.scrollY || 0") or 0)
    assert after > before, (before, after)
    assert "events" in browser.tool_call("ax_events", {})
    # text fallback: works on engines without a focused keyboard channel
    assert browser.tool_call("ax_type", {"selector": "#notes", "text": "hello"})["ok"]
