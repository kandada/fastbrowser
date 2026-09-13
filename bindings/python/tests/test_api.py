"""Public-API regression tests for the Python bindings (mock engine).

These pin the documented, user-facing surface (version, tool registry shape,
screenshot encoding, info/status, lifecycle idempotency, async concurrency) so
the PyPI package stays stable. No browser required.
"""

import asyncio
import struct
from importlib.metadata import version as _pkg_version

import pytest

import fastbrowser as fb
from fastbrowser import _core


def make(config=None):
    b = fb.FastBrowser()
    b.init(config)
    return b


def test_version_is_exposed_and_consistent():
    assert isinstance(fb.__version__, str) and fb.__version__
    # The compiled extension reports the kernel version; the installed package
    # metadata (from pyproject.toml) must mirror it.
    assert _core.version() == fb.__version__
    assert fb.__version__ == _pkg_version("fastbrowser")


def test_tool_list_is_well_formed():
    b = make()
    tools = b.tool_list()
    assert len(tools) == b.tool_count()
    names = set()
    for t in tools:
        name = t["name"]
        assert name and name not in names, f"duplicate/empty tool name: {name!r}"
        names.add(name)
        assert t["description"].strip(), f"{name} missing description"
        assert t["schema"]["type"] == "object", f"{name} schema must be an object"
    # A representative slice of the documented tools must exist.
    # NOTE: `open`/`snapshot` are Python methods, not tools.
    for expected in ["navigate", "click", "type", "get_current_url", "list_tabs"]:
        assert expected in names


def test_screenshot_png_ihdr_dimensions_match():
    b = make()
    b.open("https://example.com")
    w, h, rgba = b.screenshot()
    assert len(rgba) == w * h * 4
    png = b.screenshot_png()
    assert png[:8] == b"\x89PNG\r\n\x1a\n"
    # IHDR: 8-byte signature + 4-byte length + 4-byte type, then width/height.
    iw, ih = struct.unpack(">II", png[16:24])
    assert (iw, ih) == (w, h)


def test_get_info_reports_platform_and_version():
    b = make()
    info = b.get_info()
    assert info["engine"] == "mock"
    assert info["initialized"] is True
    assert info["tools"] == b.tool_count()
    assert isinstance(info["version"], str) and info["version"]
    assert isinstance(info["platform"], str) and info["platform"]


def test_snapshot_shape():
    b = make()
    b.open("https://example.com")
    snap = b.snapshot()
    for key in ("title", "url", "viewport", "interactive"):
        assert key in snap, f"snapshot missing {key!r}"
    assert isinstance(snap["interactive"], list) and snap["interactive"]
    first = snap["interactive"][0]
    assert "id" in first and "tag" in first


def test_shutdown_is_idempotent():
    b = make()
    b.shutdown()
    b.shutdown()  # must not raise
    assert not b.is_initialized


def test_callbacks_receive_json_events():
    b = make()
    got = []
    b.register_event_callback(lambda tab, e: got.append((tab, e)))
    b.open("https://example.com")
    b.navigate("https://example.com/login")
    assert got, "expected at least one page event"
    tab, raw = got[0]
    assert isinstance(tab, int)
    import json

    assert isinstance(json.loads(raw), dict)


def test_async_gather_three_tabs():
    async def main():
        b = fb.AsyncFastBrowser()
        await b.init()
        tabs = []
        for i in range(3):
            tabs.append((await b.open(f"https://example.com/p{i}"))["tab"])
        titles = await asyncio.gather(
            *[b.tool_call("get_page_title", {"tab": t}) for t in tabs]
        )
        assert len(titles) == 3
        assert all(isinstance(x.get("title"), str) for x in titles)
        b.shutdown()

    asyncio.run(main())


def test_error_is_runtime_error_before_init():
    b = fb.FastBrowser()
    with pytest.raises(RuntimeError):
        b.snapshot()
    with pytest.raises(RuntimeError):
        b.tool_call("click", {"id": "a"})
