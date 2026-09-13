// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! WebView 引擎端到端测试（feature `engine-webview`）。
//!
//! 用一个内存模拟的 `WebViewOps`（宿主 WebView 实现）驱动 `WebViewEngine`：
//! open → snapshot → click/type → cookie/storage → 多标签 → 截图。
//!
//! 注意：`register_webview_ops` 是进程级全局单例（真实宿主只注册一次），因此
//! 本文件内多个测试共用同一份 MockOps；MockOps 按 handle 维护 per-tab 状态，
//! 保证测试并行/串行都稳定。
//!
//! C ABI 注入路径见 `webview_cabi_e2e.rs`（独立进程，各自注册）。

#![cfg(feature = "engine-webview")]

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use fastbrowser::engine::host::WebViewOps;
use fastbrowser::engine::{Image, InputEvent, Result, ViewHandle, Viewport};
use fastbrowser::sdk::Fastbrowser;
use fastbrowser::Config;
use serde_json::{json, Value};

// ── 模拟宿主 WebView（按 handle 维护状态，线程安全）────────────

static HANDLE_SEQ: AtomicU64 = AtomicU64::new(100);

#[derive(Default)]
struct TabState {
    url: String,
    title: String,
}

/// 极简宿主 WebView 模拟：`runtime_extract_js()` 返回固定快照 JSON，
/// 动作脚本返回 "ok"，title/url 按 handle 查表。
struct MockOps {
    state: Mutex<HashMap<u64, TabState>>,
}

fn snapshot_json(title: &str, url: &str) -> String {
    json!({
        "elements": [
            {"id":"a","tag":"h1","text":title,"rect":{"x":0,"y":0,"width":100,"height":30},"visible":true},
            {"id":"b","tag":"input","text":"","value":"","input_type":"text","name":"username","placeholder":"Username","rect":{"x":0,"y":50,"width":200,"height":30},"visible":true},
            {"id":"c","tag":"button","text":"Submit","rect":{"x":0,"y":90,"width":100,"height":30},"visible":true},
            {"id":"d","tag":"a","text":"Register","href":url,"rect":{"x":0,"y":130,"width":80,"height":30},"visible":true}
        ],
        "meta": {"total":4,"truncated":false,"viewport_h":600,"scroll_h":600,"scroll_y":0},
        "frames": []
    })
    .to_string()
}

impl MockOps {
    fn new() -> Self {
        MockOps {
            state: Mutex::new(HashMap::new()),
        }
    }

    fn tab(&self, handle: u64) -> Result<std::sync::MutexGuard<'_, HashMap<u64, TabState>>> {
        let st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if !st.contains_key(&handle) {
            return Err(fastbrowser::engine::EngineError::new(
                fastbrowser::engine::ErrorKind::Plugin,
                format!("mock webview handle {handle} not found"),
            ));
        }
        Ok(st)
    }

    fn evaluate_inner(&self, handle: u64, script: &str) -> Result<Value> {
        let st = self.tab(handle)?;
        let t = st.get(&handle).unwrap();
        if script.contains("FB_EXTRACT_V3") {
            return Ok(json!(snapshot_json(&t.title, &t.url)));
        }
        if script.contains("document.title") {
            return Ok(json!(t.title.clone()));
        }
        if script.contains("location.href") {
            return Ok(json!(t.url.clone()));
        }
        if script.contains("document.body") && script.contains("innerText") {
            return Ok(json!("Sign in\nSubmit\nRegister\n"));
        }
        if script.contains("document.images") {
            return Ok(json!([]));
        }
        if script.contains("querySelector('table')") {
            return Ok(json!([]));
        }
        if script.contains("__fbFind") {
            return Ok(json!("ok"));
        }
        Ok(json!(script.to_string()))
    }
}

