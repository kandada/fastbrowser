// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! 方言兼容契约测试（契约规范 §5 / §9.2）。
//!
//! 断言 Playwright MCP / Playwright 库 / browser-use 风格的调用能被正确映射，
//! 且参数归一化/类型容错/未知参数告警按契约工作。

use fastbrowser::engine::{ElementRef, RefKind};
use fastbrowser::sdk::Fastbrowser;
use fastbrowser::tools::aliases::{
    coerce_params, prepare_params, resolve_tool_alias, unknown_params,
};
use fastbrowser::tools::tool::resolve_target_from;
use fastbrowser::Config;
use serde_json::{json, Value};

fn sdk() -> Fastbrowser {
    let s = Fastbrowser::new();
    s.init(Config::default()).unwrap();
    s.open("https://example.com/login").unwrap();
    s
}

#[test]
fn tool_name_aliases_resolve() {
    let cases = [
        ("browser_navigate", "navigate"),
        ("browser_navigate_forward", "forward"),
        ("browser_click", "click"),
        ("browser_evaluate", "execute_js"),
        ("browser_run_code", "execute_js"),
        ("browser_resize", "set_viewport"),
        ("browser_wait_for", "wait_for_text"),
        ("browser_tab_new", "new_tab"),
        ("browser_tab_select", "switch_tab"),
        ("browser_tab_close", "close_tab"),
        ("browser_handle_dialog", "dialog_accept"),
        ("go_to_url", "navigate"),
        ("click_element", "click"),
    ];
    for (alias, canonical) in cases {
        assert_eq!(resolve_tool_alias(alias), canonical, "alias {alias}");
    }
    // 规范名不变
    assert_eq!(resolve_tool_alias("navigate"), "navigate");
}

