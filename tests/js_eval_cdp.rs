// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! JS 求值契约 —— **真 Chromium（CDP 引擎）** 版本。
//!
//! `js_eval_contract.rs` 用 Node 作为参考引擎验证 `wrap_script` 的输出；本文件
//! 把同一批脚本形态跑在真实浏览器上（`execute_js` → `Runtime.evaluate`
//! `awaitPromise:true`），确保内核 → 引擎 → 宿主的整条链路一致。
//!
//! 前置：本机有 Chrome/Chromium（或 `CHROME_PATH`）。未找到时 SKIP。

#![cfg(all(feature = "engine-cdp", feature = "heavy-tests"))]

mod common;

use fastbrowser::sdk::Fastbrowser;
use fastbrowser::Config;
use serde_json::{json, Value};

const PAGE: &str = "data:text/html,<title>JSEvalCDP</title><h1>hi</h1>";

fn setup() -> Option<(Fastbrowser, common::BrowserGuard)> {
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
    Some((s, guard))
}

#[test]
fn cdp_js_eval_returns_expected_values() {
    let Some((s, _guard)) = setup() else {
        eprintln!("SKIP: no chrome");
        return;
    };
    s.open(PAGE).expect("open page");

    let cases: &[(&str, Value)] = &[
        ("1+1", json!(2)),
        ("'hello'", json!("hello")),
        ("[1,2,3].length", json!(3)),
        ("(function(){ return 1+1; })()", json!(2)),
        ("(() => 42)()", json!(42)),
        ("(async () => 42)()", json!(42)),
        (
            "(async () => { await new Promise(r => setTimeout(r, 1)); return 'ok'; })()",
            json!("ok"),
        ),
        ("return 7", json!(7)),
        ("const x = 2; return x*3;", json!(6)),
        ("'return'", json!("return")),
        ("document.title", json!("JSEvalCDP")),
        (
            "(function(){ return [1,2,3].map(x => x*2); })()",
            json!([2, 4, 6]),
        ),
        (
            "(function(){ return {a: 1, b: 'x'}; })()",
            json!({"a": 1, "b": "x"}),
        ),
        ("Math.max(1, 2, 3)", json!(3)),
        (
            "(function(){ return document.querySelectorAll('h1').length; })()",
            json!(1),
        ),
    ];
    for (script, expected) in cases {
        let v = s
            .tool_call("execute_js", json!({"script": script}))
            .unwrap_or_else(|e| panic!("script {script:?} errored: {e}"));
        assert_eq!(&v["result"], expected, "script {script:?} → {v}");
    }
}

#[test]
fn cdp_execute_js_function_param() {
    let Some((s, _guard)) = setup() else {
        eprintln!("SKIP: no chrome");
        return;
    };
    s.open(PAGE).expect("open page");
    // Playwright MCP style: `function` is CALLED.
    let v = s
        .tool_call("execute_js", json!({"function": "() => document.title"}))
        .unwrap();
    assert_eq!(v["result"], json!("JSEvalCDP"), "{v}");
}

#[test]
fn cdp_js_eval_error_and_edge_cases() {
    let Some((s, _guard)) = setup() else {
        eprintln!("SKIP: no chrome");
        return;
    };
    s.open(PAGE).expect("open page");

    // `undefined` normalizes to null.
    let v = s
        .tool_call("execute_js", json!({"script": "undefined"}))
        .unwrap();
    assert!(v["result"].is_null(), "undefined should be null: {v}");

    // A throwing script: the tool must either error or surface an {error} object
    // — never panic and never return a bogus value.
    let r = s.tool_call(
        "execute_js",
        json!({"script": "(function(){ throw new Error('boom'); })()"}),
    );
    match r {
        Ok(v) => assert!(
            v["result"].is_null() || v["result"]["error"].is_string(),
            "throw should surface as error/null, got {v}"
        ),
        Err(_) => {}
    }

    // Syntax error, non-serializable (circular), and a bare function value must
    // all be handled gracefully (no panic, no hang).
    for script in [
        "this is not js",
        "(() => { const a = {}; a.self = a; return a; })()",
        "(function(){})",
        "(() => { const f = () => 1; return f; })()",
    ] {
        let _ = s.tool_call("execute_js", json!({"script": script}));
    }
}
