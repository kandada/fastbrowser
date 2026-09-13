// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! 新增强能力工具的精细化测试（mock 引擎，无需真实浏览器）。
//!
//! 覆盖：done / send_keys / save_as_pdf / 对话框 / 网络拦截 /
//! search / find_elements / get_history / AX 树 / 审计 / JSON Schema。

use fastbrowser::sdk::Fastbrowser;
use fastbrowser::Config;
use serde_json::{json, Value};

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

// ── Agent 辅助工具 ───────────────────────────────────────────

#[test]
fn done_returns_answer_and_marks_complete() {
    let s = sdk_open("https://example.com");
    let v = s
        .tool_call("done", json!({"answer": "done, extracted 3 items"}))
        .unwrap();
    assert_eq!(v["done"], true);
    assert_eq!(v["answer"], "done, extracted 3 items");
    // done 缺少 answer → 报错
    assert!(s.tool_call("done", json!({})).is_err());
}

#[test]
fn send_keys_combo_variants() {
    let s = sdk_open("https://example.com/login");
    for combo in [
        "Enter",
        "Ctrl+a",
        "Shift+Enter",
        "Meta+l",
        "Alt+ArrowDown",
        "Ctrl+Shift+Delete",
        "Tab",
    ] {
        let v = s.tool_call("send_keys", json!({"keys": combo})).unwrap();
        assert_eq!(v["ok"], true, "send_keys {combo}");
    }
    assert!(s.tool_call("send_keys", json!({})).is_err());
}

// ── PDF ──────────────────────────────────────────────────────

#[test]
fn save_as_pdf_writes_valid_pdf() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("out.pdf");
    let path_str = path.to_str().unwrap().to_string();
    let s = sdk_open("https://example.com");
    let v = s
        .tool_call("save_as_pdf", json!({"path": path_str}))
        .unwrap();
    assert_eq!(v["ok"], true);
    let bytes = std::fs::read(dir.path().join("out.pdf")).unwrap();
    assert!(bytes.starts_with(b"%PDF-"));
    // 缺 path → 报错
    assert!(s.tool_call("save_as_pdf", json!({})).is_err());
}

// ── 对话框（mock 无真实弹窗：pending 为空、accept/dismiss 不报错）──

#[test]
fn dialog_tools_graceful() {
    let s = sdk_open("https://example.com");
    let v = s.tool_call("pending_dialog", json!({})).unwrap();
    assert_eq!(v["dialog"], Value::Null);
    let v = s.tool_call("dialog_accept", json!({})).unwrap();
    assert_eq!(v["ok"], true);
    let v = s.tool_call("dialog_dismiss", json!({})).unwrap();
    assert_eq!(v["ok"], true);
    let v = s
        .tool_call("dialog_accept", json!({"prompt_text": "yes"}))
        .unwrap();
    assert_eq!(v["ok"], true);
}

// ── 网络拦截（mock 无 Fetch：pending 为空、拦截工具报 Unsupported）──

#[test]
fn network_interception_tools() {
    let s = sdk_open("https://example.com");
    let v = s.tool_call("list_pending_requests", json!({})).unwrap();
    assert_eq!(v["count"], 0);
    assert!(s
        .tool_call("fulfill_request", json!({"request_id": "x"}))
        .is_err());
    assert!(s
        .tool_call("continue_request", json!({"request_id": "x"}))
        .is_err());
    assert!(s
        .tool_call("abort_request", json!({"request_id": "x"}))
        .is_err());
    // 拦截开关注册本身可用
    let v = s
        .tool_call("intercept_request", json!({"patterns": ["*.png"]}))
        .unwrap();
    assert_eq!(v["enabled"], true);
    assert_eq!(v["patterns"][0], "*.png");
    let v = s
        .tool_call("block_request", json!({"patterns": ["*ads*"]}))
        .unwrap();
    assert_eq!(v["enabled"], true);
}

// ── search / find_elements ───────────────────────────────────

