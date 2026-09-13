// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! 性能回归守卫：作为普通 `cargo test` 运行，用**宽松阈值**捕获严重性能回退
//! （如意外 O(n²)、误加阻塞等待）。真实基准见 `benches/perf.rs`（`cargo bench`）。
//!
//! 阈值取实测值的 ~100 倍量级余量，避免在慢 CI 上误报。

#[cfg(feature = "engine-cdp")]
mod common;

use std::time::Instant;

use fastbrowser::sdk::Fastbrowser;
use fastbrowser::Config;
use serde_json::json;

/// 运行 n 次，返回平均耗时（µs/次）。
fn avg_us(n: u32, mut f: impl FnMut()) -> f64 {
    for _ in 0..3 {
        f(); // 预热
    }
    let start = Instant::now();
    for _ in 0..n {
        f();
    }
    start.elapsed().as_micros() as f64 / n as f64
}

/// 断言 avg < budget（µs）。
fn assert_under(name: &str, avg: f64, budget: f64) {
    assert!(
        avg < budget,
        "perf regression: {name} took {avg:.1}µs/iter, budget {budget:.0}µs"
    );
    eprintln!("perf {name:30} {avg:9.1} µs/iter (budget {budget:.0})");
}

#[test]
fn mock_hot_paths_are_fast() {
    let sdk = Fastbrowser::new();
    sdk.init(Config::default()).unwrap();
    sdk.open("https://example.com/login").unwrap();

    assert_under(
        "mock snapshot",
        avg_us(500, || {
            let _ = sdk.snapshot().unwrap();
        }),
        1000.0,
    );
    assert_under(
        "mock screenshot",
        avg_us(50, || {
            let _ = sdk.screenshot().unwrap();
        }),
        50_000.0,
    );
    assert_under(
        "tool_call(light)",
        avg_us(500, || {
            let _ = sdk.tool_call("get_page_title", json!({})).unwrap();
        }),
        5000.0,
    );
    assert_under(
        "tool_list+schema",
        avg_us(50, || {
            let _ = sdk.tool_list();
        }),
        20_000.0,
    );
}

#[test]
fn session_persistence_is_fast() {
    let sdk = Fastbrowser::new();
    sdk.init(Config::default()).unwrap();
    sdk.open("https://example.com/login").unwrap();
    let path = format!(
        "{}/fb_perf_state.json",
        std::env::temp_dir().to_str().unwrap()
    );

    assert_under(
        "session_save",
        avg_us(100, || {
            let _ = sdk.session_save(&path).unwrap();
        }),
        10_000.0,
    );
    assert_under(
        "session_load",
        avg_us(100, || {
            let _ = sdk.session_load(&path).unwrap();
        }),
        10_000.0,
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn png_decode_is_fast() {
    // 构造一张 128×128 RGBA PNG 并解码
    let png = {
        use std::io::Write;
        let mut scan = Vec::new();
        for _ in 0..128 {
            scan.push(0u8);
            scan.extend_from_slice(&[100u8, 150, 200, 255].repeat(128));
        }
        let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
        enc.write_all(&scan).unwrap();
        let idat = enc.finish().unwrap();
        let mut out = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        let mut chunk = |t: &[u8; 4], d: &[u8]| {
            out.extend_from_slice(&(d.len() as u32).to_be_bytes());
            out.extend_from_slice(t);
            out.extend_from_slice(d);
            out.extend_from_slice(&[0u8; 4]);
        };
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&128u32.to_be_bytes());
        ihdr.extend_from_slice(&128u32.to_be_bytes());
        ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
        chunk(b"IHDR", &ihdr);
        chunk(b"IDAT", &idat);
        chunk(b"IEND", &[]);
        out
    };
    assert_under(
        "png decode 128x128",
        avg_us(100, || {
            let _ = fastbrowser::png::decode_png(&png).unwrap();
        }),
        10_000.0,
    );
}

/// 真实浏览器关键路径（feature engine-cdp 时生效；无浏览器则跳过）。
#[cfg(feature = "engine-cdp")]
#[test]
fn chromium_hot_paths_are_reasonable() {
    use fastbrowser::engine::{BrowserEngine, ElementRef};

    let Some(chrome) = fastbrowser::engines::bundled::find_bundled_binary() else {
        eprintln!("SKIP chromium perf: no bundled chromium (run scripts/fetch-chromium.sh)");
        return;
    };
    let cfg = Config {
        engine: "bundled".into(),
        ..Config::default()
    };
    let Ok(engine) = fastbrowser::engines::bundled::BundledChromium::connect_engine(chrome, &cfg)
    else {
        eprintln!("SKIP chromium perf: launch failed");
        return;
    };
    let tab = engine.create_tab(
        "data:text/html,<html><body><h1>x</h1><button>go</button><input><a href='#x'>l</a></body></html>",
        &Default::default(),
    )
    .unwrap();

    assert_under(
        "chromium snapshot",
        avg_us(5, || {
            let _ = engine.snapshot(tab).unwrap();
        }),
        200_000.0,
    );
    assert_under(
        "chromium click",
        avg_us(5, || {
            engine
                .click_element(tab, &ElementRef::snapshot('a'))
                .unwrap();
        }),
        2_000_000.0,
    );
}
