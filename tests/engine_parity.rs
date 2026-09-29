// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! 引擎语义不变量测试（契约规范 §3 / §9.3）。
//!
//! 覆盖 `navigate`（不伪造完成、目标 URL、wait_until 默认值）、`wait_for_load_state`、
//! `execute_js`、`set_viewport` 等跨引擎应一致的语义。CDP/webview 的宿主一致性另由
//! 平台自检覆盖（§4.3）。

use fastbrowser::sdk::Fastbrowser;
use fastbrowser::Config;
use serde_json::{json, Value};

fn sdk() -> Fastbrowser {
    let s = Fastbrowser::new();
    s.init(Config::default()).unwrap();
    s.open("https://example.com/login").unwrap();
    s
}

#[test]
fn navigate_defaults_to_waiting_for_load() {
    let s = sdk();
    let v = s
        .tool_call("navigate", json!({"url": "https://example.com/a"}))
        .unwrap();
    assert_eq!(v["ok"], json!(true));
    assert_eq!(v["url"], json!("https://example.com/a"));
    // 默认 wait_until=load → 结果带 waited/ready
    assert_eq!(v["waited"], json!("load"), "{v}");
    assert_eq!(v["ready"], json!("complete"), "{v}");
}

#[test]
fn navigate_wait_until_none_returns_immediately() {
    let s = sdk();
    let v = s
        .tool_call(
            "navigate",
            json!({"url": "https://example.com/b", "wait_until": "none"}),
        )
        .unwrap();
    assert_eq!(v["url"], json!("https://example.com/b"));
    assert!(v.get("waited").is_none(), "no wait fields when none: {v}");
}

#[test]
fn current_url_reflects_last_navigate() {
    let s = sdk();
    let _ = s.tool_call("navigate", json!({"url": "https://example.com/x"}));
    let v = s.tool_call("get_current_url", json!({})).unwrap();
    assert_eq!(v["url"], json!("https://example.com/x"), "{v}");
}

#[test]
fn wait_for_load_state_after_navigate_is_ready() {
    let s = sdk();
    let _ = s.tool_call("navigate", json!({"url": "https://example.com/c"}));
    let v = s.tool_call("wait_for_load_state", json!({})).unwrap();
    assert_eq!(v["state"], json!("load"));
    assert_eq!(v["ready"], json!("complete"), "{v}");
}

#[test]
fn execute_js_returns_value_for_expression() {
    let s = sdk();
    let v = s.tool_call("execute_js", json!({"script": "1+1"})).unwrap();
    assert_eq!(v["result"].as_f64(), Some(2.0), "{v}");
}

#[test]
fn set_viewport_ok() {
    let s = sdk();
    let v = s
        .tool_call("set_viewport", json!({"width": 1024, "height": 768}))
        .unwrap();
    assert_eq!(v["ok"], json!(true), "{v}");
    assert_eq!(v["width"], json!(1024));
}

/// navigate 不再伪造完成：目标地址被记录，且随后 wait 工具仍能正常返回。
#[test]
fn navigate_records_target_without_faking_completion() {
    let s = sdk();
    let _ = s.tool_call(
        "navigate",
        json!({"url": "https://example.com/target", "wait_until": "none"}),
    );
    let url = s.tool_call("get_current_url", json!({})).unwrap();
    assert_eq!(url["url"], json!("https://example.com/target"));
    // wait_for_navigation 不应因"伪造完成"而秒回错误状态；应正常返回。
    let nav = s.tool_call("wait_for_navigation", json!({"timeout_ms": 1000}));
    assert!(
        nav.is_ok(),
        "wait_for_navigation should not hang/fail: {nav:?}"
    );
    let _ = Value::Null;
}