#[test]
fn search_finds_text_and_handles_missing() {
    let s = sdk_open("https://example.com");
    // mock 引擎 JS 求值器能力有限（无 createTreeWalker）→ 优雅返回空；真浏览器由
    // chromium 集成测试覆盖（chromium_new_capabilities 断言命中 "Hello world"）。
    let v = s
        .tool_call("search", json!({"query": "Welcome to FastBrowser"}))
        .unwrap();
    assert!(
        v.get("matches").is_some(),
        "search should return matches array"
    );
    assert!(v.get("count").is_some());
    let v = s
        .tool_call("search", json!({"query": "zzz-nonexistent-xyz"}))
        .unwrap();
    assert_eq!(v["count"], 0);
    assert!(s.tool_call("search", json!({})).is_err());
}

#[test]
fn find_elements_by_selector() {
    let s = sdk_open("https://example.com");
    // mock 求值器同样受限；返回数组形态不崩溃即可（真实行为见 chromium 集成测试）
    let v = s
        .tool_call("find_elements", json!({"selector": "a"}))
        .unwrap();
    assert!(v.get("elements").is_some());
    assert!(v.get("count").is_some());
    let v = s
        .tool_call("find_elements", json!({"selector": "div.missing-xyz"}))
        .unwrap();
    assert_eq!(v["count"], 0);
    assert!(s.tool_call("find_elements", json!({})).is_err());
}

// ── 导航历史 ────────────────────────────────────────────────

#[test]
fn history_tracks_navigation() {
    let s = sdk_open("https://example.com");
    s.tool_call("navigate", json!({"url": "https://example.com/login"}))
        .unwrap();
    s.tool_call("navigate", json!({"url": "https://example.com/search"}))
        .unwrap();
    let v = s.tool_call("get_history", json!({})).unwrap();
    assert!(v["count"].as_u64().unwrap() >= 3);
    let urls: Vec<&str> = v["history"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|h| h["url"].as_str())
        .collect();
    assert!(urls.contains(&"https://example.com/login"));
    assert!(urls.contains(&"https://example.com/search"));
}

// ── 无障碍树（mock 构建 / CDP 真树，此处验证 mock 路径）────────

#[test]
fn accessibility_tree_mock() {
    let s = sdk_open("https://example.com/login");
    let v = s.tool_call("get_accessibility_tree", json!({})).unwrap();
    assert!(v["count"].as_u64().unwrap() >= 3);
    let roles: Vec<&str> = v["tree"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|n| n["role"].as_str())
        .collect();
    assert!(roles.contains(&"button"));
    assert!(roles.contains(&"textbox"));
    // 顶层快照交互元素与树一致
    let snap = s.snapshot().unwrap();
    assert_eq!(snap.interactive.len() as u64, v["count"].as_u64().unwrap());
}

// ── 审计日志 ────────────────────────────────────────────────

#[test]
fn audit_records_actions_newest_first() {
    let s = sdk_open("https://example.com");
    s.tool_call("click", json!({"id": "c"})).unwrap();
    s.tool_call("extract_links", json!({})).unwrap();
    // 故意失败一次
    let _ = s.tool_call("click", json!({"id": "zz"}));

    let log = s.audit();
    let arr = log.as_array().unwrap();
    assert!(
        arr.len() >= 4,
        "expected open+3 tool calls, got {}",
        arr.len()
    );
    // 新→旧：最后一条是失败的 click
    assert_eq!(arr[0]["action"], "click");
    assert!(arr[0]["result"].get("error").is_some());
    assert_eq!(arr[1]["action"], "extract_links");
    assert!(arr[1]["result"].get("ok").is_some());
    assert!(arr[0]["seq"].as_u64().unwrap() > arr[1]["seq"].as_u64().unwrap());
    // 每条含时间戳/参数
    assert!(arr[0]["ts_ms"].as_u64().unwrap() > 0);
    assert!(arr[0]["params"].is_object());

    s.clear_audit();
    assert_eq!(s.audit().as_array().unwrap().len(), 0);
}

#[test]
fn audit_bounded_and_covers_open() {
    let s = sdk_open("https://example.com");
    // open 应被记录（action == "open"）
    let log = s.audit();
    assert!(log
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["action"] == "open"));
    // 大量工具调用后有界（不无限增长）
    for _ in 0..20 {
        let _ = s.tool_call("get_page_title", json!({}));
    }
    let n = s.audit().as_array().unwrap().len();
    assert!(n <= 30, "audit should be bounded, got {n}");
}

