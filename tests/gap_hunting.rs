// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! 查漏补缺：针对 mock 引擎与 SDK 的深入探测测试。
//!
//! 目的：
//! 1. 回归验证「非法快照编号不再 panic」（曾在 mock 元素解析中因
//!    无符号下溢崩溃：`('A' as usize) - ('a' as usize)`）；
//! 2. 探测描述与实现不一致的工具（extract_text 的 ref 分支）；
//! 3. 覆盖尚未被既有测试触碰的 SDK / 会话 / 事件 / 引擎路径。

use std::sync::Arc;

use fastbrowser::engine::host::{PageEventSink, ViewFrameSink};
use fastbrowser::engine::{PageEvent, RenderingMode, ViewFrame};
use fastbrowser::sdk::Fastbrowser;
use fastbrowser::Config;
use serde_json::json;

fn sdk() -> Fastbrowser {
    let s = Fastbrowser::new();
    s.init(Config::default()).unwrap();
    s
}

fn sdk_open(url: &str) -> Fastbrowser {
    let s = sdk();
    s.open(url).unwrap();
    s
}

// ── 回归：非法快照编号必须报错而非 panic ──────────────────────

/// 非小写字母 / 非字母的单字符 id 不应导致 mock 引擎崩溃（曾在 debug 下下溢 panic）。
#[test]
fn invalid_snapshot_ids_return_errors_not_panic() {
    for id in ["A", "1", "?", "@", " ", "{", "["] {
        let s = sdk_open("https://example.com");
        for (tool, params) in [
            ("click", json!({"id": id})),
            ("dblclick", json!({"id": id})),
            ("hover", json!({"id": id})),
            ("type", json!({"id": id, "text": "x"})),
            ("focus", json!({"id": id})),
            ("get_element_text", json!({"id": id})),
            ("get_element_info", json!({"id": id})),
            ("is_visible", json!({"id": id})),
            ("is_enabled", json!({"id": id})),
            ("clear_input", json!({"id": id})),
            ("screenshot_element", json!({"id": id})),
        ] {
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                s.tool_call(tool, params.clone())
            }));
            match r {
                Ok(Err(_)) => {} // 预期：优雅报错
                Ok(Ok(v)) => panic!("{tool} id={id:?} unexpectedly succeeded: {v}"),
                Err(_) => panic!("{tool} id={id:?} PANICKED"),
            }
        }
    }
}

/// 多字符 id 取首字母（设计如此），但不允许崩溃。
#[test]
fn multichar_ids_never_panic() {
    for id in ["aa", "ab", "aGarbage", "zzzz"] {
        let s = sdk_open("https://example.com");
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            s.tool_call("click", json!({"id": id}))
        }));
        assert!(r.is_ok(), "click id={id:?} PANICKED");
    }
}

/// 空 id / 空 ref 也应报错（而非 panic）。
#[test]
fn empty_element_ref_errors() {
    let s = sdk_open("https://example.com");
    assert!(s.tool_call("click", json!({"id": ""})).is_err());
    assert!(s.tool_call("click", json!({"ref": {}})).is_err());
    assert!(s
        .tool_call("click", json!({"ref": {"kind": "css"}}))
        .is_err());
    assert!(s
        .tool_call("click", json!({"ref": {"value": "x"}}))
        .is_err());
    // 同一元素 id 后跟多余字符：取首字母（与设计一致）
    assert!(s.tool_call("click", json!({"id": "agarbage"})).is_ok());
}

// ── 工具描述与实现一致性 ──────────────────────────────────────

/// extract_text 声称支持 'id'/'ref'，但实现只处理了 id → 回归。
#[test]
fn extract_text_supports_ref_and_id() {
    let s = sdk_open("https://example.com");
    // 通过 ref.text 提取单个元素文本（"Learn more" 链接）
    let v = s
        .tool_call(
            "extract_text",
            json!({"ref": {"kind": "text", "value": "Learn more"}}),
        )
        .unwrap();
    assert_eq!(v["text"], "Learn more");
    // 通过 ref.css 提取（通用页没有 id/class，此处验证 css 选择器能解析）
    let v2 = s
        .tool_call(
            "extract_text",
            json!({"ref": {"kind": "xpath", "value": "//h1"}}),
        )
        .unwrap();
    assert_eq!(v2["text"], "Welcome to FastBrowser");
}

