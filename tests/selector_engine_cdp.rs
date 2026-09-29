// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Playwright 选择器引擎的真 Chromium 集成测试（feature `engine-cdp`）。
//!
//! 验证注入 JS 引擎（`engine::inject::fb_query_js`）：链式 `>>`、`:visible`、
//! `:has-text()`、`:text()`、`text=`、`role=...[name="..."]`、`nth=`。
//!
//! 前置：本机存在 Chrome/Chromium（或 `CHROME_PATH`）。未找到时打印 SKIP。

#![cfg(all(feature = "engine-cdp", feature = "heavy-tests"))]

mod common;

use std::time::{Duration, Instant};

use fastbrowser::sdk::Fastbrowser;
use fastbrowser::Config;
use serde_json::{json, Value};

const HTML: &str = r##"<!doctype html><html><head><title>Selector Engine</title></head>
<body>
  <div id="outer">
    <button id="go" onclick="document.getElementById('out').textContent='clicked'">Go</button>
    <button id="hidden" style="display:none">Hidden</button>
  </div>
  <div id="second"><button class="inner" onclick="document.getElementById('out').textContent='second'">Second</button></div>
  <span id="out">initial</span>
</body></html>"##;

fn wait_until<F: FnMut() -> bool>(what: &str, timeout: Duration, mut f: F) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if f() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("timeout waiting for {what}");
}

fn setup() -> Option<(Fastbrowser, common::BrowserGuard, String)> {
    let b = common::shared_browser(true).ok()?;
    let guard = common::browser_guard(b);
    let ws = b.ws.clone();
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("sel.html");
    std::fs::write(&file, HTML).unwrap();
    let url = common::file_url(&file);
    let s = Fastbrowser::new();
    s.init(Config {
        engine: "chromium".into(),
        cdp_url: Some(ws),
        ..Config::default()
    })
    .ok()?;
    s.open(&url).unwrap();
    wait_until("page loaded", Duration::from_secs(15), || {
        s.tool_call("get_page_title", json!({}))
            .ok()
            .and_then(|v| v["title"].as_str().map(|t| t.contains("Selector Engine")))
            .unwrap_or(false)
    });
    Some((s, guard, url))
}

fn out_text(s: &Fastbrowser) -> String {
    s.tool_call(
        "execute_js",
        json!({"script": "document.getElementById('out').textContent"}),
    )
    .ok()
    .and_then(|v| v["result"].as_str().map(String::from))
    .unwrap_or_default()
}

fn click_selector(s: &Fastbrowser, selector: &str) -> Value {
    s.tool_call("click", json!({ "selector": selector }))
        .unwrap_or_else(|e| panic!("click selector {selector:?} failed: {e}"))
}

#[test]
fn selector_engine_chaining_and_pseudos() {
    let Some((s, _g, _url)) = setup() else {
        eprintln!("SKIP: selector_engine_chaining_and_pseudos (no chrome)");
        return;
    };

    // 链式 `>>`
    click_selector(&s, "#outer >> button");
    wait_until("chained click", Duration::from_secs(5), || {
        out_text(&s) == "clicked"
    });

    // :has-text("...")
    click_selector(&s, "button:has-text(\"Go\")");
    wait_until("has-text click", Duration::from_secs(5), || {
        out_text(&s) == "clicked"
    });

    // :visible（#hidden 是 display:none，应选中可见的 Go）
    click_selector(&s, "button:visible");
    wait_until("visible click", Duration::from_secs(5), || {
        out_text(&s) == "clicked"
    });

    // :text(...)
    click_selector(&s, "button:text(\"Go\")");
    wait_until("text-pseudo click", Duration::from_secs(5), || {
        out_text(&s) == "clicked"
    });

    // 段引擎前缀 text=
    click_selector(&s, "text=Go");
    wait_until("text= click", Duration::from_secs(5), || {
        out_text(&s) == "clicked"
    });

    // role=...[name="..."]
    click_selector(&s, "role=button[name=\"Go\"]");
    wait_until("role name click", Duration::from_secs(5), || {
        out_text(&s) == "clicked"
    });

    // 链式 + CSS 组合，点第二个按钮
    click_selector(&s, "#second >> button.inner");
    wait_until("second click", Duration::from_secs(5), || {
        out_text(&s) == "second"
    });

    // 原生 CSS :has()（现代 Chromium 支持）
    click_selector(&s, "div:has(> button#go) > button#go");
    wait_until(":has click", Duration::from_secs(5), || {
        out_text(&s) == "clicked"
    });
}

#[test]
fn selector_engine_not_found_errors() {
    let Some((s, _g, _url)) = setup() else {
        eprintln!("SKIP: selector_engine_not_found_errors (no chrome)");
        return;
    };
    let r = s.tool_call(
        "click",
        json!({"selector": "button:has-text(\"__nope__\")"}),
    );
    assert!(r.is_err(), "missing selector should error, got {r:?}");
}
