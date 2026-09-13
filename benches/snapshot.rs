// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! 轻量基准（无 criterion 依赖，手动计时）：
//!
//!   cargo bench
//!
//! 覆盖：快照生成 / 工具调用 / 截图 / JS 求值。

use std::time::{Duration, Instant};

use fastbrowser::engine::{BrowserEngine, TabOptions};
use fastbrowser::engines::mock::MockEngine;
use serde_json::json;

fn bench<F: FnMut()>(name: &str, n: u32, mut f: F) -> Duration {
    // 预热
    for _ in 0..3 {
        f();
    }
    let start = Instant::now();
    for _ in 0..n {
        f();
    }
    let dur = start.elapsed();
    println!(
        "{name:24} {n:>8} iters  {:>8.2} µs/iter  ({:>8.2} ops/s)",
        dur.as_micros() as f64 / n as f64,
        n as f64 / dur.as_secs_f64()
    );
    dur
}

fn main() {
    let engine = MockEngine::new();
    let tab = engine
        .create_tab("https://example.com/login", &TabOptions::default())
        .unwrap();

    bench("snapshot", 1000, || {
        let _ = engine.snapshot(tab).unwrap();
    });

    bench("evaluate_js", 2000, || {
        let _ = engine
            .evaluate(tab, "document.querySelectorAll('a').length")
            .unwrap();
    });

    bench("get_links", 2000, || {
        let _ = engine.get_links(tab).unwrap();
    });

    bench("screenshot", 200, || {
        let _ = engine.screenshot(tab).unwrap();
    });

    let sdk = fastbrowser::Fastbrowser::new();
    sdk.init(fastbrowser::Config::default()).unwrap();
    sdk.open("https://example.com").unwrap();
    bench("tool_call(snapshot)", 1000, || {
        let _ = sdk.tool_call("snapshot", json!({})).unwrap();
    });
}
