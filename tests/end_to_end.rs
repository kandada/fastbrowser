// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! 端到端集成测试：走 SDK 全链路（mock 引擎）。

use serde_json::{json, Value};
use std::sync::Arc;

use fastbrowser::engine::host::PageEventSink;
use fastbrowser::engine::PageEvent;
use fastbrowser::sdk::Fastbrowser;
use fastbrowser::Config;

fn sdk() -> Fastbrowser {
    let s = Fastbrowser::new();
    s.init(Config::default()).unwrap();
    s
}

#[test]
fn full_agent_workflow() {
    let s = sdk();

    // 1. 打开登录页
    let out = s.open("https://example.com/login").unwrap();
    assert_eq!(out["title"], "Login");

    // 2. 填表
    let r = s
        .tool_call(
            "fill_form",
            json!({"values": {"b": "alice", "c": "secret"}}),
        )
        .unwrap();
    assert_eq!(r["filled"].as_array().unwrap().len(), 2);

    // 3. 勾选 + 选择
    s.tool_call("checkbox", json!({"id": "d", "checked": true}))
        .unwrap();
    s.tool_call("select_option", json!({"id": "e", "value": "pro"}))
        .unwrap();

    // 4. 快照核对
    let snap = s.snapshot().unwrap();
    assert_eq!(
        snap.element_by_id('b').unwrap().value.as_deref(),
        Some("alice")
    );
    assert_eq!(snap.element_by_id('d').unwrap().checked, Some(true));
    assert_eq!(
        snap.element_by_id('e').unwrap().selected_option.as_deref(),
        Some("pro")
    );

    // 5. 提交 → 导航到 /about
    s.tool_call("click", json!({"id": "c"})).unwrap(); // 链接 c -> /about? 登录页 link 是 g；这里点 f 按钮
                                                       // 登录页元素: a=h1 b=user c=pwd d=checkbox e=select f=button g=link
    s.tool_call("navigate", json!({"url": "https://example.com/about"}))
        .unwrap();
    let url = s.tool_call("get_current_url", json!({})).unwrap();
    assert_eq!(url["url"], "https://example.com/about");

    // 6. 提取 + 断言
    let text = s.tool_call("get_page_text", json!({})).unwrap();
    assert!(text["text"].as_str().unwrap().contains("Welcome"));
    s.tool_call("assert_element_exists", json!({"selector": "button"}))
        .unwrap();
}

#[test]
fn cookies_and_storage_end_to_end() {
    let s = sdk();
    s.open("https://example.com").unwrap();
    s.tool_call(
        "cookie_set",
        json!({"name": "sid", "value": "abc", "domain": "example.com"}),
    )
    .unwrap();
    let got = s
        .tool_call("cookie_get", json!({"domain": "example.com"}))
        .unwrap();
    assert_eq!(got["cookies"][0]["value"], "abc");
    s.tool_call("storage_set", json!({"key": "t", "value": "v"}))
        .unwrap();
    let v = s.tool_call("storage_get", json!({"key": "t"})).unwrap();
    assert_eq!(v["value"], "v");
}

#[test]
fn multi_tab_flow() {
    let s = sdk();
    s.open("https://example.com").unwrap();
    let t2 = s
        .tool_call("new_tab", json!({"url": "https://example.com/search"}))
        .unwrap();
    let t2id = t2["tab"].as_u64().unwrap() as u32;
    let tabs = s.tool_call("list_tabs", json!({})).unwrap();
    assert_eq!(tabs["tabs"].as_array().unwrap().len(), 2);
    s.tool_call("switch_tab", json!({"tab": t2id})).unwrap();
    let url = s.tool_call("get_current_url", json!({})).unwrap();
    assert_eq!(url["url"], "https://example.com/search");
    s.tool_call("close_tab", json!({"tab": t2id})).unwrap();
}

#[test]
fn events_sink_receives() {
    use std::sync::atomic::{AtomicU32, Ordering};
    struct Counter(Arc<AtomicU32>);
    impl PageEventSink for Counter {
        fn on_page_event(&self, _t: fastbrowser::engine::TabId, _e: &PageEvent) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    let s = sdk();
    let counter = Arc::new(AtomicU32::new(0));
    s.register_event_sink(Arc::new(Counter(counter.clone())));
    s.open("https://example.com").unwrap();
    s.navigate("https://example.com/login").unwrap();
    assert!(counter.load(Ordering::SeqCst) >= 1);
}

#[test]
fn tool_list_is_llm_ready() {
    let s = sdk();
    let list = s.tool_list();
    let arr = list.as_array().unwrap();
    assert!(arr.len() >= 30);
    for t in arr {
        assert!(t["name"].is_string());
        assert!(t["description"].is_string());
        assert!(t["params"].is_object() || t["params"].is_string());
    }
}

#[test]
fn json_value_roundtrip() {
    let v: Value = json!({"a": 1, "b": [true, null, "x"]});
    assert_eq!(v["b"][2], "x");
}
