// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! 错误指引测试：模型传错工具名/参数时，错误消息应给出可自纠的提示。

use fastbrowser::sdk::Fastbrowser;
use fastbrowser::Config;
use serde_json::json;

fn sdk() -> Fastbrowser {
    let s = Fastbrowser::new();
    s.init(Config::for_engine("mock")).unwrap();
    s
}

#[test]
fn unknown_tool_suggests_close_match_and_list_hint() {
    let s = sdk();
    let e = s.tool_call("clik", json!({})).unwrap_err().to_string();
    assert!(e.contains("unknown tool"), "{e}");
    assert!(e.contains("did you mean 'click'"), "{e}");
    assert!(e.contains("list available tools"), "{e}");
}

#[test]
fn missing_param_lists_valid_params_and_example() {
    let s = sdk();
    let e = s.tool_call("navigate", json!({})).unwrap_err().to_string();
    assert!(e.contains("missing required param"), "{e}");
    assert!(e.contains("valid params"), "{e}");
    assert!(e.contains("url"), "{e}");
    assert!(e.contains("example"), "{e}");
}

#[test]
fn wrong_param_type_is_enriched() {
    let s = sdk();
    let e = s
        .tool_call("navigate", json!({"url": 123}))
        .unwrap_err()
        .to_string();
    assert!(e.contains("param 'url'"), "{e}");
    assert!(e.contains("valid params"), "{e}");
}

#[test]
fn unknown_tool_without_close_match_still_hints() {
    let s = sdk();
    let e = s
        .tool_call("zzzzzzzzzz", json!({}))
        .unwrap_err()
        .to_string();
    assert!(e.contains("unknown tool"), "{e}");
    assert!(e.contains("list available tools"), "{e}");
}
