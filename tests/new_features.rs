// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! 新功能测试：Role 定位、ref 边界、可回放审计、能力过滤边界。
//! 全部走 mock 引擎，无需真实浏览器。

use fastbrowser::engine::{Capability, EngineCapabilities, RefKind};
use fastbrowser::sdk::Fastbrowser;
use fastbrowser::Config;
use serde_json::json;

fn sdk() -> Fastbrowser {
    let s = Fastbrowser::new();
    s.init(Config::default()).unwrap();
    s.open("https://example.com").unwrap();
    s
}

// ── Role 定位 ────────────────────────────────────────────────

#[test]
fn role_ref_click_locates_element() {
    let s = sdk();
    // mock 页面有 button（role="button"）与 link（role="link"）
    let r = s
        .tool_call("click", json!({"ref": {"kind": "role", "value": "button"}}))
        .unwrap();
    assert_eq!(r["clicked"], "button");
    let r = s
        .tool_call("click", json!({"ref": {"kind": "role", "value": "link"}}))
        .unwrap();
    assert_eq!(r["clicked"], "link");
}

#[test]
fn role_ref_is_case_insensitive() {
    let s = sdk();
    let r = s
        .tool_call("click", json!({"ref": {"kind": "role", "value": "BUTTON"}}))
        .unwrap();
    assert_eq!(r["clicked"], "BUTTON");
}

#[test]
fn role_ref_hover_returns_coords() {
    let s = sdk();
    let r = s
        .tool_call("hover", json!({"ref": {"kind": "role", "value": "button"}}))
        .unwrap();
    assert!(r["x"].is_number() && r["y"].is_number());
}

#[test]
fn role_ref_missing_errors_not_panic() {
    let s = sdk();
    let r = s.tool_call(
        "click",
        json!({"ref": {"kind": "role", "value": "nonexistent"}}),
    );
    assert!(r.is_err(), "a non-existent role should error");
}

#[test]
fn invalid_ref_kind_errors() {
    let s = sdk();
    let r = s.tool_call("click", json!({"ref": {"kind": "bogus", "value": "x"}}));
    assert!(r.is_err(), "an invalid ref.kind should error");
}

// ── 可回放审计 ───────────────────────────────────────────────

#[test]
fn export_replay_script_contains_actions() {
    let s = sdk();
    s.tool_call("get_page_title", json!({})).unwrap();
    s.tool_call("click", json!({"id": "c"})).unwrap();
    s.tool_call("type", json!({"id": "e", "text": "hello"}))
        .unwrap();

    let script = s.tool_call("export_replay", json!({})).unwrap()["script"]
        .as_str()
        .unwrap()
        .to_string();

    assert!(script.contains("import fastbrowser"));
    assert!(script.contains("b.open(\"https://example.com\")"));
    assert!(script.contains("get_page_title"));
    assert!(script.contains("click"));
    assert!(script.contains("type"));
    assert!(script.contains("b.shutdown()"));
}

#[test]
fn export_replay_skips_failed_actions() {
    let s = sdk();
    s.tool_call("get_page_title", json!({})).unwrap();
    let _ = s.tool_call("click", json!({"id": "zzz"})); // 失败：非法 id
    let script = s.tool_call("export_replay", json!({})).unwrap()["script"]
        .as_str()
        .unwrap()
        .to_string();
    // 失败动作（click zzz）不应出现在回放脚本里
    assert!(!script.contains("zzz"));
}

// ── 能力过滤边界 ─────────────────────────────────────────────

#[test]
fn capability_supports_matches_bits() {
    let full = EngineCapabilities::full();
    let js = EngineCapabilities::js_injection();

    assert!(full.supports(Capability::Cdp));
    assert!(full.supports(Capability::NetworkControl));

    assert!(!js.supports(Capability::Cdp));
    assert!(!js.supports(Capability::NetworkControl));
}

#[test]
fn ref_kind_has_role_variant() {
    // RefKind::Role 是可构造的引用（deserialize 已覆盖，这里验证枚举变体存在）
    let r = fastbrowser::engine::ElementRef::role("button");
    assert_eq!(r.kind, RefKind::Role);
    assert_eq!(r.value, "button");
}
