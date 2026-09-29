// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Playwright 选择器引擎测试（mock 引擎；确定性，无需浏览器）。
//!
//! mock 无 DOM 层级，链式 `>>` 近似取最后一段；但引擎前缀、`:visible`、
//! `:has-text()`、`text=`、`nth=` 均可用。真 Chromium 覆盖见 `selector_engine_cdp.rs`。

use fastbrowser::sdk::Fastbrowser;
use fastbrowser::Config;
use serde_json::json;

fn sdk() -> Fastbrowser {
    let s = Fastbrowser::new();
    s.init(Config::for_engine("mock")).unwrap();
    s.open("https://example.com/login").unwrap();
    s
}

fn clicked(s: &Fastbrowser, selector: &str) -> bool {
    s.tool_call("click", json!({ "selector": selector }))
        .map(|v| v.get("clicked").is_some())
        .unwrap_or(false)
}

#[test]
fn chaining_and_engine_prefixes() {
    // 链式（mock 近似取最后一段）
    assert!(clicked(&sdk(), "form >> button"));
    // 段引擎前缀
    assert!(clicked(&sdk(), "text=Submit"));
    assert!(clicked(&sdk(), "role=button"));
    assert!(clicked(&sdk(), "css=button"));
    assert!(clicked(&sdk(), "xpath=//button"));
}

#[test]
fn pseudo_classes() {
    // :visible
    assert!(clicked(&sdk(), "button:visible"));
    // :has-text("...")（引号）
    assert!(clicked(&sdk(), "button:has-text(\"Submit\")"));
    // :has-text('...')（单引号）
    assert!(clicked(&sdk(), "button:has-text('Submit')"));
    // :text(...)
    assert!(clicked(&sdk(), "button:text(\"Submit\")"));
}

#[test]
fn nth_snapshot_index() {
    // `nth=N` 映射为快照下标（0-based）；登录页第 6 个可交互元素是 Submit 按钮。
    assert!(clicked(&sdk(), "nth=5"));
}

#[test]
fn not_found_is_error_not_panic() {
    let s = sdk();
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        s.tool_call(
            "click",
            json!({"selector": "button:has-text(\"__nope__\")"}),
        )
    }));
    assert!(r.is_ok(), "must not panic");
    assert!(r.unwrap().is_err(), "should be a graceful error");
}
