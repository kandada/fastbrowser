// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! 真 Chromium（CDP 引擎）工具覆盖：把此前只在 mock 上测过的工具，在真实浏览器上
//! 走一遍（导航/标签/交互/提取/等待/断言/cookie/storage/设备/网络/杂项）。
//!
//! 未找到浏览器时 SKIP。

#![cfg(all(feature = "engine-cdp", feature = "heavy-tests"))]

mod common;

use fastbrowser::sdk::Fastbrowser;
use fastbrowser::Config;
use serde_json::{json, Value};

const HTML: &str = r##"<!doctype html><html><head><title>ToolsCDP</title></head><body>
<h1 id="h">Heading</h1>
<input id="q" value="">
<input id="cb" type="checkbox">
<input id="rd" type="radio" name="r">
<select id="sel"><option value="a">A</option><option value="b">B</option></select>
<input id="file" type="file">
<button id="go" onclick="document.getElementById('out').textContent='clicked'">Go</button>
<a id="lnk" href="#next">Next</a>
<img id="img" src="data:image/gif;base64,R0lGODlhAQABAAAAACwAAAAAAQABAAA=" alt="px">
<table id="t"><tr><td>Name</td><td>Age</td></tr><tr><td>Al</td><td>9</td></tr></table>
<span id="out">initial</span>
</body></html>"##;

/// Minimal single-page HTTP server so cookies / storage / network work
/// (file:// origins cannot set cookies and block localStorage in Chrome).
fn serve_http() -> u16 {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::OnceLock;
    static PORT: OnceLock<u16> = OnceLock::new();
    *PORT.get_or_init(|| {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let mut s = match stream {
                    Ok(s) => s,
                    Err(_) => continue,
                };
                let mut buf = [0u8; 2048];
                let _ = s.read(&mut buf);
                let body = HTML.as_bytes();
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = s.write_all(resp.as_bytes());
                let _ = s.write_all(body);
                let _ = s.flush();
            }
        });
        port
    })
}

fn setup() -> Option<(
    Fastbrowser,
    common::BrowserGuard,
    String,
    std::sync::MutexGuard<'static, ()>,
)> {
    static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let lock = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let b = common::shared_browser(true).ok()?;
    let guard = common::browser_guard(b);
    let ws = b.ws.clone();
    let s = Fastbrowser::new();
    s.init(Config {
        engine: "chromium".into(),
        cdp_url: Some(ws),
        ..Config::default()
    })
    .ok()?;
    let url = format!("http://127.0.0.1:{}/tools.html", serve_http());
    Some((s, guard, url, lock))
}

fn call(s: &Fastbrowser, name: &str, args: Value) -> Value {
    s.tool_call(name, args)
        .unwrap_or_else(|e| panic!("tool '{name}' failed: {e}"))
}

