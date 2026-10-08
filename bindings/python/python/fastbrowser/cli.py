"""fastbrowser command-line interface.

Usage: ``python -m fastbrowser <command> [options]`` (or the installed
``fastbrowser`` console script).

Two execution models:

* **Ephemeral** (``tools`` / ``fetch`` / ``run`` / ``download``): a throwaway
  browser in this process — no daemon, nothing left behind.
* **Stateful** (``open`` / ``navigate`` / ``call`` / ``ax`` / ``snapshot`` /
  ``screenshot`` / ``tabs`` / ``info`` / ``status``): served by a background
  **daemon** that keeps one browser alive, so cookies / tabs / the current page
  persist across separate CLI invocations. The daemon is auto-started on demand
  and can be managed with ``serve`` / ``daemon <start|stop|status>``.

All output is JSON on stdout.
"""

from __future__ import annotations

import argparse
import json
import sys
import time
from typing import Any, Dict, List, Optional

from . import __version__
from . import daemon as _daemon


# ── helpers ────────────────────────────────────────────────────────────────


def _print(obj: Any) -> None:
    print(json.dumps(obj, ensure_ascii=False, indent=2))


def _ephemeral(engine: Optional[str], cdp_url: Optional[str], headed: bool):
    from . import FastBrowser

    cfg: Dict[str, Any] = {
        "engine": engine or "auto",
        "surface": {"enabled": True, "provider": "auto", "prefer_ax_tree": True},
    }
    if cdp_url:
        cfg["cdp_url"] = cdp_url
    if headed:
        cfg["rendering_mode"] = "hosted"
    b = FastBrowser()
    b.init(cfg)
    return b


def _daemon_call(args, cmd: str, payload: dict, local_fn=None) -> Any:
    """Route a stateful command to the daemon (or run ``local_fn`` with --local)."""
    if getattr(args, "local", False) and local_fn is not None:
        return local_fn()
    r = _daemon.send(
        cmd,
        payload,
        ensure=True,
        timeout=getattr(args, "timeout", 300),
        engine=getattr(args, "engine", None),
        cdp_url=getattr(args, "cdp_url", None),
        headed=getattr(args, "headed", False),
    )
    if not r.get("ok"):
        raise RuntimeError(r.get("error", "daemon error"))
    return r.get("result")


# ── ephemeral commands ──────────────────────────────────────────────────────


def _cmd_tools(args) -> Any:
    # Tool specs are engine-independent, so listing works with a cheap mock
    # engine (no Chromium / no daemon needed).
    from . import FastBrowser

    b = FastBrowser()
    b.init({"engine": "mock"})
    try:
        tools: List[dict] = b.tool_list()
    finally:
        b.shutdown()
    mode = args.mode
    if mode == "help":
        if not args.name:
            raise SystemExit("tools --mode help requires --name")
        for t in tools:
            if t.get("name") == args.name:
                return t
        raise SystemExit(f"unknown tool '{args.name}'")
    if mode == "names":
        return {"count": len(tools), "tools": [t.get("name") for t in tools]}
    if mode == "compact":
        return {
            "count": len(tools),
            "tools": [
                {
                    "name": t.get("name"),
                    "description": (t.get("description") or "").splitlines()[0][:80],
                    "params": list((t.get("params") or {}).keys()),
                }
                for t in tools
            ],
        }
    return {"count": len(tools), "tools": tools}


def _cmd_fetch(args) -> Any:
    b = _ephemeral(args.engine, args.cdp_url, args.headed)
    try:
        opened = b.open(args.url)
        tab = opened.get("tab", 0)
        for state in ("networkidle", "load"):
            try:
                b.tool_call("wait_for_load_state", {"tab": tab, "state": state, "timeout_ms": 8000})
            except Exception:  # noqa: BLE001
                pass
        text = ""
        for _ in range(12):
            time.sleep(0.3)
            tv = b.tool_call("extract_text", {"tab": tab, "max_chars": args.max_chars})
            text = (tv.get("text") or "").strip()
            if text:
                break
        title = b.tool_call("get_page_title", {"tab": tab}).get("title", "")
        try:
            b.tool_call("close_tab", {"tab": tab})
        except Exception:  # noqa: BLE001
            pass
        return {
            "success": bool(text),
            "url": args.url,
            "title": title,
            "content": text[: args.max_chars],
            "engine": (b.get_info() or {}).get("engine"),
        }
    finally:
        b.shutdown()


def _load_steps(args) -> List[dict]:
    if args.steps:
        raw = args.steps
    elif args.steps_file:
        with open(args.steps_file, "r", encoding="utf-8") as f:
            raw = f.read()
    else:
        raise SystemExit("run requires --steps '<json>' or --steps-file <path>")
    steps = json.loads(raw)
    if not isinstance(steps, list):
        raise SystemExit("--steps must be a JSON array of {\"tool\": name, \"args\": {...}}")
    return steps


