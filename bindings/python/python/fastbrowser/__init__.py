"""fastbrowser — Python bindings for the browser-automation kernel.

Thin wrapper: `_core` is the PyO3 extension returning JSON strings; this layer
converts JSON into Python dict/list and exposes two Pythonic classes:

- `FastBrowser`: synchronous interface (blocking calls release the GIL).
- `AsyncFastBrowser`: asyncio interface (blocking ops offloaded to a thread pool,
  supports `await`/`gather`).

Example (sync):
    import fastbrowser as fb
    b = fb.FastBrowser(); b.init()
    b.open("https://example.com"); print(b.snapshot()["title"])

Example (async):
    import fastbrowser as fb
    b = fb.AsyncFastBrowser(); await b.init()
    await b.open("https://example.com"); print((await b.snapshot())["title"])
"""

import asyncio
import functools
import json
import struct
import zlib
from typing import Any, Callable, Dict, List, Optional, Tuple

from . import _core

__version__: str = _core.version()

__all__ = ["FastBrowser", "AsyncFastBrowser", "__version__"]


def _rgba_to_png(width: int, height: int, rgba: bytes) -> bytes:
    """Encode raw RGBA8 bytes as PNG (pure stdlib, no Pillow dependency)."""

    def chunk(typ: bytes, data: bytes) -> bytes:
        c = struct.pack(">I", len(data)) + typ + data
        c += struct.pack(">I", zlib.crc32(typ + data) & 0xFFFFFFFF)
        return c

    stride = width * 4
    raw = bytearray()
    for y in range(height):
        raw.append(0)  # per-row filter type 0
        raw.extend(rgba[y * stride : (y + 1) * stride])

    ihdr = struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)  # 8-bit RGBA
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", ihdr)
        + chunk(b"IDAT", zlib.compress(bytes(raw), 9))
        + chunk(b"IEND", b"")
    )


async def _block(func, *args):
    """Offload a blocking sync call to a thread pool (kernel calls release the GIL, so this is truly concurrent)."""
    if hasattr(asyncio, "to_thread"):
        return await asyncio.to_thread(func, *args)
    return await asyncio.get_running_loop().run_in_executor(None, functools.partial(func, *args))


class FastBrowser:
    """fastbrowser kernel handle (synchronous interface; single in-process instance semantics)."""

    def __init__(self) -> None:
        self._b = _core.Browser()

    # ── lifecycle ───────────────────────────────────────────────
    def init(self, config: Optional[dict] = None) -> dict:
        """Initialize the kernel. `config` is a `Config` dict (engine/cdp_url/viewport/...); defaults to the mock engine."""
        cfg = json.dumps(config) if config is not None else None
        return json.loads(self._b.init(cfg))

    @property
    def is_initialized(self) -> bool:
        return self._b.is_initialized()

    def shutdown(self) -> None:
        self._b.shutdown()

    # ── navigation ──────────────────────────────────────────────
    def open(self, url: str) -> dict:
        """Open a URL and activate it (creates a tab if needed)."""
        return json.loads(self._b.open(url))

    def navigate(self, url: str) -> dict:
        return json.loads(self._b.navigate(url))

    # ── tools ───────────────────────────────────────────────────
    def tool_call(self, name: str, params: Optional[dict] = None) -> dict:
        """Run a single tool call; returns the result dict."""
        p = json.dumps(params or {})
        return json.loads(self._b.tool_call(name, p))

    def tool_list(self) -> List[dict]:
        return json.loads(self._b.tool_list())

    def tool_count(self) -> int:
        return self._b.tool_count()

    # ── page content ────────────────────────────────────────────
    def snapshot(self) -> dict:
        """Interactive-element snapshot (for LLM perception)."""
        return json.loads(self._b.snapshot())

    def screenshot(self) -> Tuple[int, int, bytes]:
        """Return `(width, height, rgba_bytes)`."""
        return self._b.screenshot()

    def screenshot_png(self) -> bytes:
        """Return the screenshot encoded as PNG (pure stdlib encoding)."""
        w, h, rgba = self.screenshot()
        return _rgba_to_png(w, h, rgba)

    def save_screenshot(self, path: str) -> None:
        """Save the current page screenshot as PNG to `path`."""
        with open(path, "wb") as f:
            f.write(self.screenshot_png())

    def set_viewport(self, width: int, height: int) -> None:
        self._b.set_viewport(width, height)

    def get_view(self) -> Optional[dict]:
        """Host view handle for the current tab (`{"kind", "handle"}`)."""
        raw = self._b.get_view()
        return json.loads(raw) if raw is not None else None

    # ── rendering ───────────────────────────────────────────────
    def set_rendering_mode(self, mode: str) -> None:
        """Switch rendering mode: 'hosted' (windowed) or 'headless' (offscreen)."""
        self._b.set_rendering_mode(mode)

    def start_frame_stream(self, tab: int, fps: int = 10, max_width: int = 0, max_height: int = 0) -> dict:
        """Start offscreen frame streaming (register a frame callback first)."""
        return json.loads(self._b.start_frame_stream(tab, fps, max_width, max_height))

    def stop_frame_stream(self, tab: int) -> dict:
        return json.loads(self._b.stop_frame_stream(tab))

    # ── callbacks (run on a separate dispatch thread; safe to re-enter the browser) ──
    def register_event_callback(self, callback: Callable[[int, str], None]) -> None:
        """Register a page-event callback `callback(tab: int, event_json: str)`."""
        self._b.register_event_callback(callback)

    def register_frame_callback(self, callback: Callable[[int, str], None]) -> None:
        """Register an offscreen-frame callback `callback(tab: int, frame_json: str)`."""
        self._b.register_frame_callback(callback)

    # ── state ───────────────────────────────────────────────────
    def status(self) -> dict:
        return json.loads(self._b.status())

    def audit(self) -> List[dict]:
        return json.loads(self._b.audit())

    def clear_audit(self) -> None:
        self._b.clear_audit()

    def get_info(self) -> dict:
        return json.loads(self._b.get_info())

    # ── session ─────────────────────────────────────────────────
    def session_save(self, path: str) -> dict:
        return json.loads(self._b.session_save(path))

    def session_load(self, path: str) -> dict:
        return json.loads(self._b.session_load(path))

    def clear_state(self) -> dict:
        return json.loads(self._b.clear_state())


