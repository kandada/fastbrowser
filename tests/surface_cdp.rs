// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! 真 Chromium 上的「AX 统一 + 几何关联」集成测试（feature `engine-cdp` + `surface`）。
//!
//! 验证：`ax_snapshot` 走引擎原生 AX 树（`Accessibility.getFullAXTree` +
//! `DOM.getBoxModel` 几何），产出 `web:<tab>:ax:<backendId>` 引用与几何；并可用
//! `ax_click` 以该 AX 引用做坐标点击，触发真实页面行为。
//!
//! 前置：本机存在 Chrome/Chromium（或 `CHROME_PATH`）。未找到时打印 SKIP。

#![cfg(all(feature = "engine-cdp", feature = "surface", feature = "heavy-tests"))]

mod common;

use std::time::{Duration, Instant};

use fastbrowser::sdk::Fastbrowser;
use fastbrowser::Config;
use serde_json::{json, Value};

const HTML: &str = r##"<!doctype html><html><head><title>AX Surface</title></head>
<body>
  <h1>AX Surface</h1>
  <button id="go" onclick="document.getElementById('out').textContent='clicked'">Go</button>
  <span id="out">initial</span>
</body></html>"##;

fn wait_until<F: FnMut() -> bool>(what: &str, timeout: Duration, mut f: F) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if f() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("timeout waiting for {what}");
}

/// 递归展平 ax_snapshot 的 `tree`。
fn flatten(node: &Value, out: &mut Vec<Value>) {
    if node.is_null() {
        return;
    }
    out.push(node.clone());
    if let Some(children) = node.get("children").and_then(Value::as_array) {
        for c in children {
            flatten(c, out);
        }
    }
}

fn init_surface_sdk(ws: &str, prefer_ax: bool) -> Fastbrowser {
    let sdk = Fastbrowser::new();
    let mut cfg = Config {
        engine: "chromium".into(),
        cdp_url: Some(ws.to_string()),
        ..Config::default()
    };
    cfg.surface.provider = "browser".into();
    cfg.surface.prefer_ax_tree = prefer_ax;
    sdk.init(cfg).unwrap();
    sdk
}

#[test]
fn ax_surface_snapshot_and_coordinate_click() {
    let Some(_) = common::find_chrome() else {
        eprintln!("SKIP: ax_surface_snapshot_and_coordinate_click (no chrome; set CHROME_PATH)");
        return;
    };
    let b = common::shared_browser(true).expect("launch headless chrome");
    let _guard = common::browser_guard(b);
    let ws = b.ws.clone();

    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("ax.html");
    std::fs::write(&file, HTML).unwrap();
    let url = common::file_url(&file);

    let sdk = init_surface_sdk(&ws, true);
    sdk.open(&url).unwrap();
    wait_until("page loaded", Duration::from_secs(15), || {
        sdk.tool_call("get_page_title", json!({}))
            .ok()
            .and_then(|v| v["title"].as_str().map(|s| s.contains("AX Surface")))
            .unwrap_or(false)
    });

    // ── AX 快照：应产出带几何的 `ax:` 引用 ──────────────────────────────
    let snap = sdk
        .tool_call("ax_snapshot", json!({"max_nodes": 300}))
        .unwrap();
    assert_eq!(snap["surface"]["kind"], "web");
    let mut nodes = Vec::new();
    flatten(&snap["tree"], &mut nodes);
    assert!(nodes.len() > 1, "AX tree should have nodes: {snap}");

    let button = nodes
        .iter()
        .find(|n| {
            n["role"].as_str().map(|r| r.eq_ignore_ascii_case("button")) == Some(true)
                && n["name"].as_str() == Some("Go")
        })
        .unwrap_or_else(|| panic!("button 'Go' not found in AX tree: {nodes:?}"));
    let ref_ = button["ref"].as_str().unwrap().to_string();
    assert!(ref_.contains(":ax:"), "expected an AX ref, got {ref_}");
    assert!(
        button["geometry"].is_object(),
        "AX node should carry geometry: {button}"
    );

    // ── 以 AX 引用做坐标点击（几何关联）─────────────────────────────────
    sdk.tool_call("ax_click", json!({ "ref": ref_ })).unwrap();
    wait_until("onclick effect", Duration::from_secs(5), || {
        sdk.tool_call(
            "execute_js",
            json!({"script": "document.getElementById('out').textContent"}),
        )
        .ok()
        .and_then(|v| v["result"].as_str().map(|s| s == "clicked"))
        .unwrap_or(false)
    });
    let out = sdk
        .tool_call(
            "execute_js",
            json!({"script": "document.getElementById('out').textContent"}),
        )
        .unwrap();
    assert_eq!(out["result"], "clicked");
}

#[test]
fn prefer_ax_tree_off_falls_back_to_interactive_snapshot() {
    let Some(_) = common::find_chrome() else {
        eprintln!("SKIP: prefer_ax_tree_off_falls_back_to_interactive_snapshot (no chrome)");
        return;
    };
    let b = common::shared_browser(true).expect("launch headless chrome");
    let _guard = common::browser_guard(b);
    let ws = b.ws.clone();

    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("ax.html");
    std::fs::write(&file, HTML).unwrap();
    let url = common::file_url(&file);

    let sdk = init_surface_sdk(&ws, false);
    sdk.open(&url).unwrap();
    wait_until("page loaded", Duration::from_secs(15), || {
        sdk.tool_call("get_page_title", json!({}))
            .ok()
            .and_then(|v| v["title"].as_str().map(|s| s.contains("AX Surface")))
            .unwrap_or(false)
    });

    let snap = sdk
        .tool_call("ax_snapshot", json!({"max_nodes": 300}))
        .unwrap();
    let mut nodes = Vec::new();
    flatten(&snap["tree"], &mut nodes);
    // 回退路径：引用是快照字母（`web:<tab>:a`），不是 `ax:`。
    let has_ax = nodes
        .iter()
        .filter_map(|n| n["ref"].as_str())
        .any(|r| r.contains(":ax:"));
    assert!(!has_ax, "prefer_ax_tree=false should not emit ax: refs");
}
