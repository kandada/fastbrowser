"""CLI tests for the fastbrowser package.

Covers argument parsing, local tool listing, and daemon lifecycle with the
``mock`` engine (no Chromium required). Skips the daemon test when
``FASTBROWSER_NO_DAEMON_TEST`` is set.
"""

import json
import os
import subprocess
import sys
import time

import pytest

from fastbrowser import cli


def _run(*argv, timeout=60):
    return subprocess.run(
        [sys.executable, "-m", "fastbrowser", *argv],
        capture_output=True,
        text=True,
        timeout=timeout,
    )


def test_parser_accepts_engine_after_subcommand():
    args = cli.build_parser().parse_args(["open", "https://x", "--engine", "mock"])
    assert args.command == "open"
    assert args.engine == "mock"


def test_parser_ax_and_daemon_are_positional():
    a = cli.build_parser().parse_args(["ax", "snapshot", "--target", "desktop:1"])
    assert a.ax_action == "snapshot" and a.target == "desktop:1"
    d = cli.build_parser().parse_args(["daemon", "start", "--engine", "mock"])
    assert d.daemon_cmd == "start" and d.engine == "mock"
    c = cli.build_parser().parse_args(["call", "navigate", "--args", '{"url":"u"}'])
    assert c.name == "navigate"


def test_tools_names_local_mock():
    args = cli.build_parser().parse_args(["tools", "--mode", "names"])
    out = cli._cmd_tools(args)
    assert out["count"] >= 100
    assert "ax_snapshot" in out["tools"] and "navigate" in out["tools"]


def test_tools_help_local_mock():
    args = cli.build_parser().parse_args(["tools", "--mode", "help", "--name", "ax_snapshot"])
    tool = cli._cmd_tools(args)
    assert tool["name"] == "ax_snapshot"


def test_cli_main_tools_prints_json(capsys):
    rc = cli.main(["tools", "--mode", "names"])
    assert rc == 0
    payload = json.loads(capsys.readouterr().out)
    assert payload["count"] >= 100


@pytest.mark.skipif(
    os.environ.get("FASTBROWSER_NO_DAEMON_TEST") == "1", reason="daemon test disabled"
)
def test_daemon_lifecycle_and_stateful_call_mock():
    # Clean any stale daemon from a previous run.
    _run("daemon", "stop", timeout=20)
    start = _run("daemon", "start", "--engine", "mock", timeout=40)
    assert start.returncode == 0, start.stderr
    assert json.loads(start.stdout)["started"] is True
    try:
        # poll status until running
        running = False
        for _ in range(50):
            st = _run("daemon", "status", timeout=20)
            if st.returncode == 0 and json.loads(st.stdout).get("running"):
                running = True
                break
            time.sleep(0.1)
        assert running, "daemon did not report running"
        # a stateful call routes through the daemon
        tabs = _run("call", "list_tabs", timeout=30)
        assert tabs.returncode == 0
        assert "tabs" in json.loads(tabs.stdout)
    finally:
        stop = _run("daemon", "stop", timeout=20)
        assert stop.returncode == 0