#[test]
fn cdp_navigation_tabs_and_extraction() {
    let Some((s, _g, url, _lock)) = setup() else {
        eprintln!("SKIP: no chrome");
        return;
    };
    call(&s, "navigate", json!({"url": url}));
    // navigation meta
    assert!(call(&s, "get_current_url", json!({}))["url"].is_string());
    assert_eq!(
        call(&s, "get_page_title", json!({}))["title"],
        json!("ToolsCDP")
    );
    assert!(call(&s, "get_page_meta", json!({}))["meta"].is_object());
    assert!(call(&s, "get_history", json!({}))["history"].is_array());
    // extraction
    assert!(call(&s, "extract_text", json!({}))["text"]
        .as_str()
        .unwrap()
        .contains("Heading"));
    assert!(call(&s, "extract_html", json!({}))["html"]
        .as_str()
        .unwrap()
        .contains("<h1"));
    assert!(call(&s, "extract_links", json!({}))["links"].is_array());
    assert!(call(&s, "extract_images", json!({"all": true}))["images"].is_array());
    assert!(call(&s, "extract_table", json!({}))["table"].is_array());
    assert!(call(&s, "extract_json", json!({"script": "1+1"}))["result"].is_number());
    assert!(
        call(&s, "find_elements", json!({"selector": "input"}))["count"]
            .as_u64()
            .unwrap()
            >= 4
    );
    assert!(call(&s, "search", json!({"query": "Heading"}))["count"].is_number());
    assert_eq!(
        call(&s, "get_element_info", json!({"selector": "#go"}))["tag"],
        json!("button")
    );
    assert!(
        call(&s, "get_element_text", json!({"selector": "#h"}))["text"]
            .as_str()
            .unwrap()
            .contains("Heading")
    );
    assert!(call(&s, "get_attributes", json!({"selector": "#h"}))["attrs"].is_object());
    assert!(call(&s, "get_accessibility_tree", json!({})).is_object());
    assert!(call(&s, "evaluate_xpath", json!({"expr": "//h1"}))["result"].is_array());
    // history + reload on the same tab (before adding tabs)
    call(&s, "navigate", json!({"url": format!("{url}#next")}));
    let _ = call(&s, "reload", json!({}));
    call(&s, "back", json!({}));
    call(&s, "forward", json!({}));
    let _ = call(&s, "stop", json!({}));
    // tabs
    let t = call(&s, "new_tab", json!({"url": url}));
    let tab = t["tab"].as_u64().unwrap();
    assert!(call(&s, "list_tabs", json!({}))["tabs"].is_array());
    let _ = call(&s, "switch_tab", json!({"tab": tab}));
    assert!(call(&s, "get_active_tab", json!({}))["tab"].is_number());
    let _ = call(&s, "get_tab", json!({"tab": tab}));
    let _ = call(&s, "close_tab", json!({"tab": tab}));
}

#[test]
fn cdp_interaction_tools() {
    let Some((s, _g, url, _lock)) = setup() else {
        eprintln!("SKIP: no chrome");
        return;
    };
    call(&s, "navigate", json!({"url": url}));
    // click flips #out
    call(&s, "click", json!({"selector": "#go"}));
    assert_eq!(
        call(
            &s,
            "execute_js",
            json!({"script": "document.getElementById('out').textContent"})
        )["result"],
        json!("clicked")
    );
    let _ = call(&s, "dblclick", json!({"selector": "#go"}));
    let _ = call(&s, "right_click", json!({"selector": "#go"}));
    let _ = call(&s, "hover", json!({"selector": "#go"}));
    // type + clear_input
    call(&s, "type", json!({"selector": "#q", "text": "hello"}));
    assert_eq!(
        call(
            &s,
            "execute_js",
            json!({"script": "document.getElementById('q').value"})
        )["result"],
        json!("hello")
    );
    call(&s, "clear_input", json!({"selector": "#q"}));
    assert_eq!(
        call(
            &s,
            "execute_js",
            json!({"script": "document.getElementById('q').value"})
        )["result"],
        json!("")
    );
    let _ = call(&s, "focus", json!({"selector": "#q"}));
    let _ = call(&s, "press", json!({"key": "Tab"}));
    let _ = call(&s, "blur", json!({}));
    // select / checkbox / radio
    call(
        &s,
        "select_option",
        json!({"selector": "#sel", "value": "b"}),
    );
    assert_eq!(
        call(
            &s,
            "execute_js",
            json!({"script": "document.getElementById('sel').value"})
        )["result"],
        json!("b")
    );
    call(&s, "checkbox", json!({"selector": "#cb", "checked": true}));
    assert_eq!(
        call(
            &s,
            "execute_js",
            json!({"script": "document.getElementById('cb').checked"})
        )["result"],
        json!(true)
    );
    call(&s, "radio", json!({"selector": "#rd"}));
    let _ = call(&s, "scroll", json!({"dy": 50}));
    let _ = call(&s, "inject_css", json!({"css": "body{outline:none}"}));
}

