"""fastbrowser local daemon + client.

Holds a single ``FastBrowser`` so state (cookies / tabs / current page) persists
across separate CLI processes (``python -m fastbrowser ...`` is a new process
each time). The daemon listens on **both** a Unix domain socket and a loopback
TCP port; clients auto-detect a running daemon via a discovery file and connect
over either transport (Unix first, then TCP).

Protocol: newline-delimited JSON.
    request  {"token": "...", "cmd": "...", "args": {...}}\n
    response {"ok": true, "result": <any>}\n  |  {"ok": false, "error": "..."}\n
"""

from __future__ import annotations

import atexit
import json
import os
import secrets
import socket
import subprocess
import sys
import threading
import time
from pathlib import Path
from typing import Any, Dict, Optional, Tuple

DEFAULT_IDLE_TIMEOUT = 1800  # seconds of inactivity before the daemon exits
_MAX_MSG = 32 * 1024 * 1024


# ── discovery file ────────────────────────────────────────────────────────


def _state_dir() -> Path:
    d = Path(os.getenv("FASTBROWSER_STATE_DIR") or (Path.home() / ".fastbrowser"))
    try:
        d.mkdir(parents=True, exist_ok=True)
    except OSError:
        pass
    return d


def _state_file() -> Path:
    return _state_dir() / "daemon.json"


def _default_socket_path() -> str:
    uid = getattr(os, "getuid", lambda: 0)()
    return f"/tmp/fastbrowser-{uid}.sock"


def _pid_alive(pid: int) -> bool:
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    except OSError:
        return False
    return True


def read_state() -> Optional[Dict[str, Any]]:
    f = _state_file()
    try:
        s = json.loads(f.read_text(encoding="utf-8"))
    except Exception:
        return None
    pid = s.get("pid")
    if not isinstance(pid, int) or not _pid_alive(pid):
        try:
            f.unlink()
        except OSError:
            pass
        return None
    return s


def _write_state(state: Dict[str, Any]) -> None:
    f = _state_file()
    tmp = f.with_name(f.name + ".tmp")
    tmp.write_text(json.dumps(state), encoding="utf-8")
    tmp.replace(f)


def _clear_state() -> None:
    try:
        _state_file().unlink()
    except OSError:
        pass


# ── client ────────────────────────────────────────────────────────────────


def _roundtrip(sock: socket.socket, token: str, cmd: str, args: dict, timeout: float) -> dict:
    payload = json.dumps({"token": token, "cmd": cmd, "args": args or {}}) + "\n"
    sock.settimeout(timeout)
    sock.sendall(payload.encode("utf-8"))
    buf = bytearray()
    while not buf.endswith(b"\n"):
        chunk = sock.recv(65536)
        if not chunk:
            break
        buf += chunk
        if len(buf) > _MAX_MSG:
            raise RuntimeError("response too large")
    if not buf:
        raise RuntimeError("empty response from daemon")
    return json.loads(buf.decode("utf-8"))


def call_daemon(state: dict, cmd: str, args: dict = None, timeout: float = 300) -> Optional[dict]:
    """Send one request to a known daemon. Returns None if unreachable on both transports."""
    token = state.get("token", "")
    socket_path = state.get("socket")
    if socket_path and hasattr(socket, "AF_UNIX"):
        try:
            s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            try:
                s.connect(socket_path)
                return _roundtrip(s, token, cmd, args or {}, timeout)
            finally:
                s.close()
        except OSError:
            pass
    port = state.get("port")
    if port:
        try:
            s = socket.create_connection(("127.0.0.1", int(port)), timeout=timeout)
            try:
                return _roundtrip(s, token, cmd, args or {}, timeout)
            finally:
                s.close()
        except OSError:
            pass
    return None