#[test]
fn element_target_dialects() {
    let cases: &[(&str, RefKind, &str)] = &[
        // snapshot letter (string ref)
        (r#"{"ref":"a"}"#, RefKind::Snapshot, "a"),
        // CSS via selector
        (r#"{"selector":"a.x"}"#, RefKind::Css, "a.x"),
        // Playwright text= / role= / xpath=
        (r#"{"selector":"text=submit"}"#, RefKind::Text, "submit"),
        (r#"{"selector":"role=button"}"#, RefKind::Role, "button"),
        (r#"{"selector":"xpath=//a"}"#, RefKind::Xpath, "//a"),
        (r#"{"selector":"//div[@id]"}"#, RefKind::Xpath, "//div[@id]"),
        // :has-text(...) → Playwright 选择器引擎
        (
            r#"{"selector":"button:has-text(\"Submit\")"}"#,
            RefKind::Selector,
            r#"button:has-text("Submit")"#,
        ),
        // id variants
        (r##"{"id":"#list_zwz"}"##, RefKind::Css, "#list_zwz"),
        (r#"{"id":"a"}"#, RefKind::Snapshot, "a"),
        (r#"{"id":"submit-btn"}"#, RefKind::Css, "#submit-btn"),
        // MCP human-readable element → text
        (
            r#"{"element":"Submit button"}"#,
            RefKind::Text,
            "Submit button",
        ),
        // ref object (legacy)
        (
            r##"{"ref":{"kind":"css","value":"#x"}}"##,
            RefKind::Css,
            "#x",
        ),
        (
            r#"{"ref":{"kind":"text","value":"Login"}}"#,
            RefKind::Text,
            "Login",
        ),
    ];
    for (params, kind, value) in cases {
        let p: Value = serde_json::from_str(params).unwrap();
        let got = resolve_target_from(&p).unwrap_or_else(|e| panic!("{params}: {e}"));
        assert_eq!(
            got,
            ElementRef {
                kind: *kind,
                value: value.to_string()
            },
            "params {params}"
        );
    }
    // 无目标 → 明确报错
    assert!(resolve_target_from(&json!({})).is_err());
    assert!(resolve_target_from(&json!({"ref": ""})).is_err());
}

/// Dialect fuzz: a broad matrix of element-target shapes → resolved ref.
#[test]
fn element_target_dialect_fuzz() {
    let cases: &[(&str, RefKind, &str)] = &[
        (r#"{"selector":"text=Login"}"#, RefKind::Text, "Login"),
        (r#"{"selector":"role=textbox"}"#, RefKind::Role, "textbox"),
        (
            r#"{"selector":"xpath=//button"}"#,
            RefKind::Xpath,
            "//button",
        ),
        (r#"{"selector":"//a[1]"}"#, RefKind::Xpath, "//a[1]"),
        (r#"{"selector":"(/html)"}"#, RefKind::Xpath, "(/html)"),
        (r#"{"selector":"css=#a"}"#, RefKind::Css, "#a"),
        (
            r#"{"selector":"div.card > span"}"#,
            RefKind::Css,
            "div.card > span",
        ),
        (
            r#"{"selector":"button:has-text('Submit')"}"#,
            RefKind::Selector,
            "button:has-text('Submit')",
        ),
        (
            r#"{"selector":"a:has-text(\"Home\")"}"#,
            RefKind::Selector,
            r#"a:has-text("Home")"#,
        ),
        (r#"{"ref":"text=x"}"#, RefKind::Text, "x"),
        (r##"{"ref":"#a"}"##, RefKind::Css, "#a"),
        (r#"{"ref":"a"}"#, RefKind::Snapshot, "a"),
        (r#"{"ref":"z9"}"#, RefKind::Css, "z9"),
        (r#"{"id":"btn"}"#, RefKind::Css, "#btn"),
        (r#"{"id":".cls"}"#, RefKind::Css, ".cls"),
        (r#"{"id":"[data-x]"}"#, RefKind::Css, "[data-x]"),
        (r#"{"id":"a b"}"#, RefKind::Css, "a b"),
        (r#"{"element":"Sign in"}"#, RefKind::Text, "Sign in"),
        (
            r#"{"ref":{"kind":"role","value":"button"}}"#,
            RefKind::Role,
            "button",
        ),
        (
            r#"{"ref":{"kind":"snapshot","value":"b"}}"#,
            RefKind::Snapshot,
            "b",
        ),
        (
            r#"{"ref":{"kind":"xpath","value":"//x"}}"#,
            RefKind::Xpath,
            "//x",
        ),
    ];
    for (params, kind, value) in cases {
        let p: Value = serde_json::from_str(params).unwrap();
        let got = resolve_target_from(&p).unwrap_or_else(|e| panic!("{params}: {e}"));
        assert_eq!(
            got,
            ElementRef {
                kind: *kind,
                value: value.to_string()
            },
            "params {params}"
        );
    }
    // No usable target → a clear error (never a panic).
    for bad in [
        json!({}),
        json!({"ref": 123}),
        json!({"ref": null}),
        json!({"selector": ""}),
        json!({"id": ""}),
        json!({"element": ""}),
        json!({"ref": ""}),
    ] {
        assert!(
            resolve_target_from(&bad).is_err(),
            "expected error for {bad}"
        );
    }
}

#[test]
fn param_type_coercion() {
    let declared = json!({
        "timeout_ms": {"type": "integer"},
        "checked": {"type": "boolean"},
        "ratio": {"type": "number"}
    });
    let out = coerce_params(
        json!({"timeout_ms": "5000", "checked": "true", "ratio": "1.5"}),
        &declared,
    );
    assert_eq!(out["timeout_ms"], json!(5000));
    assert_eq!(out["checked"], json!(true));
    assert_eq!(out["ratio"], json!(1.5));
    // 非数字字符串保持原样（由工具报类型错）
    let out = coerce_params(json!({"timeout_ms": "abc"}), &declared);
    assert_eq!(out["timeout_ms"], json!("abc"));
}

#[test]
fn unknown_param_warnings() {
    let declared = json!({"id": {"type": "string"}, "selector": {"type": "string"}});
    let w = unknown_params(&json!({"id": "a", "element": "x", "foo": 1}), &declared);
    assert!(w.iter().any(|s| s.contains("'element'")), "{w:?}");
    assert!(w.iter().any(|s| s.contains("'foo'")), "{w:?}");
    // 已知参数不告警
    assert!(unknown_params(&json!({"id": "a"}), &declared).is_empty());
}

#[test]
fn prepare_params_alias_then_coerce() {
    let declared = json!({"script": {"type": "string"}, "timeout_ms": {"type": "integer"}});
    let (out, warns) = prepare_params(json!({"code": "1+1", "timeout_ms": "100"}), &declared);
    assert_eq!(out["script"], json!("1+1"));
    assert_eq!(out["timeout_ms"], json!(100));
    assert!(warns.is_empty(), "{warns:?}");
}

#[test]
fn dialect_end_to_end_via_sdk() {
    let s = sdk();
    // MCP click by snapshot letter (on the initial page)
    assert!(s.tool_call("browser_click", json!({"id": "g"})).is_ok());
    // MCP navigate
    assert!(s
        .tool_call("browser_navigate", json!({"url": "https://example.com"}))
        .is_ok());
    // MCP resize → set_viewport
    assert!(s
        .tool_call("browser_resize", json!({"width": 800, "height": 600}))
        .is_ok());
    // MCP handle_dialog {accept:false} → dismiss
    assert!(s
        .tool_call("browser_handle_dialog", json!({"accept": false}))
        .is_ok());
    // select_option via 'values' (MCP shape)
    let _ = s.tool_call(
        "browser_select_option",
        json!({"id": "e", "values": ["pro"]}),
    );
    // execute_js accepts Playwright-style `function` (not a "missing script" error).
    if let Err(e) = s.tool_call("execute_js", json!({"function": "() => 1"})) {
        assert!(
            !e.message.contains("missing required param"),
            "function param must be accepted: {}",
            e.message
        );
    }
}

#[test]
fn unknown_param_surfaces_warning_in_result() {
    let s = sdk();
    let v = s.tool_call("get_page_text", json!({"bogus": 1})).unwrap();
    assert!(
        v.get("warnings").is_some(),
        "expected warnings for unknown param, got {v}"
    );
}
