// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! 会话回放测试（契约规范 §9.5）。
//!
//! 语料来自真实设备导出的会话（`session_*.json`）里模型实际发出的
//! `browser_call` 调用。断言：
//! 1. 每个工具名都能解析到已注册工具（缺别名 = 回归）；
//! 2. 回放不 panic；
//! 3. 模型**漏传必填参数**时给出可执行的 `invalid_argument`（而非莫名报错）。
//!
//! 这把"真机反馈"固化成了回归，并让模型方言驱动别名扩展。

use fastbrowser::engine::EngineError;
use fastbrowser::sdk::Fastbrowser;
use fastbrowser::tools::aliases::{normalize_params, resolve_tool_alias};
use fastbrowser::Config;
use serde_json::{json, Value};
use std::panic::{catch_unwind, AssertUnwindSafe};

fn sdk() -> Fastbrowser {
    let s = Fastbrowser::new();
    s.init(Config::default()).unwrap();
    s.open("https://example.com/login").unwrap();
    s
}

/// (工具名, 参数) —— 摘自真实会话的 `browser_call`。
const CORPUS: &[(&str, &str)] = &[
    ("click", r##"{"id": "#list_zwz"}"##),
    ("close_tab", r##"{"tab": "3"}"##),
    (
        "download",
        r##"{"path": "/projects/Test1/zhicheng_images/zhi_cheng_01.jpg", "url": "https://img8.iplant.cn/image2/236/30DEC561AE293746.jpg"}"##,
    ),
    (
        "execute_js",
        r##"{"expr": "Array.from(document.querySelectorAll('input')).map((el,i)=>({i,type:el.type,name:el.name,placeholder:el.placeholder,id:el.id,form:el.form?el.form.action:null}))"}"##,
    ),
    (
        "execute_js",
        r##"{"expression": "(function(){const items=document.querySelectorAll('.vcode-item, [class*=vcode], .verify-item'); const all=[]; document.querySelectorAll('*').forEach(el=>{const t=(el.innerText||'').trim(); if(t.length===1 && /[\\u4e00-\\u9fa5]/.test(t)){const r=el.getBoundingClientRect(); if(r.width>0&&r.height>0) all.push({tag:el.tagName,cls:el.className,text:t,x:Math.round(r.left+r.width/2),y:Math.round(r.top+r.height/2)});}}); return all.slice(0,40);})()"}"##,
    ),
    (
        "execute_js",
        r##"{"script": "return document.querySelector('meta[name=description]')?.content || document.querySelector('meta[property=\"og:description\"]')?.content || null"}"##,
    ),
    ("extract_html", r##"{}"##),
    ("extract_html", r##"{"max_chars": "3000"}"##),
    ("extract_images", r##"{"limit": 50}"##),
    ("extract_images", r##"{"max_chars": "5000"}"##),
    ("extract_links", r##"{"limit": 60}"##),
    ("extract_links", r##"{"max_chars": "5000"}"##),
    ("extract_text", r##"{}"##),
    ("extract_text", r##"{"max_chars": "10000"}"##),
    ("find_elements", r##"{"limit": 30, "selector": "a"}"##),
    ("find_elements", r##"{"max_chars": "3000"}"##),
    ("find_elements", r##"{"selector": "input"}"##),
    ("get_accessibility_tree", r##"{}"##),
    ("get_active_tab", r##"{}"##),
    ("get_active_tab", r##"{"max_chars": "100"}"##),
    ("get_current_url", r##"{}"##),
    ("get_element_text", r##"{"id": "a"}"##),
    ("get_page_meta", r##"{}"##),
    ("get_page_meta", r##"{"max_chars": 4000}"##),
    ("get_page_text", r##"{}"##),
    ("get_page_text", r##"{"max_chars": 8000}"##),
    ("get_page_title", r##"{}"##),
    ("list_tabs", r##"{}"##),
    (
        "navigate",
        r##"{"url": "https://baike.baidu.com/item/Budgie"}"##,
    ),
    (
        "new_tab",
        r##"{"url": "https://all-americaselections.org/product/basil-siam-queen/"}"##,
    ),
    ("reload", r##"{}"##),
    (
        "reload",
        r##"{"url": "https://baike.baidu.com/item/ThaiBasil"}"##,
    ),
    ("search", r##"{"limit": 100, "query": "Rosa"}"##),
    ("switch_tab", r##"{"tab": 3}"##),
    ("type", r##"{"id": "txt_key", "text": "SnakePlant"}"##),
    (
        "wait_for_element",
        r##"{"selector": "div.lemmaWgt-focus, .mainContent_R21Ht, .body-wrapper, h1", "timeout_ms": "10000"}"##,
    ),
    ("wait_for_load_state", r##"{}"##),
    ("wait_for_load_state", r##"{"selector": "body"}"##),
    (
        "wait_for_load_state",
        r##"{"state": "domcontentloaded", "timeout_ms": 15000}"##,
    ),
    ("wait_for_load_state", r##"{"timeout": "15000"}"##),
    ("wait_for_load_state", r##"{"timeout_ms": "10000"}"##),
    ("wait_for_navigation", r##"{}"##),
    ("wait_for_navigation", r##"{"timeout_ms": 15000}"##),
    (
        "wait_for_text",
        r##"{"selector": ".mainContent_RkEDc, .body-wrapper, .J-lemma-content, .mainContent_aA4T9, #contentContainer", "timeout_ms": "8000"}"##,
    ),
    ("wait_for_text", r##"{"text": "Budgie"}"##),
    (
        "wait_for_text",
        r##"{"text": "ThaiBasil", "timeout_ms": 15000}"##,
    ),
    ("wait_for_text", r##"{"timeout_ms": 8000}"##),
];

fn tool_specs(s: &Fastbrowser) -> Value {
    s.tool_list()
}

fn required_params(list: &Value, canonical: &str) -> Vec<String> {
    list.as_array()
        .and_then(|a| a.iter().find(|t| t["name"] == canonical))
        .and_then(|t| t["params"].as_object())
        .map(|params| {
            params
                .iter()
                .filter(|(_, def)| def["required"].as_bool().unwrap_or(false))
                .map(|(k, _)| k.clone())
                .collect()
        })
        .unwrap_or_default()
}

fn declared_params(list: &Value, canonical: &str) -> Value {
    list.as_array()
        .and_then(|a| a.iter().find(|t| t["name"] == canonical))
        .map(|t| t["params"].clone())
        .unwrap_or_else(|| json!({}))
}

#[test]
fn corpus_tool_names_resolve_to_registered_tools() {
    let s = sdk();
    let list = tool_specs(&s);
    let names: Vec<&str> = list
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect();
    let mut bad = Vec::new();
    for (tool, _) in CORPUS {
        let canonical = resolve_tool_alias(tool);
        if !names.contains(&canonical.as_str()) {
            bad.push(format!("{tool} -> {canonical} (unregistered)"));
        }
    }
    assert!(
        bad.is_empty(),
        "unresolved session tool names:\n{}",
        bad.join("\n")
    );
}

#[test]
fn corpus_calls_do_not_panic() {
    let s = sdk();
    for (tool, args) in CORPUS {
        let mut args: Value = serde_json::from_str(args).unwrap();
        // This test only asserts "no panic". A mock page never satisfies a
        // `wait_for_*` condition, so clamping the timeout avoids waiting out
        // the real 8–15s values (which made this test ~30s).
        if tool.starts_with("wait_for_") || *tool == "wait" {
            if let Some(obj) = args.as_object_mut() {
                for k in ["timeout_ms", "timeout", "timeout_secs"] {
                    if obj.contains_key(k) {
                        obj.insert(k.to_string(), json!(20));
                    }
                }
            }
        }
        let r = catch_unwind(AssertUnwindSafe(|| s.tool_call(tool, args.clone())));
        assert!(r.is_ok(), "tool '{tool}' panicked with {args}");
    }
}

#[test]
fn missing_required_param_is_actionable() {
    let s = sdk();
    let list = tool_specs(&s);
    let mut problems = Vec::new();
    for (tool, args) in CORPUS {
        let args: Value = serde_json::from_str(args).unwrap();
        let canonical = resolve_tool_alias(tool);
        let declared = declared_params(&list, &canonical);
        let normalized = normalize_params(args.clone(), &declared);
        let required = required_params(&list, &canonical);
        let missing: Vec<&String> = required
            .iter()
            .filter(|r| normalized.get(r.as_str()).is_none())
            .collect();
        if missing.is_empty() {
            continue;
        }
        match s.tool_call(tool, args) {
            Err(e) => {
                let msg = e.message.to_string();
                if !(matches!(e.kind, fastbrowser::engine::ErrorKind::InvalidArgument)
                    && msg.contains("missing required param"))
                {
                    problems.push(format!("{tool} missing {missing:?} but error was: {msg}"));
                }
            }
            Ok(v) => problems.push(format!("{tool} missing {missing:?} but returned Ok: {v}")),
        }
    }
    assert!(
        problems.is_empty(),
        "missing-required feedback not actionable:\n{}",
        problems.join("\n")
    );
}

#[test]
fn error_kind_is_serializable() {
    let e = EngineError::invalid("x");
    assert_eq!(format!("{:?}", e.kind), "InvalidArgument");
}