def start_daemon(
    engine: Optional[str] = None,
    cdp_url: Optional[str] = None,
    headed: bool = False,
    idle_timeout: int = DEFAULT_IDLE_TIMEOUT,
    wait: float = 20.0,
) -> Optional[dict]:
    """Start a detached daemon if none is running; return its discovery state."""
    existing = read_state()
    if existing:
        return existing
    argv = [sys.executable, "-m", "fastbrowser", "serve", "--idle-timeout", str(idle_timeout)]
    if engine:
        argv += ["--engine", engine]
    if cdp_url:
        argv += ["--cdp-url", cdp_url]
    if headed:
        argv += ["--headed"]
    subprocess.Popen(
        argv,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        stdin=subprocess.DEVNULL,
        start_new_session=True,
        close_fds=True,
    )
    deadline = time.time() + wait
    while time.time() < deadline:
        st = read_state()
        if st:
            r = call_daemon(st, "ping", {}, timeout=2)
            if r and r.get("ok"):
                return st
        time.sleep(0.1)
    return read_state()


def send(
    cmd: str,
    args: dict = None,
    ensure: bool = True,
    timeout: float = 300,
    engine: Optional[str] = None,
    cdp_url: Optional[str] = None,
    headed: bool = False,
) -> dict:
    """Send a request to the daemon, starting one on demand when ``ensure``."""
    state = read_state()
    if state:
        r = call_daemon(state, cmd, args, timeout)
        if r is not None:
            return r
    if not ensure:
        return {"ok": False, "error": "no daemon running"}
    state = start_daemon(engine=engine, cdp_url=cdp_url, headed=headed)
    if not state:
        return {"ok": False, "error": "failed to start daemon"}
    r = call_daemon(state, cmd, args, timeout)
    return r if r is not None else {"ok": False, "error": "daemon unreachable"}


def stop_daemon() -> dict:
    state = read_state()
    if not state:
        return {"ok": True, "result": {"stopped": False, "reason": "no daemon running"}}
    r = call_daemon(state, "shutdown", {}, timeout=5)
    # give it a moment, then force-clear the stale discovery file
    time.sleep(0.3)
    pid = state.get("pid")
    if isinstance(pid, int) and _pid_alive(pid):
        try:
            os.kill(pid, 15)
            time.sleep(0.3)
            if _pid_alive(pid):
                os.kill(pid, 9)
        except OSError:
            pass
    _clear_state()
    return {"ok": True, "result": {"stopped": True, "ack": r}}


def daemon_status() -> dict:
    state = read_state()
    if not state:
        return {"running": False}
    r = call_daemon(state, "ping", {}, timeout=3)
    if not r or not r.get("ok"):
        return {"running": False, "state": state}
    return {"running": True, "state": state, "info": r.get("result")}


# ── server ────────────────────────────────────────────────────────────────