def _cmd_run(args) -> Any:
    steps = _load_steps(args)
    b = _ephemeral(args.engine, args.cdp_url, args.headed)
    results: List[dict] = []
    try:
        if args.url:
            b.open(args.url)
        for s in steps:
            name = s.get("tool") or s.get("name")
            targs = s.get("args") or {}
            try:
                results.append({"tool": name, "ok": True, "result": b.tool_call(name, targs)})
            except Exception as e:  # noqa: BLE001
                results.append({"tool": name, "ok": False, "error": str(e)})
        return {"success": all(r["ok"] for r in results) if results else True, "steps": results}
    finally:
        b.shutdown()


def _cmd_download(args) -> Any:
    headers = {}
    for h in args.header or []:
        if ":" in h:
            k, v = h.split(":", 1)
            headers[k.strip()] = v.strip()
    params: Dict[str, Any] = {"url": args.url, "path": args.path}
    if args.referer:
        params["referer"] = args.referer
    if headers:
        params["headers"] = headers
    b = _ephemeral("mock", None, False)  # download is a standalone tool; no engine needed
    try:
        return b.tool_call("download", params)
    finally:
        b.shutdown()


# ── stateful (daemon) commands ──────────────────────────────────────────────


def _cmd_open(args) -> Any:
    return _daemon_call(
        args,
        "open",
        {"url": args.url},
        local_fn=lambda: _ephemeral(args.engine, args.cdp_url, args.headed).open(args.url),
    )


def _cmd_navigate(args) -> Any:
    return _daemon_call(args, "tool", {"name": "navigate", "args": {"url": args.url}})


def _cmd_call(args) -> Any:
    targs = json.loads(args.args) if args.args else {}
    return _daemon_call(
        args,
        "tool",
        {"name": args.name, "args": targs},
        local_fn=lambda: _ephemeral(args.engine, args.cdp_url, args.headed).tool_call(
            args.name, targs
        ),
    )


def _cmd_ax(args) -> Any:
    if args.ax_action == "list":
        return _daemon_call(args, "tool", {"name": "ax_list", "args": {}})
    if args.ax_action == "snapshot":
        params = {"target": args.target} if args.target else {}
        return _daemon_call(args, "tool", {"name": "ax_snapshot", "args": params})
    # ax act
    params: Dict[str, Any] = {"action": args.action}
    for key in ("ref", "target", "text", "value", "key"):
        val = getattr(args, key)
        if val is not None:
            params[key] = val
    for key in ("dx", "dy", "x", "y"):
        val = getattr(args, key)
        if val is not None:
            params[key] = val
    local = None
    if getattr(args, "local", False):
        local = lambda: _ephemeral(args.engine, args.cdp_url, args.headed).tool_call(  # noqa: E731
            "ax_act", params
        )
    return _daemon_call(args, "tool", {"name": "ax_act", "args": params}, local_fn=local)


def _cmd_snapshot(args) -> Any:
    return _daemon_call(args, "snapshot", {})


def _cmd_screenshot(args) -> Any:
    return _daemon_call(args, "screenshot", {"path": args.path})


def _cmd_tabs(args) -> Any:
    return _daemon_call(args, "tool", {"name": "list_tabs", "args": {}})


def _cmd_info(args) -> Any:
    return _daemon_call(args, "info", {})


def _cmd_status(args) -> Any:
    return _daemon_call(args, "status", {})


# ── daemon lifecycle ────────────────────────────────────────────────────────


def _cmd_daemon(args) -> Any:
    if args.daemon_cmd == "start":
        st = _daemon.start_daemon(
            engine=args.engine,
            cdp_url=args.cdp_url,
            headed=args.headed,
            idle_timeout=args.idle_timeout,
        )
        if not st:
            raise SystemExit("failed to start daemon")
        return {"started": True, "state": st}
    if args.daemon_cmd == "stop":
        return _daemon.stop_daemon()
    return _daemon.daemon_status()


# ── argument parser ─────────────────────────────────────────────────────────


def _common_parent() -> argparse.ArgumentParser:
    """Options shared by every subcommand (defaults SUPPRESS so top-level and
    subcommand positions don't clobber each other)."""
    c = argparse.ArgumentParser(add_help=False)
    c.add_argument("--engine", default=argparse.SUPPRESS, help="auto | bundled | chromium | mock")
    c.add_argument("--cdp-url", dest="cdp_url", default=argparse.SUPPRESS, help="Attach to a CDP endpoint")
    c.add_argument("--headed", action="store_true", default=argparse.SUPPRESS, help="Show a window")
    c.add_argument("--timeout", type=float, default=argparse.SUPPRESS, help="Daemon timeout (s)")
    c.add_argument("--local", action="store_true", default=argparse.SUPPRESS, help="Bypass the daemon")
    return c


