// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! 打包集成 Chromium 的真机集成测试（feature `engine-cdp`）。
//!
//! 使用 `vendor/chromium/`（Chrome for Testing，由 scripts/fetch-chromium.sh 下载）
//! 作为随应用打包的 Chromium：`engine: "bundled"` 自动启动无头进程并走 CDP，
//! 驱动真实页面验证全链路。无 vendor/chromium 时 SKIP。

#![cfg(feature = "engine-cdp")]

mod common;

use std::time::{Duration, Instant};

use fastbrowser::sdk::Fastbrowser;
use fastbrowser::Config;

const HTML: &str = r##"<!doctype html><html><head><title>Bundled Chromium Test</title></head>
<body>
  <h1>Bundled CFT</h1>
  <input id="q" placeholder="search">
  <button id="go">Go</button>
  <a href="#next">Next link</a>
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

#[test]
fn bundled_chromium_end_to_end() {
    if fastbrowser::engines::bundled::find_bundled_binary().is_none() {
        eprintln!("SKIP: bundled_chromium_end_to_end (run scripts/fetch-chromium.sh)");
        return;
    }

    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("fixture.html");
    std::fs::write(&file, HTML).unwrap();
    let url = format!("file://{}", file.display());

    let sdk = Fastbrowser::new();
    let cfg = Config {
        engine: "bundled".into(),
        ..Config::default()
    };
    sdk.init(cfg).unwrap();

    let out = sdk.open(&url).unwrap();
    assert!(out["tab"].is_number());

    wait_until("page loaded", Duration::from_secs(15), || {
        sdk.snapshot()
            .map(|s| s.title.contains("Bundled Chromium Test"))
            .unwrap_or(false)
    });

    let snap = sdk.snapshot().unwrap();
    assert_eq!(snap.title, "Bundled Chromium Test");
    let input = snap
        .interactive
        .iter()
        .find(|e| e.tag == "input")
        .unwrap()
        .id;
    let btn = snap
        .interactive
        .iter()
        .find(|e| e.tag == "button")
        .unwrap()
        .id;

    sdk.tool_call(
        "type",
        serde_json::json!({"id": input.to_string(), "text": "hello"}),
    )
    .unwrap();
    let v = sdk
        .tool_call(
            "execute_js",
            serde_json::json!({"script": "document.getElementById('q').value"}),
        )
        .unwrap();
    assert_eq!(v["result"], "hello");

    sdk.tool_call("click", serde_json::json!({"id": btn.to_string()}))
        .unwrap();
    let links = sdk
        .tool_call("extract_links", serde_json::json!({}))
        .unwrap();
    assert!(links["links"].as_array().unwrap().iter().any(|l| l["url"]
        .as_str()
        .map(|u| u.contains("#next"))
        .unwrap_or(false)));

    let meta = sdk
        .tool_call("get_page_meta", serde_json::json!({}))
        .unwrap();
    assert_eq!(meta["meta"]["title"], "Bundled Chromium Test");

    sdk.set_viewport(800, 600).unwrap();
    let _ = sdk.screenshot().unwrap();

    let info = sdk.get_info();
    assert_eq!(info.engine, "chromium"); // BundledChromium 引擎实为 ChromiumCdpEngine
    eprintln!("PASS: bundled_chromium_end_to_end");
}

/// 有头（真实窗口）模式：`RenderingMode::Hosted` 时启动的 Chromium 不传
/// `--headless`，会弹出一个真实浏览器窗口，人可直接操作；Agent 经 CDP 驱动
/// 同一实例。测试运行时屏幕上会出现一个 Chrome 窗口（这是预期现象）。
#[test]
fn bundled_chromium_headed_window() {
    if fastbrowser::engines::bundled::find_bundled_binary().is_none() {
        eprintln!("SKIP: bundled_chromium_headed_window (run scripts/fetch-chromium.sh)");
        return;
    }
    let sdk = fastbrowser::sdk::Fastbrowser::new();
    let cfg = fastbrowser::Config::for_engine("bundled").hosted();
    sdk.init(cfg).unwrap();

    sdk.open("https://example.com").unwrap();

    wait_until("headed page loaded", Duration::from_secs(20), || {
        sdk.snapshot()
            .map(|s| s.title.contains("Example"))
            .unwrap_or(false)
    });

    // 聚焦真实窗口 + 截图（有头下同样走 CDP 全链路）。
    sdk.focus_window().unwrap();
    let img = sdk.screenshot().unwrap();
    assert!(img.is_valid() && img.width > 0);

    let status = sdk.status();
    assert_eq!(status["rendering_mode"], "hosted");
    sdk.shutdown();
    eprintln!("PASS: bundled_chromium_headed_window");
}