class _Server:
    def __init__(self, browser: Any, token: str, idle_timeout: int):
        self.b = browser
        self.token = token
        self.idle_timeout = idle_timeout
        self.lock = threading.Lock()
        self.last = time.time()
        self.stop_evt = threading.Event()
        self.listeners: list = []

    # -- request handling --
    def _dispatch(self, req: dict) -> dict:
        if req.get("token") != self.token:
            return {"ok": False, "error": "unauthorized"}
        self.last = time.time()
        try:
            return {"ok": True, "result": self._run(req.get("cmd", ""), req.get("args") or {})}
        except Exception as e:  # noqa: BLE001
            return {"ok": False, "error": f"{type(e).__name__}: {e}"}

    def _run(self, cmd: str, args: dict) -> Any:
        b = self.b
        if cmd == "ping":
            return {"pong": True, "pid": os.getpid(), "version": _version()}
        with self.lock:
            if cmd == "info":
                return b.get_info()
            if cmd == "status":
                return b.status()
            if cmd == "tools":
                return b.tool_list()
            if cmd == "open":
                return b.open(args.get("url", ""))
            if cmd == "navigate":
                return b.navigate(args.get("url", ""))
            if cmd == "tool":
                return b.tool_call(args.get("name", ""), args.get("args") or {})
            if cmd == "snapshot":
                return b.snapshot()
            if cmd == "screenshot":
                path = args.get("path") or str(_state_dir() / f"screenshot-{int(time.time())}.png")
                b.save_screenshot(path)
                return {"path": path}
            if cmd == "viewport":
                b.set_viewport(int(args["width"]), int(args["height"]))
                return {"ok": True}
            if cmd == "shutdown":
                threading.Thread(target=self.shutdown, daemon=True).start()
                return {"stopping": True}
        raise ValueError(f"unknown cmd '{cmd}'")

    # -- connection handling --
    def _handle(self, conn: socket.socket) -> None:
        try:
            conn.settimeout(600)
            buf = bytearray()
            while not buf.endswith(b"\n"):
                chunk = conn.recv(65536)
                if not chunk:
                    return
                buf += chunk
                if len(buf) > _MAX_MSG:
                    return
            try:
                req = json.loads(bytes(buf).decode("utf-8"))
            except Exception:
                resp = {"ok": False, "error": "bad json"}
            else:
                resp = self._dispatch(req)
            conn.sendall((json.dumps(resp, ensure_ascii=False) + "\n").encode("utf-8"))
        except OSError:
            pass
        finally:
            try:
                conn.close()
            except OSError:
                pass

    def _accept_loop(self, sock: socket.socket) -> None:
        while not self.stop_evt.is_set():
            try:
                conn, _ = sock.accept()
            except OSError:
                break
            threading.Thread(target=self._handle, args=(conn,), daemon=True).start()

    def start(self, socket_path: Optional[str], port: int) -> Tuple[Optional[str], int]:
        if socket_path and hasattr(socket, "AF_UNIX"):
            try:
                us = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
                try:
                    os.unlink(socket_path)
                except OSError:
                    pass
                us.bind(socket_path)
                os.chmod(socket_path, 0o600)
                us.listen(64)
                self.listeners.append(us)
                threading.Thread(target=self._accept_loop, args=(us,), daemon=True).start()
            except OSError:
                socket_path = None
        ts = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        ts.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        ts.bind(("127.0.0.1", int(port or 0)))
        ts.listen(64)
        actual_port = ts.getsockname()[1]
        self.listeners.append(ts)
        threading.Thread(target=self._accept_loop, args=(ts,), daemon=True).start()
        return socket_path, actual_port

    def _idle_loop(self) -> None:
        while not self.stop_evt.is_set():
            if self.stop_evt.wait(5):
                return
            if self.idle_timeout and (time.time() - self.last) > self.idle_timeout:
                self.shutdown()
                return

    def shutdown(self) -> None:
        if self.stop_evt.is_set():
            return
        self.stop_evt.set()
        with self.lock:
            try:
                self.b.shutdown()
            except Exception:  # noqa: BLE001
                pass
        for s in self.listeners:
            try:
                s.close()
            except OSError:
                pass
        _clear_state()


def _version() -> str:
    from . import __version__

    return __version__


def serve(
    engine: Optional[str] = None,
    cdp_url: Optional[str] = None,
    headed: bool = False,
    socket_path: Optional[str] = None,
    port: int = 0,
    idle_timeout: int = DEFAULT_IDLE_TIMEOUT,
) -> int:
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
    try:
        b.init(cfg)
    except Exception as e:  # noqa: BLE001
        _clear_state()
        print(f"fastbrowser daemon: init failed: {e}", file=sys.stderr)
        return 1

    token = secrets.token_hex(16)
    srv = _Server(b, token, idle_timeout)
    used_socket, used_port = srv.start(socket_path or _default_socket_path(), port)
    _write_state(
        {
            "pid": os.getpid(),
            "version": _version(),
            "socket": used_socket,
            "port": used_port,
            "token": token,
            "started_at": time.time(),
        }
    )
    atexit.register(_clear_state)
    threading.Thread(target=srv._idle_loop, daemon=True).start()
    print(
        json.dumps({"daemon": "ready", "pid": os.getpid(), "socket": used_socket, "port": used_port}),
        flush=True,
    )
    try:
        while not srv.stop_evt.is_set():
            srv.stop_evt.wait(1)
    except KeyboardInterrupt:
        pass
    finally:
        srv.shutdown()
    return 0
