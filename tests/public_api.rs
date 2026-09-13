// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Public-API regression tests (mock engine, no browser required).
//!
//! These pin the documented SDK surface (README / docs/api-reference.md) so the
//! open-source API stays stable: config defaults, the tool registry shape, the
//! structured error JSON, snapshot element lookup, and the audit trail.

use std::collections::HashSet;

use serde_json::json;

use fastbrowser::sdk::Fastbrowser;
use fastbrowser::Config;

fn sdk() -> Fastbrowser {
    let s = Fastbrowser::new();
    s.init(Config::for_engine("mock")).unwrap();
    s
}

#[test]
fn config_for_engine_and_default() {
    let c = Config::for_engine("mock");
    assert_eq!(c.engine, "mock");
    // The mock engine is the out-of-the-box default (no browser needed).
    assert_eq!(Config::default().engine, "mock");
}

#[test]
fn not_initialized_errors_are_structured() {
    let s = Fastbrowser::new();
    let err = s.open("https://example.com").unwrap_err();
    let j = err.to_json();
    // Documented shape: {"error": {"kind": "...", "message": "..."}}
    assert!(
        j.get("error").is_some(),
        "expected nested error object: {j}"
    );
    assert!(
        j["error"].get("kind").and_then(|k| k.as_str()).is_some(),
        "error.kind missing: {j}"
    );
    assert!(
        j["error"].get("message").and_then(|m| m.as_str()).is_some(),
        "error.message missing: {j}"
    );
}

#[test]
fn tool_registry_is_well_formed() {
    let s = sdk();
    let list = s.tool_list();
    let arr = list.as_array().expect("tool_list must be an array");
    assert!(
        arr.len() >= 80,
        "expected the full toolset, got {}",
        arr.len()
    );

    let mut names: HashSet<String> = HashSet::new();
    for t in arr {
        let name = t["name"].as_str().expect("tool must have a name");
        assert!(!name.is_empty(), "empty tool name");
        assert!(
            names.insert(name.to_string()),
            "duplicate tool name: {name}"
        );
        assert!(
            !t["description"].as_str().unwrap_or("").trim().is_empty(),
            "tool '{name}' missing description"
        );
        assert_eq!(
            t["schema"]["type"], "object",
            "tool '{name}' schema must be an object"
        );
    }
    assert_eq!(
        s.tool_count(),
        names.len(),
        "tool_count must match tool_list"
    );
}

#[test]
fn documented_tools_are_present() {
    let s = sdk();
    let names: HashSet<String> = s
        .tool_list()
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t["name"].as_str().map(str::to_string))
        .collect();
    // NOTE: `open`/`snapshot` are SDK methods (`sdk.open` / `sdk.snapshot`),
    // not registered tools.
    for n in [
        "navigate",
        "click",
        "type",
        "press",
        "get_current_url",
        "get_page_title",
        "get_page_text",
        "extract_links",
        "list_tabs",
        "switch_tab",
        "get_active_tab",
        "wait_for_element",
        "execute_js",
    ] {
        assert!(names.contains(n), "documented tool '{n}' is missing");
    }
}

#[test]
fn unknown_tool_fails_gracefully() {
    let s = sdk();
    assert!(s.tool_call("definitely_not_a_tool", json!({})).is_err());
}

#[test]
fn snapshot_element_lookup_roundtrip() {
    let s = sdk();
    s.open("https://example.com").unwrap();
    let snap = s.snapshot().unwrap();
    assert!(
        !snap.interactive.is_empty(),
        "mock snapshot must have elements"
    );
    for el in &snap.interactive {
        assert_eq!(
            el.id,
            snap.element_by_id(el.id).unwrap().id,
            "element_by_id must find every interactive element"
        );
    }
}

#[test]
fn audit_records_and_clears() {
    let s = sdk();
    s.open("https://example.com").unwrap();
    s.clear_audit();
    assert_eq!(s.audit().as_array().map(Vec::len), Some(0));
    s.tool_call("get_page_title", json!({})).unwrap();
    assert!(
        s.audit().as_array().map(|a| !a.is_empty()).unwrap_or(false),
        "audit should record the tool call"
    );
}

#[test]
fn active_tab_reports_bound_page() {
    let s = sdk();
    s.open("https://example.com/active").unwrap();
    let v = s.tool_call("get_active_tab", json!({})).unwrap();
    assert!(v.get("tab").is_some());
    assert!(v["url"]
        .as_str()
        .unwrap_or("")
        .contains("example.com/active"));
}
