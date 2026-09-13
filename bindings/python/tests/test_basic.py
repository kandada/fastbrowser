import json

import pytest

import fastbrowser as fb


def make_browser(config=None):
    b = fb.FastBrowser()
    b.init(config)
    return b


def test_open_snapshot_tool_flow():
    b = make_browser()
    assert b.is_initialized

    out = b.open("https://example.com")
    assert out["title"] == "Example Page"
    assert out["tab"] == 1

    snap = b.snapshot()
    assert snap["title"] == "Example Page"
    assert snap["interactive"], "snapshot 应包含交互元素"

    url = b.tool_call("get_current_url")
    assert url["url"] == "https://example.com"

    assert b.tool_count() >= 30
    assert len(b.tool_list()) >= 30

    w, h, rgba = b.screenshot()
    assert w > 0 and h > 0
    assert len(rgba) == w * h * 4

    b.shutdown()
    assert not b.is_initialized


def test_screenshot_png():
    b = make_browser()
    b.open("https://example.com")
    png = b.screenshot_png()
    assert png[:8] == b"\x89PNG\r\n\x1a\n", "应为合法 PNG 签名"
    assert png[-8:-4] == b"IEND", "应以 IEND 块收尾"


def test_tool_call_and_viewport():
    b = make_browser()
    b.open("https://example.com")

    title = b.tool_call("get_page_title")
    assert title["title"] == "Example Page"

    b.set_viewport(320, 480)
    snap = b.snapshot()
    assert snap["viewport"]["width"] == 320
    assert snap["viewport"]["height"] == 480


def test_navigate_and_get_view():
    b = make_browser()
    b.open("https://example.com")
    out = b.navigate("https://example.com/login")
    assert out["title"] == "Login"

    view = b.get_view()
    assert view is not None and "handle" in view


def test_status_info_audit():
    b = make_browser()
    b.open("https://example.com")
    b.tool_call("get_page_title")

    info = b.get_info()
    assert info["engine"] == "mock"
    assert info["initialized"] is True
    assert info["tools"] >= 30

    st = b.status()
    assert st["engine"] == "mock"

    b.tool_call("get_current_url")
    audit = b.audit()
    assert isinstance(audit, list) and audit, "审计日志应非空"
    b.clear_audit()
    assert b.audit() == []


def test_session_save_load(tmp_path):
    path = str(tmp_path / "state.json")
    b = make_browser()
    b.open("https://example.com")
    b.tool_call("cookie_set", {"name": "sid", "value": "abc", "domain": "example.com"})
    b.tool_call("storage_set", {"key": "token", "value": "t1"})
    assert b.session_save(path)["tabs"] == 1
    b.shutdown()

    b2 = make_browser()
    assert b2.session_load(path)["tabs"] == 1
    b2.open("https://example.com")
    cookies = b2.tool_call("cookie_get", {"domain": "example.com"})
    assert cookies["cookies"][0]["value"] == "abc"
    storage = b2.tool_call("storage_get", {"key": "token"})
    assert storage["value"] == "t1"


def test_clear_state():
    b = make_browser()
    b.open("https://example.com")
    b.tool_call("new_tab", {"url": "https://example.com/search"})
    b.tool_call("cookie_set", {"name": "sid", "value": "x", "domain": "example.com"})
    out = b.clear_state()
    assert out["cleared"] is True
    assert out["closed_tabs"] >= 1
    assert b.tool_call("list_tabs")["tabs"] and len(b.tool_call("list_tabs")["tabs"]) == 1
    assert b.tool_call("cookie_get")["cookies"] == []


def test_event_callback():
    b = make_browser()
    events = []
    b.register_event_callback(lambda tab, e: events.append((tab, json.loads(e))))
    b.open("https://example.com")
    b.navigate("https://example.com/login")
    assert events, "应收到页面事件"


def test_frame_stream():
    import time

    b = make_browser()
    frames = []
    b.register_frame_callback(lambda tab, f: frames.append((tab, json.loads(f))))
    b.open("https://example.com")
    out = b.start_frame_stream(1, fps=50)
    assert out["started"] is True
    time.sleep(0.2)
    b.stop_frame_stream(1)
    assert frames, "应收到离屏帧"
    assert frames[0][1]["width"] > 0 and frames[0][1]["height"] > 0


def test_callback_can_reenter_browser():
    """回调重入浏览器（快照）不应死锁——解耦派发后回调在锁外执行。"""
    import time

    b = make_browser()
    reentered = []

    def on_frame(tab, f):
        if not reentered:  # 只在第一帧重入一次，避免高频快照
            reentered.append(b.snapshot()["title"])

    b.register_frame_callback(on_frame)
    b.open("https://example.com")
    b.start_frame_stream(1, fps=30)
    time.sleep(0.3)
    b.stop_frame_stream(1)
    assert reentered == ["Example Page"], "回调内重入快照应成功返回"


def test_not_initialized_raises():
    b = fb.FastBrowser()
    with pytest.raises(RuntimeError):
        b.open("https://example.com")
    with pytest.raises(RuntimeError):
        b.tool_call("get_page_title")
    assert not b.is_initialized


def test_invalid_config_raises():
    b = fb.FastBrowser()
    with pytest.raises(RuntimeError):
        b.init({"engine": "no-such-engine"})  # 非法引擎名 → 引擎创建失败


def test_malformed_config_raises():
    b = fb.FastBrowser()
    with pytest.raises(ValueError):
        b.init({"engine": 123})  # 字段类型不符 → 配置解析失败


def test_invalid_tool_raises():
    b = make_browser()
    b.open("https://example.com")
    with pytest.raises(RuntimeError):
        b.tool_call("no_such_tool")


def test_invalid_rendering_mode_raises():
    b = make_browser()
    b.init()
    with pytest.raises(RuntimeError):
        b.set_rendering_mode("bogus")
    b.set_rendering_mode("headless")  # 合法
    assert b.status()["rendering_mode"] == "headless"
