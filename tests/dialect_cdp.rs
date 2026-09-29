// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Element-target dialect → **真 Chromium（CDP 引擎）** 端到端验证。
//!
//! `dialect_contract.rs` 只验证 `resolve_target_from` 的解析结果；本文件在真实
//! 浏览器里用各种方言（selector / ref 字符串 / id / element / text= / role= /
//! xpath / //）去定位并点击，断言命中同一个元素。
//!
//! 前置：本机有 Chrome/Chromium（或 `CHROME_PATH`）。未找到时 SKIP。

#![cfg(all(feature = "engine-cdp", feature = "heavy-tests"))]

mod common;

use fastbrowser::sdk::Fastbrowser;
use fastbrowser::Config;
use serde_json::{json, Value};

const HTML: &str = r##"<!doctype html><html><head><title>Dialect</title></head><body>
<button id="go" onclick="document.getElementById('out').textContent='clicked'">Go</button>
<input id="q" placeholder="search">
<a id="lnk" href="#x">Next</a>
<span id="out">initial</span>
</body></html>"##;

fn setup() -> Option<(Fastbrowser, common::BrowserGuard, String)> {
    setup_with(HTML)
}

fn setup_with(html: &str) -> Option<(Fastbrowser, common::BrowserGuard, String)> {
    let (s, guard) = sdk_only()?;
    let dir = std::env::temp_dir().join(format!("fb_dialect_{}", std::process::id()));
    std::fs::create_dir_all(&dir).ok()?;
    let file = dir.join("dialect.html");
    std::fs::write(&file, html).ok()?;
    let url = common::file_url(&file);
    Some((s, guard, url))
}

fn sdk_only() -> Option<(Fastbrowser, common::BrowserGuard)> {
    let b = common::shared_browser(true).ok()?;
    let guard = common::browser_guard(b);
    let ws = b.ws.clone();
    let s = Fastbrowser::new();
    s.init(Config {
        engine: "chromium".into(),
        cdp_url: Some(ws),
        ..Config::default()
    })
    .ok()?;
    Some((s, guard))
}

/// Minimal HTTP server (same-origin pages) — `file://` iframes are treated as
/// cross-origin by Chrome, so nested-frame tests need HTTP.
fn serve(routes: Vec<(&'static str, &'static str)>) -> (std::net::TcpListener, String) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = listener.try_clone().unwrap();
    std::thread::spawn(move || {
        for stream in server.incoming() {
            let Ok(mut stream) = stream else { break };
            let mut buf = [0u8; 2048];
            let _ = std::io::Read::read(&mut stream, &mut buf);
            let req = String::from_utf8_lossy(&buf);
            let raw = req.split_whitespace().nth(1).unwrap_or("/");
            let path = raw.split('?').next().unwrap_or(raw);
            let body = routes
                .iter()
                .find(|(p, _)| *p == path)
                .map(|(_, b)| *b)
                .unwrap_or("<html><body>404</body></html>");
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = std::io::Write::write_all(&mut stream, resp.as_bytes());
        }
    });
    (listener, format!("http://127.0.0.1:{port}/"))
}

/// All these dialects must resolve to the SAME button element.
#[test]
fn cdp_element_target_dialects_resolve() {
    let Some((s, _guard, url)) = setup() else {
        eprintln!("SKIP: no chrome");
        return;
    };
    s.open(&url).expect("open page");

    let targets: &[&str] = &[
        r##"{"selector":"#go"}"##,
        r#"{"selector":"button"}"#,
        r##"{"ref":"#go"}"##,
        r#"{"id":"go"}"#,
        r#"{"selector":"text=Go"}"#,
        r#"{"selector":"role=button"}"#,
        r#"{"selector":"xpath=//button"}"#,
        r#"{"selector":"//button"}"#,
        r#"{"element":"Go"}"#,
    ];
    for params in targets {
        let p: Value = serde_json::from_str(params).unwrap();
        let v = s
            .tool_call("get_element_info", p)
            .unwrap_or_else(|e| panic!("{params}: {e}"));
        assert_eq!(v["tag"], json!("button"), "params {params} → {v}");
        assert!(
            v["text"].as_str().unwrap_or("").contains("Go"),
            "params {params} → {v}"
        );
    }
}