#[test]
fn cdp_wait_and_assert() {
    let Some((s, _g, url, _lock)) = setup() else {
        eprintln!("SKIP: no chrome");
        return;
    };
    call(&s, "navigate", json!({"url": url}));
    let _ = call(
        &s,
        "wait_for_load_state",
        json!({"state": "load", "timeout_ms": 5000}),
    );
    let _ = call(&s, "wait_for_navigation", json!({"timeout_ms": 2000}));
    call(
        &s,
        "wait_for_element",
        json!({"selector": "#go", "timeout_ms": 3000}),
    );
    call(
        &s,
        "wait_for_text",
        json!({"text": "Heading", "timeout_ms": 3000}),
    );
    call(
        &s,
        "wait_for_condition",
        json!({"script": "!!document.getElementById('go')", "timeout_ms": 3000}),
    );
    let _ = call(&s, "assert_url_contains", json!({"contains": "tools.html"}));
    let _ = call(&s, "assert_title", json!({"contains": "ToolsCDP"}));
    let _ = call(&s, "assert_text_contains", json!({"text": "Heading"}));
    let _ = call(&s, "assert_element_exists", json!({"selector": "#go"}));
}

#[test]
fn cdp_cookies_and_storage() {
    let Some((s, _g, url, _lock)) = setup() else {
        eprintln!("SKIP: no chrome");
        return;
    };
    call(&s, "navigate", json!({"url": url}));
    let _ = call(&s, "cookie_set", json!({"name": "k", "value": "v"}));
    let _ = call(&s, "cookie_set", json!({"name": "k2", "value": "v2"}));
    assert!(call(&s, "cookie_get", json!({}))["cookies"].is_array());
    let _ = call(&s, "cookie_get", json!({}));
    let _ = call(&s, "clear_cookies", json!({}));
    let _ = call(&s, "cookie_clear", json!({}));
    let _ = call(&s, "storage_set", json!({"key": "a", "value": "1"}));
    let _ = call(&s, "storage_get", json!({"key": "a"}));
    let _ = call(&s, "storage_get_all", json!({}));
    let _ = call(&s, "clear_storage", json!({}));
}

#[test]
fn cdp_device_and_network() {
    let Some((s, _g, url, _lock)) = setup() else {
        eprintln!("SKIP: no chrome");
        return;
    };
    call(&s, "navigate", json!({"url": url}));
    let _ = call(&s, "set_viewport", json!({"width": 800, "height": 600}));
    let _ = call(&s, "set_touch_emulation", json!({"enabled": true}));
    let _ = call(
        &s,
        "set_geolocation",
        json!({"latitude": 1.0, "longitude": 2.0}),
    );
    let _ = call(&s, "set_timezone", json!({"timezone_id": "Asia/Shanghai"}));
    let _ = call(
        &s,
        "set_basic_auth",
        json!({"username": "u", "password": "p"}),
    );
    // network interception (CDP-only)
    let _ = call(
        &s,
        "block_request",
        json!({"patterns": ["*.png"], "enabled": true}),
    );
    let _ = call(
        &s,
        "intercept_request",
        json!({"patterns": ["*.jpg"], "enabled": true}),
    );
    let _ = call(&s, "list_pending_requests", json!({}));
    let _ = call(
        &s,
        "block_request",
        json!({"patterns": ["*.png"], "enabled": false}),
    );
    let _ = call(
        &s,
        "intercept_request",
        json!({"patterns": ["*.jpg"], "enabled": false}),
    );
}

#[test]
fn cdp_screenshot_pdf_dialog_misc() {
    let Some((s, _g, url, _lock)) = setup() else {
        eprintln!("SKIP: no chrome");
        return;
    };
    call(&s, "navigate", json!({"url": url}));
    let shot = call(&s, "screenshot", json!({}));
    assert!(shot["width"].as_u64().unwrap_or(0) > 0, "{shot}");
    let _ = call(&s, "screenshot_element", json!({"selector": "#h"}));
    let pdf = std::env::temp_dir().join("fb_tools_test.pdf");
    let _ = call(&s, "save_as_pdf", json!({"path": pdf.to_string_lossy()}));
    let _ = call(&s, "pending_dialog", json!({}));
    let _ = call(&s, "get_performance_metrics", json!({}));
    let _ = call(&s, "get_selected_text", json!({}));
    let _ = call(&s, "get_focused_element", json!({}));
    let _ = call(&s, "export_replay", json!({}));
    let _ = call(&s, "done", json!({"answer": "ok"}));
    let _ = call(&s, "close_other_tabs", json!({}));
    let _ = call(&s, "duplicate_tab", json!({}));
}
