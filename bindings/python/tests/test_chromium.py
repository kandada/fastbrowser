"""真实 Chromium 集成测试（engine="bundled"，需 vendor/chromium 或 CHROME_PATH）。

无 bundled chromium 时 SKIP，不阻塞无浏览器环境。
用 data: URL（离线、确定性），避免外部网络依赖。
"""

import pytest

import fastbrowser as fb

DATA = "data:text/html,<title>Hello</title><h1>Hi</h1><button id='go'>Go</button><input id='q'>"


def _open_bundled():
    b = fb.FastBrowser()
    try:
        b.init({"engine": "bundled", "command_timeout_ms": 15000})
    except Exception as e:  # 无 bundled chromium / 启动失败 → 跳过
        b.shutdown()
        pytest.skip(f"bundled chromium unavailable: {e}")
    return b


def test_real_open_snapshot():
    b = _open_bundled()
    out = b.open(DATA)
    assert out["title"] == "Hello"

    snap = b.snapshot()
    assert snap["title"] == "Hello"
    assert any(e.get("tag") == "button" for e in snap["interactive"])
    assert any(e.get("tag") == "input" for e in snap["interactive"])
    b.shutdown()


def test_real_tool_calls():
    b = _open_bundled()
    b.open(DATA)
    assert b.tool_call("get_page_title")["title"] == "Hello"
    assert "Hi" in b.tool_call("get_page_text")["text"]
    assert b.tool_call("get_current_url")["url"].startswith("data:")
    assert b.tool_call("list_tabs")["tabs"], "应至少有一个标签页"
    b.shutdown()


def test_real_click_and_type():
    b = _open_bundled()
    b.open(DATA)
    snap = b.snapshot()
    btn = next(e for e in snap["interactive"] if e.get("tag") == "button")
    # 点击按钮（真实坐标级点击）
    b.tool_call("click", {"id": btn["id"]})
    # 输入框输入真实文本
    inp = next(e for e in snap["interactive"] if e.get("tag") == "input")
    b.tool_call("type", {"id": inp["id"], "text": "abc"})
    # 输入后重新快照，输入框值应可见
    snap2 = b.snapshot()
    q = next(e for e in snap2["interactive"] if e.get("tag") == "input")
    assert q.get("value") == "abc"
    b.shutdown()


def test_real_screenshot_png():
    b = _open_bundled()
    b.open(DATA)
    png = b.screenshot_png()
    assert png[:8] == b"\x89PNG\r\n\x1a\n"
    assert png[-8:-4] == b"IEND"
    b.shutdown()


def test_real_async_flow():
    import asyncio

    async def main():
        b = fb.AsyncFastBrowser()
        try:
            await b.init({"engine": "bundled", "command_timeout_ms": 15000})
        except Exception as e:
            pytest.skip(f"bundled chromium unavailable: {e}")
            return
        out = await b.open(DATA)
        assert out["title"] == "Hello"
        snap = await b.snapshot()
        assert snap["title"] == "Hello"
        png = await b.screenshot_png()
        assert png[:8] == b"\x89PNG\r\n\x1a\n"
        b.shutdown()

    asyncio.run(main())
