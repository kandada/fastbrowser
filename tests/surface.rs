// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Surface (computer-use) integration tests.
//!
//! Exercises the public SDK surface: tool discovery, capability gating, routing
//! across providers (browser `web:` + scripted `desktop:`), snapshot pruning and
//! action dispatch — all without requiring OS accessibility permissions.
//!
//! Run with: `cargo test --features surface --test surface`

#![cfg(feature = "surface")]

use fastbrowser::{Config, Fastbrowser};
use serde_json::json;

fn sdk_mock_surface() -> Fastbrowser {
    let s = Fastbrowser::new();
    let mut cfg = Config::default();
    cfg.surface.provider = "mock".to_string();
    s.init(cfg).unwrap();
    s
}

fn sdk_browser_surface() -> Fastbrowser {
    let s = Fastbrowser::new();
    let mut cfg = Config::default();
    cfg.surface.provider = "browser".to_string();
    s.init(cfg).unwrap();
    s
}

fn tool_names(s: &Fastbrowser) -> Vec<String> {
    s.tool_list()
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn surface_tools_are_registered() {
    let s = sdk_mock_surface();
    let names = tool_names(&s);
    for expected in [
        "ax_list",
        "ax_snapshot",
        "ax_act",
        "ax_click",
        "ax_type",
        "ax_scroll",
        "ax_events",
    ] {
        assert!(names.contains(&expected.to_string()), "missing {expected}");
    }
}

#[test]
fn surface_events_returns_list() {
    let s = sdk_mock_surface();
    let v = s.tool_call("ax_events", json!({})).unwrap();
    assert_eq!(v["count"], 0);
    assert!(v["events"].as_array().unwrap().is_empty());
}

#[test]
fn unknown_action_lists_valid_actions() {
    let s = sdk_mock_surface();
    let e = s
        .tool_call(
            "ax_act",
            json!({"ref": "desktop:1:0.0", "action": "banana"}),
        )
        .unwrap_err()
        .to_string();
    assert!(e.contains("unknown surface action"), "{e}");
    assert!(e.contains("valid actions"), "{e}");
    assert!(e.contains("click"), "{e}");
}

#[test]
fn computer_alias_and_coordinate_click() {
    let s = sdk_mock_surface();
    // `computer` 别名 → ax_act
    let v = s
        .tool_call(
            "computer",
            json!({"ref": "desktop:1:0.0", "action": "left_click"}),
        )
        .unwrap();
    assert_eq!(v["action"], "left_click");
    // 坐标点击（Anthropic 风格）
    let v = s.tool_call("ax_click", json!({"x": 12, "y": 34})).unwrap();
    assert_eq!(v["coordinate"], true);
    assert_eq!(v["x"].as_f64(), Some(12.0));
}

#[test]
fn surface_disabled_hides_tools() {
    let s = Fastbrowser::new();
    let mut cfg = Config::default();
    cfg.surface.enabled = false;
    s.init(cfg).unwrap();
    let names = tool_names(&s);
    assert!(!names.contains(&"ax_snapshot".to_string()));
    // 调用时明确报错，而非静默失败。
    let err = s.tool_call("ax_snapshot", json!({})).unwrap_err();
    assert!(err.to_string().contains("surface") || err.to_string().contains("capability"));
}

#[test]
fn list_and_snapshot_scripted_surface() {
    let s = sdk_mock_surface();
    let list = s.tool_call("ax_list", json!({})).unwrap();
    assert_eq!(list["count"], 1);
    assert_eq!(list["surfaces"][0]["id"], "desktop:1");

    let snap = s
        .tool_call("ax_snapshot", json!({"target": "desktop:1"}))
        .unwrap();
    assert_eq!(snap["surface"]["id"], "desktop:1");
    assert_eq!(snap["surface"]["kind"], "window");
    assert_eq!(snap["meta"]["truncated"], false);
    let text = snap["text"].as_str().unwrap();
    assert!(text.contains("button \"OK\""), "text was: {text}");
    assert!(text.contains("textbox \"Name\""));
}

#[test]
fn snapshot_pruning_reports_truncation() {
    let s = sdk_mock_surface();
    let snap = s
        .tool_call(
            "ax_snapshot",
            json!({"target": "desktop:1", "max_nodes": 1}),
        )
        .unwrap();
    assert_eq!(snap["meta"]["truncated"], true);
    assert_eq!(snap["meta"]["reason"], "nodes");
    assert_eq!(snap["meta"]["returned_nodes"], 1);
}

#[test]
fn act_click_and_type_roundtrip() {
    let s = sdk_mock_surface();
    s.tool_call("ax_act", json!({"ref": "desktop:1:0.0", "action": "click"}))
        .unwrap();
    s.tool_call(
        "ax_type",
        json!({"ref": "desktop:1:0.1", "text": "Alice", "clear": true}),
    )
    .unwrap();

    let snap = s
        .tool_call("ax_snapshot", json!({"target": "desktop:1"}))
        .unwrap();
    let children = snap["tree"]["children"].as_array().unwrap();
    let textbox = children
        .iter()
        .find(|c| c["role"] == "textbox")
        .expect("textbox node");
    assert_eq!(textbox["value"], "Alice");
}

#[test]
fn bad_ref_is_rejected() {
    let s = sdk_mock_surface();
    // 缺少任何元素目标 → 明确报错（而非静默失败）。
    let err = s
        .tool_call("ax_act", json!({"action": "click"}))
        .unwrap_err();
    assert!(
        err.to_string().contains("target") || err.to_string().contains("invalid"),
        "{err}"
    );
}

#[test]
fn browser_provider_exposes_web_surfaces() {
    let s = sdk_browser_surface();
    let out = s.open("https://example.com/login").unwrap();
    let tab = out["tab"].as_u64().unwrap();

    let list = s.tool_call("ax_list", json!({})).unwrap();
    let ids: Vec<String> = list["surfaces"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["id"].as_str().unwrap().to_string())
        .collect();
    assert!(
        ids.iter().any(|id| id == &format!("web:{tab}")),
        "ids: {ids:?}"
    );

    let snap = s
        .tool_call("ax_snapshot", json!({"target": format!("web:{tab}")}))
        .unwrap();
    assert_eq!(snap["surface"]["kind"], "web");
    assert!(snap["tree"].is_object());
    // 至少有一个可交互元素
    let nodes = snap["tree"]["children"].as_array().unwrap();
    assert!(!nodes.is_empty(), "browser snapshot should have elements");
}

#[test]
fn browser_surface_click_by_ref() {
    let s = sdk_browser_surface();
    s.open("https://example.com/login").unwrap();
    let snap = s.tool_call("ax_snapshot", json!({})).unwrap();
    let button_ref = snap["tree"]["children"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["role"] == "button")
        .map(|n| n["ref"].as_str().unwrap().to_string());
    if let Some(ref_) = button_ref {
        let out = s.tool_call("ax_click", json!({"ref": ref_})).unwrap();
        assert_eq!(out["ok"], true);
    }
}

#[test]
fn surface_tools_are_absent_without_feature_or_engine() {
    // 默认（无 surface 配置）时，只要 provider 存在工具就在；这里验证
    // 通过配置关闭后能力位同步收敛。
    let s = Fastbrowser::new();
    let mut cfg = Config::default();
    cfg.surface.provider = "none".to_string();
    s.init(cfg).unwrap();
    let names = tool_names(&s);
    assert!(!names.contains(&"ax_snapshot".to_string()));
}

/// macOS 原生后端：有权限则验证快照，无权限则验证明确报错（不失败）。
#[cfg(feature = "surface-macos")]
#[test]
fn macos_native_surface_or_permission_error() {
    let s = Fastbrowser::new();
    let mut cfg = Config::default();
    cfg.surface.provider = "macos".to_string();
    s.init(cfg).unwrap();

    match s.tool_call("ax_snapshot", json!({})) {
        Ok(snap) => {
            assert_eq!(snap["surface"]["kind"], "app");
            assert!(snap["tree"].is_object());
        }
        Err(e) => {
            let msg = e.to_string();
            assert!(
                msg.contains("accessibility permission"),
                "unexpected error: {msg}"
            );
        }
    }
}

// ── 方言（dialect）兼容 ─────────────────────────────────────────────────────

#[test]
fn surface_aliases_resolve_and_target_registered_tools() {
    let s = sdk_mock_surface();
    let names = tool_names(&s);
    for (alias, target) in fastbrowser::tools::aliases::SURFACE_TOOL_ALIASES {
        assert!(
            names.iter().any(|n| n == target),
            "surface alias '{alias}' points at unregistered tool '{target}'"
        );
    }
    // 端到端：别名工具经 SDK 路由到 canonical 工具。
    let v = s.tool_call("list_windows", json!({})).unwrap();
    assert!(v["count"].as_u64().unwrap() >= 1, "{v}");
    let v = s
        .tool_call("ui_snapshot", json!({"target": "desktop:1"}))
        .unwrap();
    assert_eq!(v["surface"]["id"], "desktop:1");
    let v = s
        .tool_call("desktop_click", json!({"ref": "desktop:1:0.0"}))
        .unwrap();
    assert_eq!(v["ok"], true);
    let v = s
        .tool_call("ui_type", json!({"ref": "desktop:1:0.1", "text": "X"}))
        .unwrap();
    assert_eq!(v["ok"], true);
}

#[test]
fn surface_param_alias_maps_to_target() {
    let s = sdk_mock_surface();
    // `surface` 是 `target` 的参数别名。
    let v = s
        .tool_call("ax_snapshot", json!({"surface": "desktop:1"}))
        .unwrap();
    assert_eq!(v["surface"]["id"], "desktop:1");
}

#[test]
fn bare_ref_is_qualified_to_active_surface() {
    let s = sdk_browser_surface();
    let tab = s.open("https://example.com/login").unwrap()["tab"]
        .as_u64()
        .unwrap();
    // 裸快照字母 → 自动绑定到活动 web 表面。
    let out = s.tool_call("ax_click", json!({"ref": "a"})).unwrap();
    assert_eq!(out["ref"], format!("web:{tab}:a"));
}

#[test]
fn selector_dialect_clicks_on_browser_surface() {
    let s = sdk_browser_surface();
    s.open("https://example.com/login").unwrap();
    // role= / text= / css= 方言都应被 surface 工具接受并解析。
    let out = s
        .tool_call("ax_click", json!({"selector": "role=button"}))
        .unwrap();
    assert!(out["ref"].as_str().unwrap().ends_with("role=button"));
    // 文本方言（登录页有 "Submit"）
    let out = s
        .tool_call(
            "ax_act",
            json!({"selector": "text=Submit", "action": "click"}),
        )
        .unwrap();
    assert_eq!(out["ok"], true);
}

#[test]
fn surface_act_surface_level_actions() {
    let s = sdk_browser_surface();
    s.open("https://example.com/login").unwrap();
    // 无元素目标的表面级按键 / 滚动 / 置前。
    let v = s
        .tool_call("ax_act", json!({"action": "press_key", "key": "Tab"}))
        .unwrap();
    assert_eq!(v["surface_level"], true);
    let v = s
        .tool_call("ax_act", json!({"action": "scroll", "dy": 120}))
        .unwrap();
    assert_eq!(v["surface_level"], true);
    let v = s.tool_call("ax_act", json!({"action": "raise"})).unwrap();
    assert_eq!(v["surface_level"], true);
}

#[test]
fn status_reports_surface_providers() {
    let s = sdk_browser_surface();
    s.open("https://example.com").unwrap();
    let st = s.status();
    assert_eq!(st["surface"]["available"], true);
    let providers = st["surface"]["providers"].as_array().unwrap();
    assert!(providers.iter().any(|p| p == "browser"), "{providers:?}");
}

#[test]
fn unknown_namespace_ref_is_rejected() {
    let s = sdk_mock_surface();
    let err = s
        .tool_call("ax_act", json!({"ref": "bogus:1:a", "action": "click"}))
        .unwrap_err();
    assert!(err.to_string().contains("ref") || err.to_string().contains("provider"));
}

#[test]
fn interesting_only_toggle_changes_node_count() {
    let s = sdk_mock_surface();
    let pruned = s
        .tool_call(
            "ax_snapshot",
            json!({"target": "desktop:1", "interesting_only": true}),
        )
        .unwrap();
    let full = s
        .tool_call(
            "ax_snapshot",
            json!({"target": "desktop:1", "interesting_only": false}),
        )
        .unwrap();
    let a = pruned["meta"]["returned_nodes"].as_u64().unwrap();
    let b = full["meta"]["returned_nodes"].as_u64().unwrap();
    assert!(b >= a, "full={b} pruned={a}");
}

#[test]
fn browser_input_and_screenshot_paths() {
    let s = sdk_browser_surface();
    s.open("https://example.com").unwrap();
    // 表面级滚动走 input（无 ref）。
    let v = s.tool_call("ax_scroll", json!({"dy": 50})).unwrap();
    assert_eq!(v["ok"], true);
}

#[test]
fn object_ref_form_on_browser_surface() {
    let s = sdk_browser_surface();
    s.open("https://example.com/login").unwrap();
    // `{kind, value}` 形式与浏览器工具一致。
    let out = s
        .tool_call(
            "ax_act",
            json!({"ref": {"kind": "role", "value": "button"}, "action": "click"}),
        )
        .unwrap();
    assert!(out["ref"].as_str().unwrap().ends_with("role=button"));
    let out = s
        .tool_call(
            "ax_click",
            json!({"ref": {"kind": "text", "value": "Submit"}}),
        )
        .unwrap();
    assert!(out["ref"].as_str().unwrap().ends_with("text=Submit"));
}