/// Each dialect can actually CLICK the button (onclick flips #out).
#[test]
fn cdp_click_by_dialect() {
    let Some((s, _guard, url)) = setup() else {
        eprintln!("SKIP: no chrome");
        return;
    };
    s.open(&url).expect("open page");

    let targets: &[&str] = &[
        r##"{"selector":"#go"}"##,
        r#"{"selector":"text=Go"}"#,
        r#"{"selector":"role=button"}"#,
        r#"{"selector":"//button"}"#,
        r##"{"ref":"#go"}"##,
    ];
    for target in targets {
        // Reset the marker.
        let _ = s.tool_call(
            "execute_js",
            json!({"script": "document.getElementById('out').textContent='initial'"}),
        );
        let p: Value = serde_json::from_str(target).unwrap();
        s.tool_call("click", p)
            .unwrap_or_else(|e| panic!("click {target}: {e}"));
        let v = s
            .tool_call(
                "execute_js",
                json!({"script": "document.getElementById('out').textContent"}),
            )
            .unwrap();
        assert_eq!(v["result"], json!("clicked"), "click target {target} → {v}");
    }
}

const NESTED_HTML: &str = r##"<!doctype html><html><head><title>Nested</title></head><body>
<div id="host"></div>
<iframe id="fr" src="inner.html"></iframe>
<script>
  var sr = document.getElementById('host').attachShadow({mode:'open'});
  sr.innerHTML = "<button id='sbtn' onclick='window.__shadowClicked=true'>ShadowBtn</button>";
</script>
</body></html>"##;

const INNER_HTML: &str = r##"<!doctype html><html><body>
<button id="ibtn" onclick="parent.__iframeClicked=true">IframeBtn</button>
</body></html>"##;

/// Snapshot refs and clicks must work for elements inside an open shadow root
/// and a same-origin iframe (the action path uses `window.__fbFind`, which
/// pierces shadow roots and iframes).
#[test]
fn cdp_shadow_and_iframe_targets() {
    let (_srv, base) = serve(vec![("/", NESTED_HTML), ("/inner.html", INNER_HTML)]);
    let Some((s, _guard)) = sdk_only() else {
        eprintln!("SKIP: no chrome");
        return;
    };
    s.open(&base).expect("open page");
    std::thread::sleep(std::time::Duration::from_millis(500));

    let snap = s.snapshot().expect("snapshot");
    let texts: Vec<Option<String>> = snap.interactive.iter().map(|e| e.text.clone()).collect();
    let shadow = snap
        .interactive
        .iter()
        .find(|e| e.text.as_deref() == Some("ShadowBtn"))
        .unwrap_or_else(|| panic!("shadow button not in snapshot: {texts:?}"));
    let iframe = snap
        .frames
        .iter()
        .flat_map(|f| f.interactive.iter())
        .find(|e| e.text.as_deref() == Some("IframeBtn"))
        .unwrap_or_else(|| panic!("iframe button not in snapshot frames: {texts:?}"));

    s.tool_call("click", json!({"id": shadow.id.to_string()}))
        .expect("click shadow button");
    let v = s
        .tool_call(
            "execute_js",
            json!({"script": "window.__shadowClicked === true"}),
        )
        .unwrap();
    assert_eq!(v["result"], json!(true), "shadow click: {v}");

    s.tool_call("click", json!({"id": iframe.id.to_string()}))
        .expect("click iframe button");
    let v = s
        .tool_call(
            "execute_js",
            json!({"script": "window.__iframeClicked === true"}),
        )
        .unwrap();
    assert_eq!(v["result"], json!(true), "iframe click: {v}");
}

const FORM_HTML: &str = r##"<!doctype html><html><head><title>Form</title></head><body>
<input id="q" value="">
<select id="sel"><option value="a">A</option><option value="b">B</option></select>
</body></html>"##;

/// `type` / `select_option` must accept CSS dialect targets on the real engine
/// (they share the live element-resolution path).
#[test]
fn cdp_type_and_select_by_selector() {
    let Some((s, _guard, url)) = setup_with(FORM_HTML) else {
        eprintln!("SKIP: no chrome");
        return;
    };
    s.open(&url).expect("open page");

    s.tool_call("type", json!({"selector": "#q", "text": "hello"}))
        .expect("type by CSS");
    let v = s
        .tool_call(
            "execute_js",
            json!({"script": "document.getElementById('q').value"}),
        )
        .unwrap();
    assert_eq!(v["result"], json!("hello"), "type result: {v}");

    s.tool_call("select_option", json!({"selector": "#sel", "value": "b"}))
        .expect("select by CSS");
    let v = s
        .tool_call(
            "execute_js",
            json!({"script": "document.getElementById('sel').value"}),
        )
        .unwrap();
    assert_eq!(v["result"], json!("b"), "select result: {v}");
}