impl WebViewOps for MockOps {
    fn create_webview(&self, url: &str, _viewport: Viewport) -> Result<u64> {
        let handle = HANDLE_SEQ.fetch_add(1, Ordering::SeqCst);
        let title = if url.contains("login") {
            "Login"
        } else {
            "Generic"
        }
        .to_string();
        self.state.lock().unwrap_or_else(|e| e.into_inner()).insert(
            handle,
            TabState {
                url: url.to_string(),
                title,
            },
        );
        Ok(handle)
    }
    fn destroy_webview(&self, handle: u64) -> Result<()> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&handle);
        Ok(())
    }
    fn evaluate_js(&self, handle: u64, script: &str) -> Result<Value> {
        self.evaluate_inner(handle, script)
    }
    fn navigate_webview(&self, handle: u64, url: &str) -> Result<()> {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(t) = st.get_mut(&handle) {
            t.url = url.to_string();
        }
        Ok(())
    }
    fn screenshot_webview(&self, _handle: u64) -> Result<Image> {
        Ok(Image::new(
            2,
            2,
            vec![
                0u8, 255, 0, 255, 0, 0, 255, 255, 255, 0, 0, 255, 0, 255, 0, 255,
            ],
        ))
    }
    fn set_viewport_webview(&self, _handle: u64, _viewport: Viewport) -> Result<()> {
        Ok(())
    }
    fn native_view(&self, handle: u64) -> Result<ViewHandle> {
        Ok(ViewHandle::Native(handle))
    }
    fn dispatch_event(&self, _handle: u64, _event: &InputEvent) -> Result<()> {
        Ok(())
    }
    fn go_back(&self, _handle: u64) -> Result<()> {
        Ok(())
    }
    fn go_forward(&self, _handle: u64) -> Result<()> {
        Ok(())
    }
}

/// 进程内注册一次 MockOps（其余测试复用）。
fn shared_ops() -> Arc<dyn WebViewOps> {
    static OPS: OnceLock<Arc<dyn WebViewOps>> = OnceLock::new();
    OPS.get_or_init(|| {
        let ops: Arc<dyn WebViewOps> = Arc::new(MockOps::new());
        let _ = fastbrowser::sdk::plugin::register_webview_ops(ops.clone());
        ops
    })
    .clone()
}

fn sdk_with_ops() -> Fastbrowser {
    let _ops = shared_ops();
    let s = Fastbrowser::new();
    let cfg = Config {
        engine: "webview".into(),
        ..Config::default()
    };
    let _ = s.init(cfg);
    s
}

#[test]
fn webview_open_snapshot_and_actions() {
    let s = sdk_with_ops();
    assert!(
        s.is_initialized(),
        "webview engine should init with registered ops"
    );
    let out = s.open("https://example.com/login").unwrap();
    assert!(out["tab"].is_number());

    let snap = s.snapshot().unwrap();
    assert_eq!(snap.title, "Login");
    assert!(snap.interactive.len() >= 4);
    let input = snap
        .interactive
        .iter()
        .find(|e| e.tag == "input")
        .unwrap()
        .id;
    assert_eq!(input, 'b');

    // 动作脚本走宿主桥（返回 ok）
    s.tool_call("click", json!({"id": "c"})).unwrap();
    s.tool_call("type", json!({"id": "b", "text": "alice"}))
        .unwrap();
    s.tool_call("select_option", json!({"id": "b", "value": "x"}))
        .unwrap_or_default();

    // 引擎能力位：JS 注入型（无 CDP / 无坐标输入 / 有存储与 cookie）
    let caps = s
        .runtime()
        .unwrap()
        .as_ref()
        .unwrap()
        .engine()
        .capabilities();
    assert!(!caps.supports_cdp);
    assert!(!caps.supports_coordinate_input);
    assert!(caps.supports_dom_injection);
    assert!(caps.supports_cookies && caps.supports_storage);
    assert!(s
        .runtime()
        .unwrap()
        .as_ref()
        .unwrap()
        .engine()
        .cdp_endpoint(fastbrowser::engine::TabId(1))
        .is_none());
}

