// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! 综合性能基准（无 criterion 依赖，手动计时）：
//!
//!   cargo bench                          # mock 引擎基准
//!   cargo bench --features engine-cdp    # 额外包含真实 Chromium 延迟基准
//!
//! 覆盖：快照 / 工具分发 / JSON Schema 生成 / PNG 解码 / 会话持久化 /
//! C ABI 调用 / （engine-cdp）真实浏览器快照·导航·点击延迟。

use std::time::{Duration, Instant};

use fastbrowser::engine::BrowserEngine;
use serde_json::json;

fn bench<F: FnMut()>(name: &str, n: u32, mut f: F) -> Duration {
    for _ in 0..3 {
        f();
    }
    let start = Instant::now();
    for _ in 0..n {
        f();
    }
    let dur = start.elapsed();
    println!(
        "{name:28} {n:>8} iters  {:>9.2} µs/iter  ({:>9.0} ops/s)",
        dur.as_micros() as f64 / n as f64,
        n as f64 / dur.as_secs_f64()
    );
    dur
}

/// 构造一张 w×h 的 RGBA PNG（用于解码基准）。
fn make_png(w: u32, h: u32) -> Vec<u8> {
    use std::io::Write;
    let mut scan = Vec::new();
    for _ in 0..h {
        scan.extend_from_slice(&[0u8]); // filter None
        for _ in 0..w {
            scan.extend_from_slice(&[200, 100, 50, 255]);
        }
    }
    let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    enc.write_all(&scan).unwrap();
    let idat = enc.finish().unwrap();
    let mut png = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    let mut chunk = |t: &[u8; 4], d: &[u8]| {
        png.extend_from_slice(&(d.len() as u32).to_be_bytes());
        png.extend_from_slice(t);
        png.extend_from_slice(d);
        png.extend_from_slice(&[0u8; 4]); // crc（解码端不校验）
    };
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // 8bit RGBA 非交织
    chunk(b"IHDR", &ihdr);
    chunk(b"IDAT", &idat);
    chunk(b"IEND", &[]);
    png
}