def build_parser() -> argparse.ArgumentParser:
    common = _common_parent()
    p = argparse.ArgumentParser(
        prog="fastbrowser", description="fastbrowser CLI (browser + accessibility)", parents=[common]
    )
    p.add_argument("--version", action="version", version=f"fastbrowser {__version__}")
    sub = p.add_subparsers(
        dest="command",
        required=True,
        parser_class=lambda **kw: argparse.ArgumentParser(parents=[common], **kw),
    )

    t = sub.add_parser("tools", help="List fastbrowser tools")
    t.add_argument("--mode", choices=["names", "compact", "full", "help"], default="names")
    t.add_argument("--name", help="With --mode help: the tool to describe")
    t.set_defaults(func=_cmd_tools)

    f = sub.add_parser("fetch", help="Render a URL in a real browser and return its text")
    f.add_argument("url")
    f.add_argument("--max-chars", type=int, default=5000)
    f.set_defaults(func=_cmd_fetch)

    r = sub.add_parser("run", help="Run a multi-step script in one ephemeral browser")
    r.add_argument("--url")
    r.add_argument("--steps", help="JSON array of {\"tool\": name, \"args\": {...}}")
    r.add_argument("--steps-file")
    r.set_defaults(func=_cmd_run)

    d = sub.add_parser("download", help="Download a URL to a file (browser headers)")
    d.add_argument("url")
    d.add_argument("path")
    d.add_argument("--referer")
    d.add_argument("--header", action="append", help="Extra header 'Name: value' (repeatable)")
    d.set_defaults(func=_cmd_download)

    o = sub.add_parser("open", help="Open a URL in the daemon browser")
    o.add_argument("url")
    o.set_defaults(func=_cmd_open)

    n = sub.add_parser("navigate", help="Navigate the daemon browser")
    n.add_argument("url")
    n.set_defaults(func=_cmd_navigate)

    c = sub.add_parser("call", help="Call any fastbrowser tool by name")
    c.add_argument("name")
    c.add_argument("--args", help="JSON object of tool args")
    c.set_defaults(func=_cmd_call)

    ax = sub.add_parser("ax", help="Accessibility: list/snapshot/act")
    ax.add_argument("ax_action", choices=["list", "snapshot", "act"])
    ax.add_argument("--target")
    ax.add_argument("--action", help="With 'act': action name (click/type/scroll/...)")
    ax.add_argument("--ref")
    ax.add_argument("--text")
    ax.add_argument("--value")
    ax.add_argument("--key")
    ax.add_argument("--dx", type=float)
    ax.add_argument("--dy", type=float)
    ax.add_argument("--x", type=float)
    ax.add_argument("--y", type=float)
    ax.set_defaults(func=_cmd_ax)

    s = sub.add_parser("snapshot", help="Interactive-element snapshot (daemon)")
    s.set_defaults(func=_cmd_snapshot)

    sc = sub.add_parser("screenshot", help="Save a screenshot (daemon)")
    sc.add_argument("path")
    sc.set_defaults(func=_cmd_screenshot)

    tb = sub.add_parser("tabs", help="List tabs (daemon)")
    tb.set_defaults(func=_cmd_tabs)

    inf = sub.add_parser("info", help="Engine/runtime info (daemon)")
    inf.set_defaults(func=_cmd_info)

    st = sub.add_parser("status", help="Runtime status (daemon)")
    st.set_defaults(func=_cmd_status)

    sv = sub.add_parser("serve", help="Run the daemon in the foreground")
    sv.add_argument("--socket")
    sv.add_argument("--port", type=int, default=0)
    sv.add_argument("--idle-timeout", type=int, default=_daemon.DEFAULT_IDLE_TIMEOUT)
    sv.set_defaults(func=None)

    dm = sub.add_parser("daemon", help="Manage the background daemon")
    dm.add_argument("daemon_cmd", choices=["start", "stop", "status"])
    dm.add_argument("--idle-timeout", type=int, default=_daemon.DEFAULT_IDLE_TIMEOUT)
    dm.set_defaults(func=_cmd_daemon)

    return p


def main(argv: Optional[List[str]] = None) -> int:
    args = build_parser().parse_args(argv)
    # SUPPRESS defaults leave attributes unset — normalize them
    for key, default in (
        ("engine", None),
        ("cdp_url", None),
        ("headed", False),
        ("timeout", 300.0),
        ("local", False),
    ):
        if not hasattr(args, key):
            setattr(args, key, default)
    try:
        if args.command == "serve":
            return _daemon.serve(
                engine=args.engine,
                cdp_url=args.cdp_url,
                headed=args.headed,
                socket_path=args.socket,
                port=args.port,
                idle_timeout=args.idle_timeout,
            )
        result = args.func(args)
        _print(result)
        return 0
    except SystemExit:
        raise
    except Exception as e:  # noqa: BLE001
        _print({"error": f"{type(e).__name__}: {e}"})
        return 1


if __name__ == "__main__":  # pragma: no cover
    raise SystemExit(main())