#[test]
fn webview_cookie_storage_and_multi_tab() {
    let s = sdk_with_ops();
    assert!(s.is_initialized());
    s.open("https://example.com/login").unwrap();
    // cookie / storage 由引擎 overlay 维护
    s.tool_call(
        "cookie_set",
        json!({"name": "sid", "value": "abc", "domain": "example.com"}),
    )
    .unwrap();
    let c = s
        .tool_call("cookie_get", json!({"domain": "example.com"}))
        .unwrap();
    assert_eq!(c["cookies"][0]["value"], "abc");
    s.tool_call("storage_set", json!({"key": "t", "value": "v"}))
        .unwrap();
    let v = s.tool_call("storage_get", json!({"key": "t"})).unwrap();
    assert_eq!(v["value"], "v");

    // 多标签（按 handle 隔离状态 → 各标签 url 独立）
    let t2 = s
        .tool_call("new_tab", json!({"url": "https://example.com/other"}))
        .unwrap();
    let t2id = t2["tab"].as_u64().unwrap() as u32;
    let tabs = s.tool_call("list_tabs", json!({})).unwrap();
    assert!(tabs["tabs"].as_array().unwrap().len() >= 2);
    s.tool_call("switch_tab", json!({"tab": t2id})).unwrap();
    let url = s.tool_call("get_current_url", json!({})).unwrap();
    assert!(url["url"].as_str().unwrap().contains("/other"));
    s.tool_call("close_tab", json!({"tab": t2id})).unwrap();

    // 截图 / 视图
    let img = s.screenshot().unwrap();
    assert!(img.is_valid());
    assert!(s.get_view().is_some());
}

#[test]
fn webview_unsupported_network_control_errors() {
    let s = sdk_with_ops();
    assert!(s.is_initialized());
    s.open("https://example.com").unwrap();
    // 移动端无网络控制能力 → 优雅报错（Unsupported）
    let r = s.tool_call("block_request", json!({"patterns": ["*ads*"]}));
    assert!(r.is_err());
    let e = r.unwrap_err();
    assert_eq!(e.kind.to_string(), "unsupported");
    let r = s.tool_call("fulfill_request", json!({"request_id": "x"}));
    assert!(r.is_err());
}

/// 引擎名 / info / 能力集走 SDK 出口。
#[test]
fn webview_engine_info_and_status() {
    let s = sdk_with_ops();
    assert!(s.is_initialized());
    let info = s.get_info();
    assert_eq!(info.engine, "webview");
    let st = s.status();
    assert_eq!(st["engine"], "webview");
    let _ = s.open("https://example.com/login").unwrap();
    assert!(s.get_info().tabs >= 1);
}

/// 能力位驱动工具收敛：webview（js_injection）隐藏网络控制与 CDP 独占工具。
#[test]
fn webview_tool_list_filters_by_capability() {
    let s = sdk_with_ops();
    assert!(s.is_initialized());

    let list = s.tool_list();
    let names: Vec<String> = list
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t["name"].as_str().map(|n| n.to_string()))
        .collect();

    for hidden in [
        "block_request",
        "intercept_request",
        "list_pending_requests",
        "fulfill_request",
        "modify_response",
        "continue_request",
        "abort_request",
        "save_as_pdf",
        "screenshot",
        "screenshot_element",
        "set_touch_emulation",
        "set_geolocation",
        "set_timezone",
        "set_basic_auth",
    ] {
        assert!(
            !names.iter().any(|n| n == hidden),
            "{hidden} should be hidden"
        );
    }
    // 常规工具仍在
    assert!(names.iter().any(|n| n == "click"));
    assert!(names.iter().any(|n| n == "navigate"));
    assert!(names.iter().any(|n| n == "execute_js"));

    // 数量 = 全部(96) - 14
    assert_eq!(s.tool_count(), 96 - 14);
}
