// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! 参数矩阵健壮性测试（契约规范 §2.2 / §9.4）。
//!
//! 对**每一个**注册工具，喂入：空参数 / 类型错误 / 未知参数 / 别名参数，
//! 断言：不 panic、错误为 `invalid_argument`（或优雅失败），未知参数产生
//! `warnings`。这是"不靠真机反馈就能发现参数类 bug"的一层。

use fastbrowser::sdk::Fastbrowser;
use fastbrowser::Config;
use serde_json::{json, Value};
use std::panic::{catch_unwind, AssertUnwindSafe};

fn sdk() -> Fastbrowser {
    let s = Fastbrowser::new();
    s.init(Config::default()).unwrap();
    s.open("https://example.com/login").unwrap();
    s
}

/// These tests only assert "no panic"; a mock page never satisfies a
/// `wait_for_*` condition, so clamp its timeout to avoid waiting out the
/// real defaults for every tool in the matrix.
fn clamp_wait(name: &str, mut args: Value) -> Value {
    if name.starts_with("wait_for_") || name == "wait" {
        if let Some(o) = args.as_object_mut() {
            o.insert("timeout_ms".to_string(), json!(20));
        } else {
            args = json!({"timeout_ms": 20});
        }
    }
    args
}

fn tool_names(s: &Fastbrowser) -> Vec<String> {
    s.tool_list()
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t["name"].as_str().map(String::from))
        .collect()
}

/// 每个工具对空参数必须"优雅"（Ok 或 Err），绝不能 panic。
#[test]
fn every_tool_survives_empty_params() {
    let s = sdk();
    for name in tool_names(&s) {
        let r = catch_unwind(AssertUnwindSafe(|| {
            s.tool_call(&name, clamp_wait(&name, json!({})))
        }));
        assert!(r.is_ok(), "tool '{name}' panicked on empty params");
    }
}

/// 每个工具对"错误的参数类型"必须不 panic。
#[test]
fn every_tool_survives_wrong_param_types() {
    let s = sdk();
    for name in tool_names(&s) {
        // 通用：给一个对象塞进本该是标量的位置
        for bad in [
            json!({"url": {"nested": true}}),
            json!({"id": 12345}),
            json!({"selector": [1, 2, 3]}),
            json!({"text": {"a": 1}}),
            json!({"timeout_ms": "not-a-number"}),
            json!({"tab": "not-a-number"}),
        ] {
            let r = catch_unwind(AssertUnwindSafe(|| {
                s.tool_call(&name, clamp_wait(&name, bad.clone()))
            }));
            assert!(r.is_ok(), "tool '{name}' panicked on params {bad}");
        }
    }
}

/// 未知参数：不报错，但在结果里给出 `warnings`（让模型自纠）。
#[test]
fn unknown_param_is_warned_not_fatal() {
    let s = sdk();
    for name in tool_names(&s) {
        let r = s.tool_call(
            &name,
            clamp_wait(&name, json!({"__definitely_unknown__": 1})),
        );
        if let Ok(v) = r {
            if let Value::Object(_) = v {
                let has = v
                    .get("warnings")
                    .and_then(|w| w.as_array())
                    .map(|a| {
                        a.iter()
                            .any(|s| s.as_str().unwrap_or("").contains("__definitely_unknown__"))
                    })
                    .unwrap_or(false);
                assert!(has, "tool '{name}' should warn on unknown param, got {v}");
            }
        }
    }
}

/// 别名参数在工具声明了目标参数时被归一化（且不再算未知参数）。
#[test]
fn alias_params_are_normalized() {
    let s = sdk();
    // execute_js 声明 script；`code` 应被映射为 script。
    let v = s.tool_call("execute_js", json!({"code": "1+1"}));
    assert!(v.is_ok(), "code alias should map to script: {v:?}");
    // navigate 声明 url；`uri` 应被映射。
    let v = s.tool_call("navigate", json!({"uri": "https://example.com"}));
    assert!(v.is_ok(), "uri alias should map to url: {v:?}");
}