/// extract_text 不存在的 id → 报错。
#[test]
fn extract_text_missing_id_errors() {
    let s = sdk_open("https://example.com");
    assert!(s.tool_call("extract_text", json!({"id": "zz"})).is_err());
}

// ── 表单深层校验 ──────────────────────────────────────────────

/// fill_form 的非字符串值：应严格报错（与 deep_param_validation 哲学一致），
/// 而不是静默把数字/布尔转成空串。
#[test]
fn fill_form_rejects_non_string_values() {
    let s = sdk_open("https://example.com/login");
    for bad in [
        json!({"values": {"b": 123}}),
        json!({"values": {"b": true}}),
        json!({"values": {"b": ["x"]}}),
        json!({"values": {"b": {"x": 1}}}),
        json!({"values": {"b": null}}),
    ] {
        assert!(
            s.tool_call("fill_form", bad.clone()).is_err(),
            "fill_form should reject non-string: {bad}"
        );
    }
    // 字符串值正常
    let v = s
        .tool_call("fill_form", json!({"values": {"b": "alice"}}))
        .unwrap();
    assert_eq!(v["filled"][0], "b");
}

// ── 等待 / 断言的选择器语义 ───────────────────────────────────

#[test]
fn wait_and_assert_accept_css_selector_forms() {
    let s = sdk_open("https://example.com");
    // mock 元素有 placeholder/name 属性，但不含 id/class；
    // css 选择器按 tag 或 ref 值匹配。
    assert!(s
        .tool_call(
            "wait_for_element",
            json!({"selector": "button", "timeout_ms": 50})
        )
        .is_ok());
    assert!(s
        .tool_call(
            "wait_for_element",
            json!({"selector": "input", "timeout_ms": 50})
        )
        .is_ok());
    // 不存在的选择器 → 超时报错
    assert!(s
        .tool_call(
            "wait_for_element",
            json!({"selector": "textarea", "timeout_ms": 50})
        )
        .is_err());
    assert!(s
        .tool_call("assert_element_exists", json!({"selector": "a"}))
        .is_ok());
    assert!(s
        .tool_call("assert_element_exists", json!({"selector": "missing"}))
        .is_err());
}

// ── 标签页工具 ────────────────────────────────────────────────

#[test]
fn get_tab_by_url_contains() {
    let s = sdk();
    s.tool_call("new_tab", json!({"url": "https://example.com/search"}))
        .unwrap();
    let v = s
        .tool_call("get_tab", json!({"url_contains": "search"}))
        .unwrap();
    assert!(v["tab"]["url"].as_str().unwrap().contains("search"));
    // 无匹配 → 报错
    assert!(s
        .tool_call("get_tab", json!({"title_contains": "zzz-none"}))
        .is_err());
}

