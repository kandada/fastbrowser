// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! JS 求值契约测试（契约规范 §3.3 / §9.1）。
//!
//! 内核 `wrap_script` 的输出会被宿主当作表达式求值（CDP `Runtime.evaluate`
//! 带 `awaitPromise:true`）。这里用真实 JS 引擎（Node）执行 `wrap_script` 的
//! 结果，断言返回值，从而堵住 "IIFE 被二次包裹 → undefined → null" 这类
//! 纯 mock 测不到的 bug。无 Node 时自动跳过（CI 无 Node 仍绿）。

use fastbrowser::tools::advanced::{call_function, wrap_script};
use serde_json::{json, Value};
use std::process::Command;

fn node_available() -> bool {
    Command::new("node")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// 在 Node 里把 `expr` 当表达式求值（await Promise），返回 JSON 值或错误串。
fn eval_expr(expr: &str) -> Result<Value, String> {
    let code = format!(
        "(async () => {{ try {{ const v = await ({expr}); return JSON.stringify({{ok:true,v}}); }} \
         catch (e) {{ return JSON.stringify({{ok:false,error:String(e)}}); }} }})().then(s => process.stdout.write(s));"
    );
    let out = Command::new("node")
        .arg("-e")
        .arg(&code)
        .output()
        .map_err(|e| format!("node spawn: {e}"))?;
    let s = String::from_utf8_lossy(&out.stdout);
    let v: Value = serde_json::from_str(s.trim()).unwrap_or(Value::Null);
    if v["ok"] == Value::Bool(true) {
        Ok(v.get("v").cloned().unwrap_or(Value::Null))
    } else {
        Err(v["error"].as_str().unwrap_or("error").to_string())
    }
}

#[test]
fn wrapped_scripts_return_expected_values() {
    if !node_available() {
        eprintln!("node not found; skipping js_eval_contract");
        return;
    }
    let cases: &[(&str, Value)] = &[
        ("1+1", json!(2)),
        ("'hello'", json!("hello")),
        ("[1,2,3].length", json!(3)),
        // 回归核心：完整 IIFE 不能被二次包裹。
        ("(function(){ return 1+1; })()", json!(2)),
        ("(() => 42)()", json!(42)),
        // async IIFE（宿主必须 await）。
        ("(async () => 42)()", json!(42)),
        (
            "(async () => { await new Promise(r => setTimeout(r, 1)); return 'ok'; })()",
            json!("ok"),
        ),
        // 顶层 return 语句块 → 包成 IIFE。
        ("return 7", json!(7)),
        ("const x = 2; return x*3;", json!(6)),
        // 含 "return" 的纯表达式不得被包裹。
        ("'return'", json!("return")),
        ("'a'.repeat(3)", json!("aaa")),
        // 更多形态：数组/对象/箭头/多语句/嵌套 async/内建。
        (
            "(function(){ return [1,2,3].map(x => x*2); })()",
            json!([2, 4, 6]),
        ),
        ("(() => { let a = 1, b = 2; return a + b; })()", json!(3)),
        (
            "(async () => { const r = await Promise.resolve(7); return r; })()",
            json!(7),
        ),
        (
            "(function(){ return {a: 1, b: 'x'}; })()",
            json!({"a": 1, "b": "x"}),
        ),
        (
            "(function(){ return [1,2,3].filter(n => n > 1); })()",
            json!([2, 3]),
        ),
        ("Math.max(1, 2, 3)", json!(3)),
        ("'a,b,c'.split(',').length", json!(3)),
        ("(function(){ return (() => 21)() * 2; })()", json!(42)),
    ];
    for (script, expected) in cases {
        let w = wrap_script(script);
        let got = eval_expr(&w)
            .unwrap_or_else(|e| panic!("script {script:?} wrapped as {w:?} errored: {e}"));
        assert_eq!(&got, expected, "script {script:?} wrapped as {w:?}");
    }
}

#[test]
fn wrap_script_does_not_double_wrap() {
    // 已是完整表达式 → 原样。
    for s in [
        "(function(){ return 1; })()",
        "(async () => 1)()",
        "(() => 1)()",
        "document.title",
        "'return'",
    ] {
        assert_eq!(wrap_script(s), s, "must be left untouched: {s}");
    }
    // 顶层 return 块 → 包成 IIFE。
    assert!(wrap_script("return 1").starts_with("(function(){"));
    assert!(wrap_script("let a=1; return a;").starts_with("(function(){"));
}

#[test]
fn call_function_invokes_function_expression() {
    if !node_available() {
        return;
    }
    assert_eq!(eval_expr(&call_function("() => 99")).unwrap(), json!(99));
    assert_eq!(
        eval_expr(&call_function("function(){ return 'hi'; }")).unwrap(),
        json!("hi")
    );
    assert_eq!(
        eval_expr(&call_function("() => ({a: 1}).a")).unwrap(),
        json!(1)
    );
}

/// Multi-statement scripts without a top-level `return` now expose the **last
/// statement's value** (a REPL-like completion value) instead of yielding
/// `undefined`. A single expression is still passed through unchanged.
#[test]
fn multi_statement_returns_last_value() {
    if !node_available() {
        return;
    }
    let w = wrap_script("1+1; 2+2");
    assert_eq!(
        w, "(function(){ 1+1; return (2+2); })()",
        "wrap and return the last statement"
    );
    assert_eq!(eval_expr(&w).unwrap(), json!(4));
    // A single expression is untouched.
    assert_eq!(wrap_script("1+1"), "1+1");
}
