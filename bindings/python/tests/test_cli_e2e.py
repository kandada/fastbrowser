"""End-to-end tests for the fastbrowser CLI.

Mock/ephemeral/daemon/transport checks run anywhere (no Chromium, no network).
The real-engine checks are skipped unless a real browser AND network are
available (or when ``FASTBROWSER_NO_E2E=1``).

Each test drives the CLI as a subprocess: ``python -m fastbrowser ...``.
"""

import json
import os
import socket
import subprocess
import sys
import tempfile
import time

import pytest

import fastbrowser
from fastbrowser import daemon as fbd

# Isolate the daemon discovery file from the user's real one.
os.environ.setdefault("FASTBROWSER_STATE_DIR", tempfile.mkdtemp(prefix="fb-cli-e2e-"))

NO_DAEMON = os.environ.get("FASTBROWSER_NO_DAEMON_TEST") == "1"
NO_E2E = os.environ.get("FASTBROWSER_NO_E2E") == "1"


# ── helpers ────────────────────────────────────────────────────────────────


def run(*argv, timeout=120):
    return subprocess.run(
        [sys.executable, "-m", "fastbrowser", *argv],
        capture_output=True,
        text=True,
        timeout=timeout,
    )


def as_json(proc):
    try:
        return json.loads(proc.stdout)
    except Exception:
        return None


def stop_daemon():
    try:
        run("daemon", "stop", timeout=20)
    except Exception:
        pass


def state():
    try:
        return json.loads(fbd._state_file().read_text())
    except Exception:
        return None


def _network_ok():
    try:
        socket.create_connection(("example.com", 443), timeout=3).close()
        return True
    except OSError:
        return False


# ── meta / tools / edge (no browser, no daemon) ─────────────────────────────


class TestCliMeta:
    def test_version(self):
        p = run("--version")
        assert p.returncode == 0
        assert fastbrowser.__version__ in p.stdout

    def test_help_lists_commands(self):
        p = run("--help")
        assert p.returncode == 0
        for c in ("tools", "fetch", "run", "ax", "daemon", "serve"):
            assert c in p.stdout

    def test_no_subcommand_is_error(self):
        assert run().returncode != 0

    def test_unknown_command_is_error(self):
        assert run("nope").returncode != 0


class TestCliTools:
    def test_names(self):
        d = as_json(run("tools", "--mode", "names"))
        assert d and d["count"] >= 100
        assert "ax_snapshot" in d["tools"] and "navigate" in d["tools"]

    def test_compact(self):
        d = as_json(run("tools", "--mode", "compact"))
        assert d and isinstance(d["tools"][0]["params"], list)

    def test_full(self):
        d = as_json(run("tools", "--mode", "full"))
        assert d and isinstance(d["tools"][0].get("params"), dict)

    def test_help_named(self):
        d = as_json(run("tools", "--mode", "help", "--name", "ax_snapshot"))
        assert d and d["name"] == "ax_snapshot"

    def test_help_without_name_is_error(self):
        assert run("tools", "--mode", "help").returncode != 0

    def test_bad_mode_is_error(self):
        assert run("tools", "--mode", "bogus").returncode != 0

    def test_run_without_steps_is_error(self):
        assert run("run").returncode != 0

    def test_daemon_stop_when_none_is_ok(self):
        d = as_json(run("daemon", "stop"))
        assert d and d["ok"] is True


# ── daemon + stateful commands (mock engine) ────────────────────────────────