fn main() {
    // ── mock 引擎：内存参考实现的热路径 ────────────────────────
    let engine = fastbrowser::engines::mock::MockEngine::new();
    let tab = engine
        .create_tab("https://example.com/login", &Default::default())
        .unwrap();

    bench("snapshot (login page)", 2000, || {
        let _ = engine.snapshot(tab).unwrap();
    });

    bench("evaluate_js", 2000, || {
        let _ = engine
            .evaluate(tab, "document.querySelectorAll('a').length")
            .unwrap();
    });

    bench("screenshot 800x600", 500, || {
        let _ = engine.screenshot(tab).unwrap();
    });

    // ── SDK 层：工具分发 / 清单 / 会话 ─────────────────────────
    let sdk = fastbrowser::Fastbrowser::new();
    sdk.init(fastbrowser::Config::default()).unwrap();
    sdk.open("https://example.com/login").unwrap();

    bench("sdk snapshot()", 2000, || {
        let _ = sdk.snapshot().unwrap();
    });

    bench("tool_call(get_page_title)", 5000, || {
        let _ = sdk.tool_call("get_page_title", json!({})).unwrap();
    });

    bench("tool_call(click by role ref)", 5000, || {
        let _ = sdk
            .tool_call("click", json!({"ref": {"kind": "role", "value": "button"}}))
            .unwrap();
    });

    bench("tool_list (JSON Schema generation)", 200, || {
        let _ = sdk.tool_list();
    });

    let dir = std::env::temp_dir().join("fb_bench_state.json");
    let path = dir.to_str().unwrap().to_string();
    sdk.session_save(&path).unwrap();
    bench("session_save", 200, || {
        let _ = sdk.session_save(&path).unwrap();
    });
    bench("session_load", 200, || {
        let _ = sdk.session_load(&path).unwrap();
    });
    let _ = std::fs::remove_file(&path);

    // ── PNG 解码（自研解码器，真实截图热路径）──────────────────
    let png = make_png(128, 128);
    let png_bytes = png.len();
    bench(&format!("png_decode 128x128 ({png_bytes}B)"), 500, || {
        let (w, h, rgba) = fastbrowser::png::decode_png(&png).unwrap();
        assert_eq!(rgba.len(), (w * h * 4) as usize);
    });

    // ── C ABI：跨 FFI 调用开销 ────────────────────────────────
    unsafe {
        let cfg = std::ffi::CString::new(r#"{"engine":"mock"}"#).unwrap();
        let p = crate_cffi("fastbrowser_init", Some(cfg.as_ptr()), None);
        let _ = std::ffi::CString::from_raw(p);
        let url = std::ffi::CString::new("https://example.com/login").unwrap();
        let p = crate_cffi("fastbrowser_open", Some(url.as_ptr()), None);
        let _ = std::ffi::CString::from_raw(p);

        let name = std::ffi::CString::new("get_page_title").unwrap();
        let params = std::ffi::CString::new("{}").unwrap();
        bench("c_abi tool_call", 2000, || {
            let p = crate_cffi(
                "fastbrowser_tool_call",
                Some(name.as_ptr()),
                Some(params.as_ptr()),
            );
            let _ = std::ffi::CString::from_raw(p);
        });
    }

    // ── 真实 Chromium 延迟（feature engine-cdp）────────────────
    #[cfg(feature = "engine-cdp")]
    chromium_benches();
}

unsafe fn crate_cffi(
    symbol: &str,
    a: Option<*const std::ffi::c_char>,
    b: Option<*const std::ffi::c_char>,
) -> *mut std::ffi::c_char {
    unsafe extern "C" {
        fn fastbrowser_init(config_json: *const std::ffi::c_char) -> *mut std::ffi::c_char;
        fn fastbrowser_open(url: *const std::ffi::c_char) -> *mut std::ffi::c_char;
        fn fastbrowser_tool_call(
            name: *const std::ffi::c_char,
            params_json: *const std::ffi::c_char,
        ) -> *mut std::ffi::c_char;
    }
    unsafe {
        match symbol {
            "fastbrowser_init" => fastbrowser_init(a.unwrap()),
            "fastbrowser_open" => fastbrowser_open(a.unwrap()),
            "fastbrowser_tool_call" => fastbrowser_tool_call(a.unwrap(), b.unwrap()),
            _ => unreachable!(),
        }
    }
}

/// 真实 Chromium 的端到端延迟（快照/导航/点击）。
#[cfg(feature = "engine-cdp")]
fn chromium_benches() {
    use fastbrowser::engine::BrowserEngine;
    use fastbrowser::engines::cdp::ChromiumCdpEngine;
    use fastbrowser::Config;

    let Some(chrome) = fastbrowser::engines::bundled::find_bundled_binary().or_else(|| {
        [
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "/usr/bin/google-chrome",
            "/usr/bin/chromium",
        ]
        .iter()
        .map(std::path::PathBuf::from)
        .find(|p| p.exists())
    }) else {
        println!("chromium benchmark skipped (no browser found; set CHROME_PATH)");
        return;
    };

    let cfg = Config {
        engine: "bundled".into(),
        ..Config::default()
    };
    let Ok(engine) = (if cfg.engine == "bundled" {
        fastbrowser::engines::bundled::BundledChromium::connect_engine(chrome.clone(), &cfg)
    } else {
        ChromiumCdpEngine::connect("ws://unused", &cfg, 3000)
    }) else {
        println!("chromium failed to launch, skipping");
        return;
    };
    let _ = chrome;
    let engine = engine;

    // 一个内含可交互元素的真实页面（data URL 免服务器）
    let url = "data:text/html,<html><body><h1>bench</h1><button id='b' onclick='this.dataset.c=1'>go</button>\
               <input id='i'><a href='#x'>link</a><select><option>a</option><option>b</option></select>\
               <script>document.body.style.height='1200px'</script></body></html>";
    let tab = engine.create_tab(url, &Default::default()).unwrap();

    bench("chromium snapshot", 30, || {
        let _ = engine.snapshot(tab).unwrap();
    });

    bench("chromium get_page_text", 30, || {
        let _ = engine.get_page_text(tab).unwrap();
    });

    bench("chromium click (coordinate)", 30, || {
        engine
            .click_element(tab, &fastbrowser::engine::ElementRef::snapshot('b'))
            .unwrap();
    });

    bench("chromium screenshot (PNG decode)", 10, || {
        let _ = engine.screenshot(tab).unwrap();
    });

    bench("chromium navigate (about:blank)", 10, || {
        engine.navigate(tab, "about:blank").unwrap();
    });
}