#[test]
fn duplicate_tab_preserves_url_and_activates() {
    let s = sdk_open("https://example.com/login");
    let dup = s.tool_call("duplicate_tab", json!({})).unwrap();
    let url = s.tool_call("get_current_url", json!({})).unwrap();
    assert_eq!(url["url"], "https://example.com/login");
    assert!(dup["tab"].is_number());
    assert_eq!(
        s.tool_call("list_tabs", json!({})).unwrap()["tabs"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn close_other_tabs_keeps_active() {
    let s = sdk_open("https://example.com");
    s.tool_call("new_tab", json!({"url": "https://example.com/search"}))
        .unwrap();
    s.tool_call("new_tab", json!({"url": "https://example.com/login"}))
        .unwrap();
    let v = s.tool_call("close_other_tabs", json!({})).unwrap();
    assert!(v["closed"].as_u64().unwrap() >= 2);
    assert_eq!(
        s.tool_call("list_tabs", json!({})).unwrap()["tabs"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn switch_tab_invalid_errors() {
    let s = sdk_open("https://example.com");
    assert!(s.tool_call("switch_tab", json!({"tab": 999})).is_err());
    assert!(s.tool_call("close_tab", json!({"tab": 999})).is_err());
}

// ── 会话 / cookie / storage ───────────────────────────────────

#[test]
fn cookie_clear_targets_domain_and_name() {
    let s = sdk_open("https://example.com");
    s.tool_call(
        "cookie_set",
        json!({"name": "a", "value": "1", "domain": "example.com"}),
    )
    .unwrap();
    s.tool_call(
        "cookie_set",
        json!({"name": "b", "value": "2", "domain": "example.com"}),
    )
    .unwrap();
    s.tool_call(
        "cookie_set",
        json!({"name": "a", "value": "3", "domain": "other.com"}),
    )
    .unwrap();

    // 只清指定 name + domain
    s.tool_call(
        "cookie_clear",
        json!({"name": "a", "domain": "example.com"}),
    )
    .unwrap();
    let cookies = s.tool_call("cookie_get", json!({})).unwrap();
    let pairs: Vec<(String, String)> = cookies["cookies"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            (
                c["name"].as_str().unwrap_or("").to_string(),
                c["domain"].as_str().unwrap_or("").to_string(),
            )
        })
        .collect();
    assert!(
        pairs.contains(&("b".into(), "example.com".into())),
        "b@example.com should remain"
    );
    assert!(
        !pairs.contains(&("a".into(), "example.com".into())),
        "a@example.com should be cleared"
    );
    // other.com 的 a 仍在
    assert!(
        pairs.contains(&("a".into(), "other.com".into())),
        "a@other.com should remain"
    );
}

#[test]
fn session_roundtrip_preserves_state_and_storage() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("multi.json").to_str().unwrap().to_string();

    let s = sdk_open("https://example.com");
    s.tool_call(
        "cookie_set",
        json!({"name": "sid", "value": "s1", "domain": "example.com"}),
    )
    .unwrap();
    s.tool_call("storage_set", json!({"key": "k1", "value": "v1"}))
        .unwrap();
    s.session_save(&path).unwrap();

    let s2 = sdk();
    s2.session_load(&path).unwrap();
    s2.open("https://example.com").unwrap();
    let c = s2
        .tool_call("cookie_get", json!({"domain": "example.com"}))
        .unwrap();
    assert_eq!(c["cookies"][0]["value"], "s1");
    let st = s2.tool_call("storage_get", json!({"key": "k1"})).unwrap();
    assert_eq!(st["value"], "v1");
}

#[test]
fn clear_state_resets_cookies_storage_and_extra_tabs() {
    let s = sdk_open("https://example.com");
    s.tool_call("new_tab", json!({"url": "https://example.com/search"}))
        .unwrap();
    s.tool_call(
        "cookie_set",
        json!({"name": "x", "value": "1", "domain": "example.com"}),
    )
    .unwrap();
    s.tool_call("storage_set", json!({"key": "x", "value": "1"}))
        .unwrap();

    let out = s.clear_state().unwrap();
    assert_eq!(out["cleared"], true);
    assert_eq!(out["closed_tabs"].as_u64().unwrap(), 1);
    assert_eq!(
        s.tool_call("list_tabs", json!({})).unwrap()["tabs"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        s.tool_call("cookie_get", json!({})).unwrap()["cookies"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    assert_eq!(
        s.tool_call("storage_get_all", json!({})).unwrap()["storage"]
            .as_object()
            .unwrap()
            .len(),
        0
    );
}

// ── 事件流 / 帧回调 ───────────────────────────────────────────

struct CountingSink(Arc<std::sync::atomic::AtomicU32>);
impl PageEventSink for CountingSink {
    fn on_page_event(&self, _t: fastbrowser::engine::TabId, _e: &PageEvent) {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}

struct FrameCounter(Arc<std::sync::atomic::AtomicU32>);
impl ViewFrameSink for FrameCounter {
    fn on_view_frame(&self, _t: fastbrowser::engine::TabId, _f: &ViewFrame) {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}

#[test]
fn frame_sink_receives_view_frames() {
    let counter = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let s = sdk();
    s.register_frame_sink(Arc::new(FrameCounter(counter.clone())));
    s.open("https://example.com").unwrap();
    let before = counter.load(std::sync::atomic::Ordering::SeqCst);
    let tab = s
        .runtime()
        .unwrap()
        .as_ref()
        .unwrap()
        .engine()
        .active_tab()
        .unwrap();
    s.runtime()
        .unwrap()
        .as_ref()
        .unwrap()
        .engine()
        .view_frame(tab)
        .unwrap();
    assert!(
        counter.load(std::sync::atomic::Ordering::SeqCst) > before,
        "view_frame should push to sink"
    );
}

#[test]
fn event_sink_receives_navigation_and_dom_events() {
    let counter = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let s = sdk();
    s.register_event_sink(Arc::new(CountingSink(counter.clone())));
    s.open("https://example.com").unwrap();
    let base = counter.load(std::sync::atomic::Ordering::SeqCst);
    s.tool_call("type", json!({"id": "e", "text": "hello"}))
        .unwrap(); // 触发 DomChanged
    assert!(counter.load(std::sync::atomic::Ordering::SeqCst) > base);
}

#[test]
fn drain_events_yields_navigation_events_after_open() {
    let s = sdk();
    s.open("https://example.com/login").unwrap();
    let tab = s
        .runtime()
        .unwrap()
        .as_ref()
        .unwrap()
        .engine()
        .active_tab()
        .unwrap();
    let evs = s
        .runtime()
        .unwrap()
        .as_ref()
        .unwrap()
        .engine()
        .drain_events(tab);
    assert!(evs
        .iter()
        .any(|e| matches!(e, PageEvent::NavigationStarted { .. })));
    assert!(evs.iter().any(|e| matches!(e, PageEvent::Loaded { .. })));
    assert!(evs
        .iter()
        .any(|e| matches!(e, PageEvent::TitleChanged { .. })));
}

// ── 引擎 / 配置 ───────────────────────────────────────────────

#[test]
fn mock_engine_capabilities_are_full() {
    let s = sdk();
    let rt_guard = s.runtime().unwrap();
    let rt = rt_guard.as_ref().unwrap();
    let caps = rt.engine().capabilities();
    assert!(caps.supports_cookies && caps.supports_storage && caps.supports_coordinate_input);
    assert!(caps.supports_cdp); // mock 声明 full，但实际无真实 CDP 端点
    let tab = rt
        .engine()
        .create_tab("about:blank", &Default::default())
        .unwrap();
    assert!(
        rt.engine().cdp_endpoint(tab).is_none(),
        "mock has no real cdp endpoint"
    );
}

#[test]
fn config_json_roundtrip_and_unknown_fields() {
    let cfg: Config = serde_json::from_str(
        r#"{
        "engine": "mock",
        "rendering_mode": "hosted",
        "viewport": {"width": 320, "height": 480, "device_scale_factor": 2.0},
        "user_agent": "test-ua",
        "unknown_field_ignored": 42
    }"#,
    )
    .unwrap();
    assert_eq!(cfg.rendering_mode, RenderingMode::Hosted);
    assert_eq!(cfg.viewport.unwrap().width, 320);
    assert_eq!(cfg.user_agent.as_deref(), Some("test-ua"));

    let out = serde_json::to_string(&cfg).unwrap();
    let back: Config = serde_json::from_str(&out).unwrap();
    assert_eq!(back.engine, "mock");
}

#[test]
fn engine_auto_falls_back_to_mock_without_cdp() {
    let s = Fastbrowser::new();
    s.init(Config {
        engine: "auto".into(),
        ..Config::default()
    })
    .unwrap();
    // 无 cdp_url 时的降级链：engine-cdp 已编译且本机有 bundled chromium → chromium；
    // 否则 → mock 兜底。两种都应能正常 open。
    let st = s.status();
    let engine = st["engine"].as_str().unwrap_or("");
    assert!(
        engine == "mock" || engine == "chromium" || engine == "webview",
        "unexpected auto engine {engine}"
    );
    // mock 对 about:blank 给出 Example Page；真实引擎仅需成功建标签
    let out = s.open("about:blank").unwrap();
    assert!(out["tab"].is_number());
    if engine == "mock" {
        assert_eq!(out["title"], "Example Page");
    }
}

#[test]
fn unsupported_engine_errors() {
    let s = Fastbrowser::new();
    let r = s.init(Config {
        engine: "banana".into(),
        ..Config::default()
    });
    assert!(r.is_err());
    let msg = r.unwrap_err().to_string();
    assert!(msg.contains("banana"));
}

// ── 渲染模式 / 视口 ───────────────────────────────────────────

#[test]
fn rendering_mode_hosted_reports_and_works() {
    let s = sdk();
    assert_eq!(s.status()["rendering_mode"], "headless");
    s.set_rendering_mode(RenderingMode::Hosted);
    assert_eq!(s.status()["rendering_mode"], "hosted");
    // 托管模式下快照/截图仍可用
    s.open("https://example.com").unwrap();
    let img = s.screenshot().unwrap();
    assert!(img.is_valid());
}

#[test]
fn viewport_setting_reflected_in_snapshot_and_screenshot() {
    let s = sdk_open("https://example.com");
    s.set_viewport(375, 667).unwrap();
    let snap = s.snapshot().unwrap();
    assert_eq!(snap.viewport.width, 375);
    assert_eq!(snap.viewport.height, 667);
    let img = s.screenshot().unwrap();
    assert_eq!(img.width, 375);
    assert_eq!(img.height, 667);
}

// ── 历史 ──────────────────────────────────────────────────────

#[test]
fn history_truncated_by_new_navigation() {
    let s = sdk_open("https://example.com");
    s.tool_call("navigate", json!({"url": "https://example.com/login"}))
        .unwrap();
    s.tool_call("navigate", json!({"url": "https://example.com/search"}))
        .unwrap();
    s.tool_call("back", json!({})).unwrap();
    // 回退后再导航 → 前进历史被截断
    s.tool_call("navigate", json!({"url": "https://example.com/register"}))
        .unwrap();
    let h = s.tool_call("get_history", json!({})).unwrap();
    let urls: Vec<&str> = h["history"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["url"].as_str())
        .collect();
    assert_eq!(urls.last().copied(), Some("https://example.com/register"));
    // register 之后再 forward 不应跳回 search
    let fwd = s.tool_call("forward", json!({})).unwrap();
    assert_eq!(fwd["url"], "https://example.com/register");
}

// ── 审计 ──────────────────────────────────────────────────────

#[test]
fn audit_captures_sdk_open_and_tool_results() {
    let s = sdk();
    s.open("https://example.com").unwrap();
    let log = s.audit();
    let arr = log.as_array().unwrap();
    assert!(arr.iter().any(|e| e["action"] == "open"));
    // open 的 result 应含 ok
    let open = arr.iter().find(|e| e["action"] == "open").unwrap();
    assert!(open["result"].get("ok").is_some());
    // params 记录了 url
    assert_eq!(open["params"]["url"], "https://example.com");
    // seq 单调
    let seqs: Vec<u64> = arr.iter().map(|e| e["seq"].as_u64().unwrap()).collect();
    for w in seqs.windows(2) {
        assert!(w[0] > w[1], "audit should be newest-first");
    }
}

// ── PDF ───────────────────────────────────────────────────────

#[test]
fn print_to_pdf_via_engine_is_valid_header() {
    let s = sdk_open("https://example.com");
    let tab = s
        .runtime()
        .unwrap()
        .as_ref()
        .unwrap()
        .engine()
        .active_tab()
        .unwrap();
    let pdf = s
        .runtime()
        .unwrap()
        .as_ref()
        .unwrap()
        .engine()
        .print_to_pdf(tab)
        .unwrap();
    assert!(pdf.starts_with(b"%PDF-"));
}

// ── 导航历史 / back / forward SDK 级 ──────────────────────────

#[test]
fn sdk_navigate_back_forward() {
    let s = sdk_open("https://example.com");
    s.navigate("https://example.com/login").unwrap();
    let rt_guard = s.runtime().unwrap();
    let back = rt_guard.as_ref().unwrap().engine();
    let tab = back.active_tab().unwrap();
    back.back(tab).unwrap();
    let url = s.tool_call("get_current_url", json!({})).unwrap();
    assert_eq!(url["url"], "https://example.com");
    back.forward(fastbrowser::engine::TabId(tab.as_u32()))
        .unwrap();
    let url = s.tool_call("get_current_url", json!({})).unwrap();
    assert_eq!(url["url"], "https://example.com/login");
}

// ── 压力：大量工具调用后的不变量 ──────────────────────────────

#[test]
fn many_tab_operations_keep_active_tab_valid() {
    let s = sdk_open("https://example.com");
    let mut ids = Vec::new();
    for i in 0..10 {
        let v = s
            .tool_call(
                "new_tab",
                json!({"url": format!("https://example.com/page{i}")}),
            )
            .unwrap();
        ids.push(v["tab"].as_u64().unwrap() as u32);
    }
    // 关闭一半
    for id in &ids[..5] {
        let _ = s.tool_call("close_tab", json!({"tab": id}));
    }
    let st = s.status();
    assert!(st["tabs"].as_u64().unwrap() <= 6);
    // 活动标签页必须仍在列表
    if let Some(active) = st["active_tab"].as_u64() {
        let tabs = s.tool_call("list_tabs", json!({})).unwrap();
        let list: Vec<u64> = tabs["tabs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["id"].as_u64().unwrap())
            .collect();
        assert!(list.contains(&active), "active {active} not in {list:?}");
    }
}

// ── 回归：事件缓冲有界（防 Agent 不 drain 时内存无限增长）──────

#[test]
fn event_buffer_is_bounded() {
    let s = sdk_open("https://example.com");
    let tab = s
        .runtime()
        .unwrap()
        .as_ref()
        .unwrap()
        .engine()
        .active_tab()
        .unwrap();
    // 制造超过上限的 DOM 事件（点击空白处 → DomChanged）
    let cap = fastbrowser::engine::MAX_BUFFERED_EVENTS;
    for _ in 0..(cap + 500) {
        s.runtime()
            .unwrap()
            .as_ref()
            .unwrap()
            .engine()
            .inject_event(
                tab,
                fastbrowser::engine::MouseEvent::click(
                    9999.0,
                    9999.0,
                    fastbrowser::engine::MouseButton::Left,
                )
                .into(),
            )
            .unwrap();
    }
    let evs = s
        .runtime()
        .unwrap()
        .as_ref()
        .unwrap()
        .engine()
        .drain_events(tab);
    assert!(evs.len() <= cap, "event buffer exceeded cap: {}", evs.len());
    assert_eq!(
        evs.len(),
        cap,
        "buffer should hold exactly the newest cap events"
    );
    // drain 后为空（可再次累积）
    assert!(s
        .runtime()
        .unwrap()
        .as_ref()
        .unwrap()
        .engine()
        .drain_events(tab)
        .is_empty());
}

// ── 长会话稳定性：大量工具调用后审计有界、内核仍可用 ───────────

#[test]
fn long_session_stays_bounded_and_usable() {
    let s = sdk_open("https://example.com");
    // 远超审计上限（500）的连续调用
    for _ in 0..1200 {
        let _ = s.tool_call("get_page_title", json!({}));
    }
    let n = s.audit().as_array().map(|a| a.len()).unwrap_or(0);
    assert!(
        n <= 500,
        "audit should stay bounded after long session, got {n}"
    );
    // 长会话后内核仍正常响应新调用
    let title = s.tool_call("get_page_title", json!({})).unwrap();
    assert_eq!(title["title"], "Example Page");
    // 快照仍可用（元素缓存未退化）
    let snap = s.snapshot().unwrap();
    assert!(!snap.interactive.is_empty());
}

// ── 回归：审计脱敏（敏感参数不外泄到日志）──────────────────────

#[test]
fn audit_redacts_sensitive_tool_params() {
    let s = sdk_open("https://example.com");
    // cookie_set 的 value 应脱敏
    s.tool_call(
        "cookie_set",
        json!({"name": "sid", "value": "super-secret", "domain": "example.com"}),
    )
    .unwrap();
    // fill_form 的 values 内容应脱敏
    s.tool_call(
        "fill_form",
        json!({"values": {"b": "alice", "c": "hunter2"}}),
    )
    .unwrap();
    // 非敏感动作保留原文
    s.tool_call("extract_links", json!({})).unwrap();

    let log = s.audit();
    let arr = log.as_array().unwrap();
    for e in arr {
        match e["action"].as_str().unwrap() {
            "cookie_set" => assert_eq!(e["params"]["value"], "***", "cookie value leaked"),
            "fill_form" => {
                assert_eq!(e["params"]["values"]["b"], "***");
                assert_eq!(e["params"]["values"]["c"], "***");
            }
            "extract_links" => {
                let s = serde_json::to_string(&e["params"]).unwrap();
                assert!(!s.contains("***"), "non-sensitive params over-redacted");
            }
            _ => {}
        }
    }
}

/// 审计不会因超大结果（如 screenshot 的 base64）而内存膨胀。
#[test]
fn audit_caps_oversized_results() {
    let s = sdk_open("https://example.com");
    // 大视口截图 → base64 结果远大于上限
    s.set_viewport(2000, 2000).unwrap();
    let shot = s.tool_call("screenshot", json!({})).unwrap();
    let b64_len = shot["base64"].as_str().map(|s| s.len()).unwrap_or(0);
    assert!(
        b64_len > fastbrowser::audit::MAX_AUDIT_JSON_BYTES,
        "setup: need big result"
    );

    let log = s.audit();
    let shot_entry = log
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["action"] == "screenshot")
        .unwrap();
    assert_eq!(
        shot_entry["result"]["ok"]["__truncated"], true,
        "oversized result not capped"
    );
    assert_eq!(shot_entry["result"]["ok"]["width"], 2000);
    let stored = serde_json::to_string(shot_entry).unwrap();
    assert!(
        stored.len() < fastbrowser::audit::MAX_AUDIT_JSON_BYTES * 2,
        "audit entry too large: {}",
        stored.len()
    );
}

// ── 回归：截图编码格式（png/jpeg 直出，不支持时回落 RGBA）─────

#[test]
fn screenshot_format_falls_back_to_rgba_on_mock() {
    let s = sdk_open("https://example.com");
    // mock 引擎无 capture_encoded → jpeg/png 请求回落为 RGBA（格式字段仍是 rgba）
    for fmt in ["jpeg", "png"] {
        let v = s.tool_call("screenshot", json!({"format": fmt})).unwrap();
        assert_eq!(
            v["format"], "rgba",
            "mock should fall back to rgba for {fmt}"
        );
        assert!(v["base64"].is_string());
    }
    // 默认 rgba 不变
    let v = s.tool_call("screenshot", json!({})).unwrap();
    assert_eq!(v["format"], "rgba");
    assert_eq!(v["width"], 800);
}

#[test]
fn snapshot_stale_flag_is_fresh_on_mock() {
    let s = sdk_open("https://example.com");
    let snap = s.snapshot().unwrap();
    assert!(!snap.meta.stale, "mock snapshot should never be stale");
}

// ── 新架构验证：per-tab 锁支持单实例并发多标签页 ──────────────

/// 引擎层：多线程并发操作【不同标签页】，互不阻塞且各标签页状态独立。
#[test]
fn per_tab_operations_are_concurrent_and_isolated() {
    use fastbrowser::engine::{BrowserEngine, TabId, TabOptions};
    use std::sync::Arc;

    let engine = Arc::new(fastbrowser::engines::mock::MockEngine::new());
    let tabs: Vec<TabId> = (0..6)
        .map(|i| {
            engine
                .create_tab(&format!("https://example.com/t{i}"), &TabOptions::default())
                .unwrap()
        })
        .collect();

    let threads: Vec<_> = tabs
        .iter()
        .enumerate()
        .map(|(i, &tab)| {
            let e = engine.clone();
            std::thread::spawn(move || {
                // 每个线程只在【自己的标签页】上操作 → per-tab 锁让它们真正并行
                for j in 0..150 {
                    let url = format!("https://example.com/page{i}-{j}");
                    e.navigate(tab, &url).unwrap();
                    let text = e.get_page_text(tab).unwrap();
                    assert!(text.contains("Welcome"), "wrong page text");
                }
                i
            })
        })
        .collect();

    for t in threads {
        t.join().unwrap();
    }
    // 各标签页最终 URL 独立、互不串扰
    for (i, &tab) in tabs.iter().enumerate() {
        let snap = engine.snapshot(tab).unwrap();
        assert!(
            snap.url.contains(&format!("page{i}")),
            "tab {i} url corrupted: {}",
            snap.url
        );
    }
}

/// SDK 层：一个 `Fastbrowser` 实例被多线程共享并发调用（RwLock 读锁可共享）。
#[test]
fn single_instance_concurrent_tool_calls() {
    use std::sync::Arc;

    let s = Arc::new(sdk());
    s.open("https://example.com").unwrap();
    for i in 0..4 {
        s.tool_call(
            "new_tab",
            json!({"url": format!("https://example.com/page{i}")}),
        )
        .unwrap();
    }

    let threads: Vec<_> = (0..4)
        .map(|i| {
            let s = s.clone();
            std::thread::spawn(move || {
                for _ in 0..80 {
                    let _ = s.tool_call("get_page_title", json!({}));
                    let _ = s.tool_call("list_tabs", json!({}));
                    let _ = s.tool_call(
                        "cookie_set",
                        json!({"name": "k", "value": format!("v{i}"), "domain": "example.com"}),
                    );
                    let _ = s.tool_call("status", json!({}));
                }
                i
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
    // 并发后不变量保持：实例仍可用
    let out = s.tool_call("list_tabs", json!({})).unwrap();
    assert!(out["tabs"].as_array().unwrap().len() >= 5);
}

/// 并发【跨标签页】写隔离：各线程只写自己的标签页，互不串扰。
#[test]
fn concurrent_cross_tab_writes_are_isolated() {
    use fastbrowser::engine::{BrowserEngine, Cookie, TabId, TabOptions};
    use std::sync::Arc;

    let engine = Arc::new(fastbrowser::engines::mock::MockEngine::new());
    let tabs: Vec<TabId> = (0..8)
        .map(|i| {
            engine
                .create_tab(&format!("https://example.com/t{i}"), &TabOptions::default())
                .unwrap()
        })
        .collect();

    let threads: Vec<_> = tabs
        .iter()
        .enumerate()
        .map(|(i, &tab)| {
            let e = engine.clone();
            std::thread::spawn(move || {
                for j in 0..100 {
                    // 只写【自己的】storage 与 cookie
                    e.storage_set(tab, "k", &format!("v{i}-{j}")).unwrap();
                    e.cookie_set(tab, &Cookie::new("sid", format!("s{i}-{j}"), "example.com"))
                        .unwrap();
                    // 读回自己的值必须一致（无跨线程覆盖）
                    let v = e.storage_get(tab, "k").unwrap().unwrap();
                    assert!(
                        v.starts_with(&format!("v{i}-")),
                        "tab {i} storage corrupted: {v}"
                    );
                    let c = e.cookie_get(tab, None).unwrap();
                    assert_eq!(c[0].value, format!("s{i}-{j}"), "tab {i} cookie corrupted");
                }
                i
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
    // 最终每个标签页只保留自己线程最后写入的值
    for (i, &tab) in tabs.iter().enumerate() {
        let v = engine.storage_get(tab, "k").unwrap().unwrap();
        assert!(v.starts_with(&format!("v{i}-")), "tab {i} leaked: {v}");
        let c = engine.cookie_get(tab, None).unwrap();
        assert!(
            c[0].value.starts_with(&format!("s{i}-")),
            "tab {i} cookie leaked"
        );
    }
}

/// 同一标签页被多线程并发操作：per-tab 锁保证串行、无死锁、无状态损坏。
#[test]
fn same_tab_concurrent_ops_are_serialized_safely() {
    use fastbrowser::engine::{BrowserEngine, TabOptions};
    use std::sync::Arc;

    let engine = Arc::new(fastbrowser::engines::mock::MockEngine::new());
    let tab = engine
        .create_tab("https://example.com", &TabOptions::default())
        .unwrap();
    let n_threads = 8;
    let iters = 100;

    let threads: Vec<_> = (0..n_threads)
        .map(|t| {
            let e = engine.clone();
            std::thread::spawn(move || {
                for j in 0..iters {
                    e.navigate(tab, &format!("https://example.com/t{t}-{j}"))
                        .unwrap();
                    let snap = e.snapshot(tab).unwrap();
                    assert!(!snap.url.is_empty());
                }
                t
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
    // 同一标签页并发后：引擎仍可用、URL 为某个线程的最后值（不损坏）
    let snap = engine.snapshot(tab).unwrap();
    assert!(!snap.url.is_empty());
    assert!(engine.list_tabs().len() == 1);
}

/// 全实例压力：多线程混合操作（导航/快照/存储/标签页），验证无死锁。
#[test]
fn concurrent_mixed_workload_no_deadlock() {
    use std::sync::Arc;

    let s = Arc::new(sdk());
    s.open("https://example.com").unwrap();
    for i in 0..6 {
        s.tool_call(
            "new_tab",
            json!({"url": format!("https://example.com/page{i}")}),
        )
        .unwrap();
    }

    let threads: Vec<_> = (0..6)
        .map(|i| {
            let s = s.clone();
            std::thread::spawn(move || {
                for j in 0..60 {
                    match j % 6 {
                        0 => {
                            let _ = s.tool_call(
                                "navigate",
                                json!({"url": format!("https://example.com/n{i}-{j}")}),
                            );
                        }
                        1 => {
                            let _ = s.snapshot();
                        }
                        2 => {
                            let _ = s.screenshot();
                        }
                        3 => {
                            let _ = s.tool_call(
                                "storage_set",
                                json!({"key": "k", "value": format!("v{i}-{j}")}),
                            );
                        }
                        4 => {
                            let _ = s.tool_call("extract_links", json!({}));
                        }
                        _ => {
                            let _ = s.tool_call("list_tabs", json!({}));
                        }
                    }
                }
                i
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
    // 无死锁 → 实例仍可正常操作
    assert!(s.tool_call("get_current_url", json!({})).is_ok());
}