@pytest.mark.skipif(NO_DAEMON, reason="daemon tests disabled")
class TestCliDaemonMock:
    @pytest.fixture(scope="class", autouse=True)
    def daemon(self):
        stop_daemon()
        p = run("daemon", "start", "--engine", "mock", timeout=40)
        assert p.returncode == 0, p.stderr
        assert (as_json(p) or {}).get("started") is True
        yield
        stop_daemon()

    def test_status_running(self):
        d = as_json(run("daemon", "status"))
        assert d and d.get("running") is True

    def test_discovery_has_both_transports(self):
        s = state()
        assert s and s.get("socket") and s.get("port")

    def test_tcp_transport_ping(self):
        s = state()
        sock = socket.create_connection(("127.0.0.1", int(s["port"])), timeout=5)
        try:
            sock.sendall(
                (
                    json.dumps({"token": s["token"], "cmd": "ping", "args": {}}) + "\n"
                ).encode()
            )
            buf = b""
            while not buf.endswith(b"\n"):
                chunk = sock.recv(4096)
                if not chunk:
                    break
                buf += chunk
        finally:
            sock.close()
        r = json.loads(buf.decode())
        assert r.get("ok") and r["result"].get("pong")

    def test_open_and_call(self):
        assert (as_json(run("open", "https://example.com")) or {}).get(
            "title"
        ) is not None
        d = as_json(run("call", "get_page_title"))
        assert d and "title" in d

    def test_navigate(self):
        d = as_json(run("navigate", "https://example.com/x"))
        assert d and "url" in d

    def test_info_status_snapshot_tabs(self):
        assert "engine" in (as_json(run("info")) or {})
        assert isinstance(as_json(run("status")), dict)
        assert isinstance(as_json(run("snapshot")), dict)
        assert "tabs" in (as_json(run("tabs")) or {})

    def test_screenshot_writes_file(self, tmp_path):
        out = tmp_path / "shot.png"
        d = as_json(run("screenshot", str(out)))
        assert d and d.get("path") and out.exists() and out.stat().st_size > 0

    def test_ax_list_and_snapshot(self):
        d = as_json(run("ax", "list"))
        assert d and "surfaces" in d
        d = as_json(run("ax", "snapshot"))
        assert d and ("surface" in d or "tree" in d)

    def test_call_unknown_tool_errors(self):
        p = run("call", "no_such_tool_xyz")
        assert p.returncode != 0
        assert "error" in (as_json(p) or {})


# ── real engine (auto -> Chrome); skips without browser/network ─────────────


@pytest.mark.skipif(NO_E2E, reason="real-engine e2e disabled")
class TestCliReal:
    @pytest.fixture(scope="class", autouse=True)
    def daemon(self):
        if not _network_ok():
            pytest.skip("no network")
        stop_daemon()
        p = run("daemon", "start", timeout=90)  # engine auto
        assert p.returncode == 0, p.stderr
        info = as_json(run("info", timeout=30)) or {}
        if (info.get("engine") or "mock") == "mock":
            stop_daemon()
            pytest.skip("no real browser available (engine fell back to mock)")
        yield
        stop_daemon()

    def test_fetch(self):
        d = as_json(
            run("fetch", "https://example.com", "--max-chars", "200", timeout=120)
        )
        assert d and d.get("success") and "Example" in (d.get("title") or "")

    def test_run_steps(self):
        d = as_json(
            run(
                "run",
                "--url",
                "https://example.com",
                "--steps",
                '[{"tool":"get_page_title","args":{}},{"tool":"extract_text","args":{"max_chars":60}}]',
                timeout=120,
            )
        )
        assert d and d.get("success") and len(d.get("steps", [])) == 2

    def test_open_and_title(self):
        d = as_json(run("open", "https://example.com", timeout=120))
        assert d and "example.com" in (d.get("url") or "")
        d = as_json(run("call", "get_page_title", timeout=120))
        assert d and d.get("title") == "Example Domain"

    def test_ax_snapshot_web(self):
        d = as_json(run("ax", "snapshot", timeout=120)) or {}
        assert str((d.get("surface") or {}).get("id", "")).startswith("web:")

    def test_extract_text(self):
        d = (
            as_json(
                run("call", "extract_text", "--args", '{"max_chars":200}', timeout=120)
            )
            or {}
        )
        assert "domain" in (d.get("text") or "").lower()