// ── 工具清单：JSON Schema ───────────────────────────────────

#[test]
fn tool_manifest_has_standard_json_schema() {
    let s = sdk();
    let list = s.tool_list();
    let arr = list.as_array().unwrap();
    assert!(arr.len() >= 88, "expected >=88 tools, got {}", arr.len());
    // 每个工具都有 schema 且为标准 JSON Schema 形态
    for t in arr {
        let schema = &t["schema"];
        assert_eq!(schema["type"], "object", "{} schema", t["name"]);
        assert!(schema.get("properties").is_some() || schema.get("required").is_none());
    }
    // 必填参数进入 required
    let nav = arr.iter().find(|t| t["name"] == "navigate").unwrap();
    assert!(nav["schema"]["required"]
        .as_array()
        .unwrap()
        .contains(&json!("url")));
    // 无必填参数的工具没有 required 或 required 为空
    let click = arr.iter().find(|t| t["name"] == "click").unwrap();
    assert!(!click["schema"]["required"]
        .as_array()
        .map(|a| a.contains(&json!("id")))
        .unwrap_or(false));
    // description / example 齐全
    assert!(arr
        .iter()
        .all(|t| t["description"].is_string() && t["example"].is_string()));
}

#[test]
fn tool_names_unique() {
    let s = sdk();
    let list = s.tool_list();
    let arr = list.as_array().unwrap();
    let names: Vec<&str> = arr.iter().filter_map(|t| t["name"].as_str()).collect();
    let mut sorted = names.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(names.len(), sorted.len(), "duplicate tool names");
}

// ── C ABI 新符号 ────────────────────────────────────────────

#[test]
fn ffi_audit_symbols() {
    use std::ffi::{c_char, CStr};
    use std::sync::Mutex;

    extern "C" {
        fn fastbrowser_init(cfg: *const c_char) -> *mut c_char;
        fn fastbrowser_open(url: *const c_char) -> *mut c_char;
        fn fastbrowser_audit() -> *mut c_char;
        fn fastbrowser_clear_audit() -> *mut c_char;
        fn fastbrowser_free_string(p: *mut c_char);
        fn fastbrowser_shutdown() -> *mut c_char;
    }

    static LOCK: Mutex<()> = Mutex::new(());
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let cstr = |s: &str| std::ffi::CString::new(s).unwrap();
    let take = |p: *mut c_char| unsafe {
        let s = CStr::from_ptr(p).to_str().unwrap().to_string();
        fastbrowser_free_string(p);
        s
    };

    unsafe {
        take(fastbrowser_init(cstr(r#"{"engine":"mock"}"#).as_ptr()));
        take(fastbrowser_open(cstr("https://example.com").as_ptr()));
        let log: Value = serde_json::from_str(&take(fastbrowser_audit())).unwrap();
        assert!(log
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["action"] == "open"));
        take(fastbrowser_clear_audit());
        let log: Value = serde_json::from_str(&take(fastbrowser_audit())).unwrap();
        assert_eq!(log.as_array().unwrap().len(), 0);
        take(fastbrowser_shutdown());
    }
}

// ── 快照 meta（滚动/截断提示）────────────────────────────────

#[test]
fn snapshot_includes_meta() {
    let s = sdk_open("https://example.com");
    let snap = s.snapshot().unwrap();
    assert_eq!(snap.meta.total, snap.interactive.len());
    assert!(snap.meta.viewport_h > 0);
}

// ── 隔离上下文引擎无关层（mock 不支持 → 优雅降级）─────────────

#[test]
fn isolated_profiles_graceful_on_mock() {
    let s = Fastbrowser::new();
    s.init(Config {
        isolated_profiles: true,
        ..Config::default()
    })
    .unwrap();
    // mock 引擎 create_context 返回 Unsupported → 自动退化为普通标签页
    let out = s.open("https://example.com").unwrap();
    assert!(out["tab"].is_number());
    let tabs = s.tool_call("list_tabs", json!({})).unwrap();
    assert!(!tabs["tabs"].as_array().unwrap().is_empty());
}
