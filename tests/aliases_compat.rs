// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Cross-ecosystem compatibility regression tests (mock engine, no browser).
//!
//! Models trained on Playwright MCP or browser-use emit their own tool/param
//! names. `tools::aliases` resolves those in the kernel so every consumer gets
//! the compatibility for free. These tests pin that behaviour end-to-end through
//! the SDK and the registry dispatch path.

use serde_json::json;

use fastbrowser::bridge::Runtime;
use fastbrowser::config::Config;
use fastbrowser::engines::mock::MockEngine;
use fastbrowser::sdk::Fastbrowser;
use fastbrowser::tools::registry::ToolRegistry;

fn sdk() -> Fastbrowser {
    let s = Fastbrowser::new();
    s.init(Config::for_engine("mock")).unwrap();
    s
}

fn open_sdk() -> Fastbrowser {
    let s = sdk();
    s.open("https://example.com/alias").unwrap();
    s
}

fn runtime() -> Runtime {
    Runtime::new(Box::new(MockEngine::new()), Config::for_engine("mock"))
}

// ── Playwright MCP tool names ───────────────────────────────────────────────

#[test]
fn playwright_tool_names_route_to_canonical() {
    let s = open_sdk();

    let v = s
        .tool_call("browser_navigate", json!({"url": "https://example.com/pw"}))
        .unwrap();
    assert!(v["url"].as_str().unwrap_or("").contains("/pw"), "{v}");

    // browser_snapshot → get_accessibility_tree
    let v = s.tool_call("browser_snapshot", json!({})).unwrap();
    assert!(v["count"].as_u64().unwrap_or(0) >= 1, "{v}");

    let v = s.tool_call("browser_click", json!({"id": "a"})).unwrap();
    assert!(v.get("clicked").is_some(), "{v}");

    let v = s
        .tool_call("browser_type", json!({"id": "b", "text": "hi"}))
        .unwrap();
    assert_eq!(v["typed"], "hi", "{v}");

    let v = s
        .tool_call("browser_press_key", json!({"key": "Enter"}))
        .unwrap();
    assert_eq!(v["key"], "Enter", "{v}");

    let v = s.tool_call("browser_tabs", json!({})).unwrap();
    assert!(v["tabs"].is_array(), "{v}");
}

// ── browser-use tool names ──────────────────────────────────────────────────

#[test]
fn browser_use_tool_names_route_to_canonical() {
    let s = open_sdk();

    let v = s
        .tool_call("go_to_url", json!({"url": "https://example.com/bu"}))
        .unwrap();
    assert!(v["url"].as_str().unwrap_or("").contains("/bu"), "{v}");

    let v = s
        .tool_call("click_element_by_index", json!({"id": "a"}))
        .unwrap();
    assert!(v.get("clicked").is_some(), "{v}");

    let v = s
        .tool_call("input_text", json!({"id": "b", "text": "world"}))
        .unwrap();
    assert_eq!(v["typed"], "world", "{v}");

    // extract_content → extract_text
    let v = s.tool_call("extract_content", json!({})).unwrap();
    assert!(v.get("text").is_some(), "{v}");

    // refresh → reload
    let v = s.tool_call("refresh", json!({})).unwrap();
    assert_eq!(v["ok"], true, "{v}");

    // go_back → back
    let v = s.tool_call("go_back", json!({})).unwrap();
    assert!(v.get("url").is_some(), "{v}");

    // open_tab → new_tab
    let v = s
        .tool_call("open_tab", json!({"url": "https://example.com/two"}))
        .unwrap();
    assert!(v.get("tab").is_some(), "{v}");

    // select_dropdown_option → select_option (login page has a <select id=e>)
    s.tool_call("navigate", json!({"url": "https://example.com/login"}))
        .unwrap();
    let v = s
        .tool_call("select_dropdown_option", json!({"id": "e", "value": "pro"}))
        .unwrap();
    assert!(v.get("selected").is_some(), "{v}");
}

// ── parameter aliases ───────────────────────────────────────────────────────