class AsyncFastBrowser:
    """fastbrowser kernel handle (asyncio interface; blocking ops offloaded to a thread pool)."""

    def __init__(self) -> None:
        self._fb = FastBrowser()

    # ── cheap sync reads / registration (non-blocking, direct delegation) ──
    @property
    def is_initialized(self) -> bool:
        return self._fb.is_initialized

    def tool_list(self) -> List[dict]:
        return self._fb.tool_list()

    def tool_count(self) -> int:
        return self._fb.tool_count()

    def status(self) -> dict:
        return self._fb.status()

    def audit(self) -> List[dict]:
        return self._fb.audit()

    def clear_audit(self) -> None:
        self._fb.clear_audit()

    def get_info(self) -> dict:
        return self._fb.get_info()

    def get_view(self) -> Optional[dict]:
        return self._fb.get_view()

    def set_rendering_mode(self, mode: str) -> None:
        self._fb.set_rendering_mode(mode)

    def register_event_callback(self, callback: Callable[[int, str], None]) -> None:
        self._fb.register_event_callback(callback)

    def register_frame_callback(self, callback: Callable[[int, str], None]) -> None:
        self._fb.register_frame_callback(callback)

    def shutdown(self) -> None:
        self._fb.shutdown()

    # ── blocking ops → async ─────────────────────────────────────
    async def init(self, config: Optional[dict] = None) -> dict:
        return await _block(self._fb.init, config)

    async def open(self, url: str) -> dict:
        return await _block(self._fb.open, url)

    async def navigate(self, url: str) -> dict:
        return await _block(self._fb.navigate, url)

    async def tool_call(self, name: str, params: Optional[dict] = None) -> dict:
        return await _block(self._fb.tool_call, name, params)

    async def snapshot(self) -> dict:
        return await _block(self._fb.snapshot)

    async def screenshot(self) -> Tuple[int, int, bytes]:
        return await _block(self._fb.screenshot)

    async def screenshot_png(self) -> bytes:
        return await _block(self._fb.screenshot_png)

    async def save_screenshot(self, path: str) -> None:
        return await _block(self._fb.save_screenshot, path)

    async def set_viewport(self, width: int, height: int) -> None:
        return await _block(self._fb.set_viewport, width, height)

    async def start_frame_stream(self, tab: int, fps: int = 10, max_width: int = 0, max_height: int = 0) -> dict:
        return await _block(self._fb.start_frame_stream, tab, fps, max_width, max_height)

    async def stop_frame_stream(self, tab: int) -> dict:
        return await _block(self._fb.stop_frame_stream, tab)

    async def session_save(self, path: str) -> dict:
        return await _block(self._fb.session_save, path)

    async def session_load(self, path: str) -> dict:
        return await _block(self._fb.session_load, path)

    async def clear_state(self) -> dict:
        return await _block(self._fb.clear_state)
