// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! 工具契约测试：对全部内置工具逐一调用，断言「不崩溃 + 输出形态符合文档」。
//!
//! 这是最强的一层"进一步测试"——每个工具都被真实走一遍（mock 引擎），
//! 而不是只测示例。分类：
//! - `ok`：预期成功（返回无 `error` 字段），断言文档声明的关键输出键存在；
//! - `err`：预期优雅失败（mock 引擎无该能力，返回标准 error JSON）。

use fastbrowser::sdk::Fastbrowser;
use fastbrowser::Config;
use serde_json::{json, Value};

fn sdk() -> Fastbrowser {
    let s = Fastbrowser::new();
    s.init(Config::default()).unwrap();
    s.open("https://example.com/login").unwrap();
    s
}

/// (工具名, 参数, 预期, 输出中必须存在的键)
const CONTRACT: &[(&str, &str, bool, &[&str])] = &[
    // ── 导航 ──
    (
        "navigate",
        r#"{"url":"https://example.com/search"}"#,
        true,
        &["url", "title"],
    ),
    ("back", "{}", true, &["url"]),
    ("forward", "{}", true, &["url"]),
    ("reload", "{}", true, &["ok"]),
    ("stop", "{}", true, &["ok"]),
    ("get_history", "{}", true, &["history", "count"]),
    // ── 交互 ──
    ("click", r#"{"id":"g"}"#, true, &["clicked"]),
    (
        "click_coords",
        r#"{"x":120,"y":80}"#,
        true,
        &["x", "y", "clicked"],
    ),
    ("dblclick", r#"{"id":"d"}"#, true, &["clicked"]),
    ("right_click", r#"{"id":"g"}"#, true, &["clicked", "button"]),
    ("type", r#"{"id":"b","text":"alice"}"#, true, &["typed"]),
    ("press", r#"{"key":"Enter"}"#, true, &["key"]),
    ("hover", r#"{"id":"g"}"#, true, &["x", "y"]),
    ("drag", r#"{"from":"d","to":"a"}"#, true, &["from", "to"]),
    ("scroll", r#"{"dy":120}"#, true, &["dy"]),
    (
        "swipe",
        r#"{"from_x":100,"from_y":200,"to_x":100,"to_y":50}"#,
        true,
        &["ok"],
    ),
    ("focus", r#"{"id":"b"}"#, true, &["focused"]),
    ("blur", "{}", true, &["blurred"]),
    ("clear_input", r#"{"id":"b"}"#, true, &["cleared"]),
    ("send_keys", r#"{"keys":"Ctrl+a"}"#, true, &["keys", "ok"]),
    // ── 内容提取 ──
    ("extract_text", "{}", true, &["text"]),
    ("extract_html", "{}", true, &["html"]),
    ("extract_links", "{}", true, &["links"]),
    ("extract_images", "{}", true, &["images"]),
    ("extract_table", "{}", true, &["table"]),
    (
        "extract_json",
        r#"{"script":"document.title"}"#,
        true,
        &["result"],
    ),
    (
        "search",
        r#"{"query":"Welcome","limit":5}"#,
        true,
        &["matches", "count"],
    ),
    (
        "find_elements",
        r#"{"selector":"a"}"#,
        true,
        &["elements", "count"],
    ),
    // ── 等待/断言 ──
    (
        "wait_for_element",
        r#"{"selector":"button","timeout_ms":100}"#,
        true,
        &["found"],
    ),
    (
        "wait_for_navigation",
        r#"{"timeout_ms":100}"#,
        true,
        &["navigated"],
    ),
    (
        "wait_for_load_state",
        r#"{"state":"load","timeout_ms":1000}"#,
        true,
        &["state", "ready"],
    ),
    (
        "wait_for_condition",
        r#"{"script":"document.title==='Login'","timeout_ms":100}"#,
        true,
        &["condition"],
    ),
    (
        "wait_for_text",
        r#"{"text":"Register","timeout_ms":100}"#,
        true,
        &["found"],
    ),
    (
        "assert_element_exists",
        r#"{"selector":"a"}"#,
        true,
        &["exists"],
    ),
    (
        "assert_text_contains",
        r#"{"text":"Register"}"#,
        true,
        &["contains"],
    ),
    (
        "assert_url_contains",
        r#"{"contains":"example.com"}"#,
        true,
        &["ok", "url"],
    ),
    (
        "assert_title",
        r#"{"contains":"Login"}"#,
        true,
        &["ok", "title"],
    ),
    // ── 表单 ──
    (
        "fill_form",
        r#"{"values":{"b":"alice","c":"secret"}}"#,
        true,
        &["filled"],
    ),
    (
        "select_option",
        r#"{"id":"e","value":"pro"}"#,
        true,
        &["selected"],
    ),
    (
        "upload_file",
        r#"{"id":"a","paths":["/tmp/x.pdf"]}"#,
        true,
        &["files"],
    ),
    (
        "checkbox",
        r#"{"id":"d","checked":true}"#,
        true,
        &["checked"],
    ),
    ("radio", r#"{"id":"d"}"#, true, &["checked"]),
    ("extract_forms", "{}", true, &["forms", "count"]),
    // ── 页面分析 ──
    ("screenshot", "{}", true, &["width", "height", "base64"]),
    (
        "screenshot_element",
        r#"{"id":"g"}"#,
        true,
        &["width", "height"],
    ),
    ("get_page_title", "{}", true, &["title"]),
    ("get_current_url", "{}", true, &["url"]),
    ("get_page_text", "{}", true, &["text"]),
    ("get_element_info", r#"{"id":"g"}"#, true, &["tag", "refs"]),
    ("get_element_text", r#"{"id":"g"}"#, true, &["text"]),
    ("get_attributes", r#"{"id":"b"}"#, true, &["attrs"]),
    ("is_visible", r#"{"id":"g"}"#, true, &["visible"]),
    ("is_enabled", r#"{"id":"g"}"#, true, &["enabled"]),
    ("get_focused_element", "{}", true, &["focused"]),
    ("get_selected_text", "{}", true, &["text"]),
    ("get_page_meta", "{}", true, &["meta"]),
    ("get_scroll_position", "{}", true, &["scroll"]),
    ("set_scroll_position", r#"{"y":100}"#, true, &["ok"]),
    ("get_performance_metrics", "{}", true, &["metrics"]),
    ("get_accessibility_tree", "{}", true, &["tree", "count"]),
    // ── 会话 ──
    (
        "cookie_get",
        r#"{"domain":"example.com"}"#,
        true,
        &["cookies"],
    ),
    (
        "cookie_set",
        r#"{"name":"k","value":"v","domain":"example.com"}"#,
        true,
        &["set", "ok"],
    ),
    (
        "cookie_clear",
        r#"{"domain":"example.com"}"#,
        true,
        &["cleared"],
    ),
    ("clear_cookies", "{}", true, &["cleared"]),
    ("storage_get", r#"{"key":"k"}"#, true, &["value"]),
    ("storage_set", r#"{"key":"k","value":"v"}"#, true, &["ok"]),
    ("storage_get_all", "{}", true, &["storage"]),
    ("clear_storage", "{}", true, &["cleared"]),
    // ── 高级 ──
    (
        "execute_js",
        r#"{"script":"document.title"}"#,
        true,
        &["result"],
    ),
    ("evaluate_xpath", r#"{"expr":"//a"}"#, true, &["result"]),
    ("inject_css", r#"{"css":"body{}"}"#, true, &["injected"]),
    (
        "block_request",
        r#"{"patterns":["*ads*"]}"#,
        true,
        &["patterns", "enabled"],
    ),
    (
        "intercept_request",
        r#"{"patterns":["*.png"]}"#,
        true,
        &["patterns", "enabled"],
    ),
    // ── 网络拦截（mock 无 Fetch → 预期优雅失败）──
    ("list_pending_requests", "{}", true, &["requests", "count"]),
    (
        "fulfill_request",
        r#"{"request_id":"x","status":200}"#,
        false,
        &[],
    ),
    ("continue_request", r#"{"request_id":"x"}"#, false, &[]),
    ("abort_request", r#"{"request_id":"x"}"#, false, &[]),
    (
        "modify_response",
        r#"{"request_id":"x","status":200}"#,
        false,
        &[],
    ),
    // ── 对话框 ──
    ("pending_dialog", "{}", true, &["dialog"]),
    (
        "dialog_accept",
        r#"{"prompt_text":"yes"}"#,
        true,
        &["accepted", "ok"],
    ),
    ("dialog_dismiss", "{}", true, &["dismissed", "ok"]),
    // ── 多标签页 ──
    (
        "new_tab",
        r#"{"url":"https://example.com/search"}"#,
        true,
        &["tab", "url"],
    ),
    (
        "new_window",
        r#"{"url":"https://example.com/search"}"#,
        true,
        &["tab", "url"],
    ),
    ("list_tabs", "{}", true, &["tabs"]),
    ("get_active_tab", "{}", true, &["tab", "url", "title"]),
    ("get_tab", r#"{"title_contains":"Login"}"#, true, &["tab"]),
    ("close_tab", r#"{"tab":1}"#, true, &["closed"]),
    ("switch_tab", r#"{"tab":1}"#, true, &["active"]),
    ("duplicate_tab", "{}", true, &["tab", "url"]),
    ("close_other_tabs", "{}", true, &["closed"]),
    // ── Agent 辅助 / PDF ──
    ("done", r#"{"answer":"ok"}"#, true, &["done", "answer"]),
    ("export_replay", "{}", true, &["script"]),
    (
        "save_as_pdf",
        r#"{"path":"/tmp/fb_contract.pdf"}"#,
        true,
        &["path", "bytes", "ok"],
    ),
    // ── 设备模拟 / 认证（mock 无 CDP/Emulation → 预期优雅失败）──
    ("set_touch_emulation", r#"{"enabled":true}"#, false, &[]),
    (
        "set_geolocation",
        r#"{"latitude":1.0,"longitude":2.0}"#,
        false,
        &[],
    ),
    (
        "set_timezone",
        r#"{"timezone_id":"Asia/Shanghai"}"#,
        false,
        &[],
    ),
    (
        "set_basic_auth",
        r#"{"username":"a","password":"b"}"#,
        false,
        &[],
    ),
];

#[test]
fn every_tool_obeys_its_contract() {
    let mut failures = Vec::new();
    for (name, params, expect_ok, keys) in CONTRACT {
        let s = sdk(); // 每个工具用全新 sdk，避免状态污染
        let params: Value = serde_json::from_str(params).unwrap();
        let result = s.tool_call(name, params);
        match (expect_ok, &result) {
            (true, Ok(v)) => {
                for k in *keys {
                    if v.get(*k).is_none() {
                        failures.push(format!("{name}: missing key '{k}' in {v}"));
                    }
                }
            }
            (false, Err(_)) => {} // 预期优雅失败
            (true, Err(e)) => failures.push(format!("{name}: unexpected error {e}")),
            (false, Ok(v)) => {
                failures.push(format!("{name}: expected graceful failure but got {v}"))
            }
        }
    }
    assert!(
        failures.is_empty(),
        "tool contract violations:\n{}",
        failures.join("\n")
    );
}

#[test]
fn tool_count_matches_manifest() {
    let s = sdk();
    assert_eq!(
        s.tool_count(),
        CONTRACT.len(),
        "manifest tools must all be covered by the contract table"
    );
}

#[test]
fn every_contract_tool_name_is_registered() {
    // 契约表里的名字都能在注册表找到（防止拼写漂移）
    let s = sdk();
    let list = s.tool_list();
    let names: Vec<&str> = list
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect();
    for (name, _, _, _) in CONTRACT {
        assert!(
            names.contains(name),
            "contract references unregistered tool '{name}'"
        );
    }
}

/// 输出形态稳定性：同一工具对同一输入在 mock 上应确定性返回（幂等重放）。
#[test]
fn tool_outputs_are_deterministic() {
    let a = sdk();
    let b = sdk();
    let probe = ["get_page_text", "get_accessibility_tree", "extract_links"];
    for p in probe {
        let va = a.tool_call(p, json!({})).unwrap();
        let vb = b.tool_call(p, json!({})).unwrap();
        assert_eq!(va, vb, "tool '{p}' not deterministic");
    }
}

/// 全部工具输出可被 JSON 序列化（serde_json 拒绝 NaN/Infinity，序列化必须成功）。
#[test]
fn tool_outputs_are_json_safe() {
    for (name, params, _, _) in CONTRACT {
        let s = sdk();
        let params: Value = serde_json::from_str(params).unwrap();
        if let Ok(v) = s.tool_call(name, params) {
            assert!(
                serde_json::to_string(&v).is_ok(),
                "{name} produced a value that cannot be serialized to JSON"
            );
        }
    }
}