#[test]
fn param_aliases_are_normalized() {
    let s = open_sdk();

    // execute_js: expression / code / js → script
    for alias in ["expression", "code", "js"] {
        let v = s
            .tool_call("execute_js", json!({alias: "document.title"}))
            .unwrap();
        assert!(
            v["result"].as_str().unwrap_or("").contains("Example"),
            "{alias}: {v}"
        );
    }

    // navigate: uri / link / address → url
    let v = s
        .tool_call("navigate", json!({"uri": "https://example.com/uri"}))
        .unwrap();
    assert!(v["url"].as_str().unwrap_or("").contains("/uri"), "{v}");

    // wait_for_element: css → selector
    let v = s
        .tool_call(
            "wait_for_element",
            json!({"css": "button", "timeout_ms": 100}),
        )
        .unwrap();
    assert_eq!(v["found"], true, "{v}");

    // search: q → query
    let v = s.tool_call("search", json!({"q": "Welcome"})).unwrap();
    assert!(
        v.get("matches").is_some() && v.get("count").is_some(),
        "{v}"
    );

    // type: content → text
    let v = s
        .tool_call("type", json!({"id": "b", "content": "alias-text"}))
        .unwrap();
    assert_eq!(v["typed"], "alias-text", "{v}");

    // find_elements: css → selector
    let v = s.tool_call("find_elements", json!({"css": "a"})).unwrap();
    assert!(
        v.get("elements").is_some() && v.get("count").is_some(),
        "{v}"
    );
}

#[test]
fn declared_param_is_not_rewritten_by_alias() {
    let s = open_sdk();

    // evaluate_xpath declares `expr`; the alias table also maps `expr`→`script`
    // for other tools. A declared key must never be rewritten.
    let v = s
        .tool_call("evaluate_xpath", json!({"expr": "//a"}))
        .unwrap();
    assert!(v.get("result").is_some(), "{v}");

    // When both the declared key and an alias are present, the declared key wins.
    let v = s
        .tool_call(
            "execute_js",
            json!({"script": "document.title", "code": "SHOULD_NOT_WIN"}),
        )
        .unwrap();
    assert!(
        v["result"].as_str().unwrap_or("").contains("Example"),
        "{v}"
    );
}

// ── audit + registry path ───────────────────────────────────────────────────

#[test]
fn aliased_calls_are_audited_under_canonical_name() {
    let s = open_sdk();
    s.clear_audit();
    s.tool_call("browser_navigate", json!({"url": "https://example.com/x"}))
        .unwrap();

    let audit = s.audit();
    let actions: Vec<&str> = audit
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["action"].as_str())
        .collect();
    assert!(actions.contains(&"navigate"), "audit: {audit}");
    assert!(
        !actions.contains(&"browser_navigate"),
        "alias leaked into audit: {audit}"
    );
}

#[test]
fn unknown_alias_passes_through_and_errors() {
    let s = sdk();
    assert!(s.tool_call("totally_unknown_tool", json!({})).is_err());
}

#[test]
fn registry_call_path_also_resolves_aliases() {
    let r = runtime();
    let reg = ToolRegistry::new();

    let out = reg
        .call(
            &r,
            "browser_navigate",
            json!({"url": "https://example.com/reg"}),
        )
        .unwrap();
    assert!(out["url"].as_str().unwrap_or("").contains("/reg"), "{out}");

    // param alias through the registry path too
    let out = reg
        .call(&r, "execute_js", json!({"expression": "document.title"}))
        .unwrap();
    assert!(out.get("result").is_some(), "{out}");
}

#[test]
fn alias_targets_are_registered_tools() {
    // Every alias must point at a real canonical tool, so the compat table can
    // never drift into referencing a name that no longer exists.
    let reg = ToolRegistry::new();
    let names: Vec<String> = reg.names();
    for (alias, target) in fastbrowser::tools::aliases::TOOL_ALIASES {
        assert!(
            names.iter().any(|n| n == target),
            "alias '{alias}' points at unregistered tool '{target}'"
        );
    }
}

#[test]
fn alias_resolution_is_stable() {
    use fastbrowser::tools::aliases::resolve_tool_alias;
    assert_eq!(resolve_tool_alias("browser_click"), "click");
    assert_eq!(resolve_tool_alias("click"), "click");
}

/// browser-use 的 `index` 参数经别名映射到 `id`，并按快照下标解析。
#[test]
fn browser_use_index_click_works() {
    let s = open_sdk();
    let v = s
        .tool_call("click_element_by_index", json!({"index": 0}))
        .unwrap();
    assert!(v.get("clicked").is_some(), "{v}");
    // 1-based 的 eN ref 也可直接用于浏览器 click
    let v = s.tool_call("click", json!({"ref": "e1"})).unwrap();
    assert!(v.get("clicked").is_some(), "{v}");
}
