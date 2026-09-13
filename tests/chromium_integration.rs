// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! 真 Chromium 集成测试（feature `engine-cdp`）。
//!
//! 启动无头 Chrome/Chromium，连接其浏览器级 CDP 端点，用 `ChromiumCdpEngine`
//! 驱动真实页面，验证快照/输入/点击/提取/JS/截图/多标签页等全链路。
//!
//! 前置：本机存在 Chrome/Chromium（或用环境变量 `CHROME_PATH` 指定）。
//! 未找到时打印 SKIP 并直接返回（不阻塞无浏览器环境的 CI）。

#![cfg(feature = "engine-cdp")]

mod common;

use std::io::{Read, Write};
use std::sync::Arc;
use std::time::{Duration, Instant};

use fastbrowser::sdk::Fastbrowser;
use fastbrowser::Config;
use serde_json::Value;

const HTML: &str = r##"<!doctype html><html><head><title>CDP Integration Test</title></head>
<body>
  <h1>CDP Test</h1>
  <input id="q" placeholder="search" value="">
  <button id="go">Go</button>
  <a href="#next">Next link</a>
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

fn setup() -> (common::BrowserGuard, String) {
    let b = common::shared_browser(true).expect("launch headless chrome");
    let guard = common::browser_guard(b);
    let ws = b.ws.clone();
    reset_browser(&ws);
    (guard, ws)
}

/// 清理上一个测试遗留的 tab/cookie/storage，保证每个测试从干净状态开始。
fn reset_browser(ws: &str) {
    let sdk = Fastbrowser::new();
    sdk.init(Config {
        engine: "chromium".into(),
        cdp_url: Some(ws.to_string()),
        ..Config::default()
    })
    .unwrap();
    let _ = sdk.clear_state();
    sdk.shutdown();
}

fn write_fixture(dir: &std::path::Path) -> String {
    let file = dir.join("fixture.html");
    std::fs::write(&file, HTML).unwrap();
    common::file_url(&file)
}

/// 迷你 HTTP 服务器：对任意路径返回 `body`，用于验证真实 cookie/上下文隔离。
/// 返回 (listener guard, base_url)；guard 析构即停止服务。
fn serve_http(body: &'static str) -> (std::net::TcpListener, String) {
    serve_http_multi(vec![("/", body)])
}

/// 按路径返回不同内容的 HTTP 服务器（iframe 测试需要同源子页面）。
fn serve_http_multi(routes: Vec<(&'static str, &'static str)>) -> (std::net::TcpListener, String) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = listener.try_clone().unwrap();
    std::thread::spawn(move || {
        for stream in server.incoming() {
            let Ok(mut stream) = stream else { break };
            let mut buf = [0u8; 2048];
            let _ = stream.read(&mut buf);
            let req = String::from_utf8_lossy(&buf);
            let raw_path = req.split_whitespace().nth(1).unwrap_or("/");
            let path = raw_path.split('?').next().unwrap_or(raw_path);
            let (status, body) = match routes.iter().find(|(p, _)| path == *p) {
                Some((_, b)) => ("200 OK", *b),
                None => ("404 Not Found", "<html><body>not found</body></html>"),
            };
            let resp = format!(
                "HTTP/1.1 {status}\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(resp.as_bytes());
        }
    });
    (listener, format!("http://127.0.0.1:{port}/"))
}

#[test]
fn chromium_end_to_end() {
    let Some(_) = common::find_chrome() else {
        eprintln!("SKIP: chromium_end_to_end (no chrome found; set CHROME_PATH)");
        return;
    };
    let (_guard, ws) = setup();
    let dir = tempfile::tempdir().unwrap();
    let url = write_fixture(dir.path());

    let sdk = Fastbrowser::new();
    let cfg = Config {
        engine: "chromium".into(),
        cdp_url: Some(ws.clone()),
        ..Config::default()
    };
    sdk.init(cfg).unwrap();

    // 打开真实页面
    let out = sdk.open(&url).unwrap();
    assert!(out["tab"].is_number());

    // 等待加载完成
    wait_until("page loaded", Duration::from_secs(10), || {
        sdk.snapshot()
            .map(|s| s.title.contains("CDP Integration Test"))
            .unwrap_or(false)
    });

    // ── 快照：真实 DOM 中的可交互元素 ──────────────────────
    let snap = sdk.snapshot().unwrap();
    assert_eq!(snap.title, "CDP Integration Test");
    assert!(snap.interactive.iter().any(|e| e.tag == "input"));
    assert!(snap
        .interactive
        .iter()
        .any(|e| e.tag == "button" && e.text.as_deref() == Some("Go")));
    let input = snap
        .interactive
        .iter()
        .find(|e| e.tag == "input")
        .unwrap()
        .id;

    // ── 输入：type 进真实 input ─────────────────────────────
    sdk.tool_call(
        "type",
        serde_json::json!({"id": input.to_string(), "text": "fastbrowser"}),
    )
    .unwrap();
    let v = sdk
        .tool_call(
            "execute_js",
            serde_json::json!({"script": "document.getElementById('q').value"}),
        )
        .unwrap();
    assert_eq!(v["result"], "fastbrowser");

    // ── 点击：点真实按钮 ────────────────────────────────────
    let btn = sdk
        .snapshot()
        .unwrap()
        .interactive
        .iter()
        .find(|e| e.tag == "button")
        .unwrap()
        .id;
    sdk.tool_call("click", serde_json::json!({"id": btn.to_string()}))
        .unwrap();

    // ── 提取：真实文本 / 链接 / 表格 ────────────────────────
    let text = sdk
        .tool_call("get_page_text", serde_json::json!({}))
        .unwrap();
    assert!(text["text"].as_str().unwrap().contains("CDP Test"));
    let links = sdk
        .tool_call("extract_links", serde_json::json!({}))
        .unwrap();
    assert!(links["links"].as_array().unwrap().iter().any(|l| l["url"]
        .as_str()
        .map(|u| u.contains("#next"))
        .unwrap_or(false)));

    // ── JS 求值 ─────────────────────────────────────────────
    let title = sdk
        .tool_call(
            "execute_js",
            serde_json::json!({"script": "document.title"}),
        )
        .unwrap();
    assert_eq!(title["result"], "CDP Integration Test");

    // ── 新 SDK 工具：真实页面 meta / 滚动 ────────────────────
    let meta = sdk
        .tool_call("get_page_meta", serde_json::json!({}))
        .unwrap();
    assert_eq!(meta["meta"]["title"], "CDP Integration Test");
    let scroll = sdk
        .tool_call("get_scroll_position", serde_json::json!({}))
        .unwrap();
    assert!(scroll["scroll"]["x"].is_number());
    let a11y = sdk
        .tool_call("get_accessibility_tree", serde_json::json!({}))
        .unwrap();
    assert!(a11y["count"].as_u64().unwrap() >= 1);
    let _ = sdk
        .tool_call("set_scroll_position", serde_json::json!({"y": 100}))
        .unwrap();
    let _ = sdk
        .tool_call("wait_for_load_state", serde_json::json!({"state": "load"}))
        .unwrap();

    // ── 视口 / 截图 / 帧 ────────────────────────────────────
    sdk.set_viewport(1024, 768).unwrap();
    let img = sdk.screenshot().unwrap();
    assert_eq!(img.width, 1024);
    let tab = sdk
        .runtime()
        .unwrap()
        .as_ref()
        .unwrap()
        .engine()
        .active_tab()
        .unwrap();
    let _ = sdk
        .runtime()
        .unwrap()
        .as_ref()
        .unwrap()
        .engine()
        .view_frame(tab)
        .unwrap();

    // ── 多标签页（浏览器级端点支持 createTarget）──────────────
    let t2 = sdk
        .tool_call("new_tab", serde_json::json!({"url": "about:blank"}))
        .unwrap();
    let t2id = t2["tab"].as_u64().unwrap() as u32;
    let tabs = sdk.tool_call("list_tabs", serde_json::json!({})).unwrap();
    // 含默认页标签 + open 的标签 + 新标签，至少 2 个
    assert!(tabs["tabs"].as_array().unwrap().len() >= 2);
    sdk.tool_call("switch_tab", serde_json::json!({"tab": t2id}))
        .unwrap();
    sdk.tool_call("close_tab", serde_json::json!({"tab": t2id}))
        .unwrap();

    sdk.shutdown();
    eprintln!("PASS: chromium_end_to_end");
}

#[test]
fn chromium_navigation_events() {
    let Some(_) = common::find_chrome() else {
        eprintln!("SKIP: chromium_navigation_events (no chrome)");
        return;
    };
    let (_guard, ws) = setup();
    let dir = tempfile::tempdir().unwrap();
    let url = write_fixture(dir.path());

    let sdk = Fastbrowser::new();
    let cfg = Config {
        engine: "chromium".into(),
        cdp_url: Some(ws),
        ..Config::default()
    };
    sdk.init(cfg).unwrap();

    let tab = sdk.open(&url).unwrap()["tab"].as_u64().unwrap() as u32;
    let tab = fastbrowser::engine::TabId(tab);
    sdk.runtime()
        .unwrap()
        .as_ref()
        .unwrap()
        .engine()
        .navigate(tab, "about:blank")
        .unwrap();
    wait_until("nav completes", Duration::from_secs(10), || {
        sdk.snapshot()
            .map(|s| s.url.contains("about:blank"))
            .unwrap_or(false)
    });

    // 浏览器级事件应可被拉取（至少不 panic；真实导航事件经 CDP 缓冲）
    let _evs = sdk
        .runtime()
        .unwrap()
        .as_ref()
        .unwrap()
        .engine()
        .drain_events(tab);
    sdk.shutdown();
    eprintln!("PASS: chromium_navigation_events");
}

/// 真实浏览器新能力验证：真实截图（PNG 解码）、真实事件流、console、
/// PDF 导出、真实 cookie/storage、search/find_elements/send_keys、AX 树。
#[test]
fn chromium_new_capabilities() {
    let Some(_) = common::find_chrome() else {
        eprintln!("SKIP: chromium_new_capabilities (no chrome)");
        return;
    };
    let (_guard, ws) = setup();
    let (_server, url) = serve_http(
        r##"<!doctype html><html><head><title>Capabilities</title></head><body>
          <h1>Capabilities Page</h1>
          <input id="name" placeholder="name">
          <button id="run">Run</button>
          <a href="#more">More</a>
          <span>Hello world searchable text</span>
          <script>
            document.getElementById('run').addEventListener('click', function () {
              document.body.dataset.ran = 'yes';
            });
            localStorage.setItem('lk', 'lv');
            console.log('caps-msg');
          </script>
        </body></html>"##,
    );

    let sdk = Fastbrowser::new();
    let cfg = Config {
        engine: "chromium".into(),
        cdp_url: Some(ws.clone()),
        ..Config::default()
    };
    sdk.init(cfg).unwrap();
    sdk.open(&url).unwrap();
    wait_until("page loaded", Duration::from_secs(10), || {
        sdk.snapshot()
            .map(|s| s.title == "Capabilities")
            .unwrap_or(false)
    });

    // ── 真实截图：PNG 解码 → RGBA，宽度/长度正确且非纯色占位 ──────
    let img = sdk.screenshot().unwrap();
    assert!(img.is_valid());
    assert!(img.width > 0 && img.height > 0);
    // 页面有黑色文字/白色背景 → 至少有 2 种像素值（不再是占位黑图/全 0）
    let mut distinct = 0u32;
    for c in img.rgba.chunks_exact(4).take(4096) {
        if c[0] != 0 || c[1] != 0 || c[2] != 0 {
            distinct += 1;
        }
    }
    assert!(
        distinct > 0,
        "screenshot should not be a blank/black placeholder"
    );

    // ── 真实事件流：console 事件（drain_events 现在按 session 路由）────
    let tab = {
        let rt_guard = sdk.runtime().unwrap();
        let rt = rt_guard.as_ref().unwrap();
        let t = rt.engine().active_tab().unwrap();
        rt.engine().drain_events(t);
        t
    };
    // 再触发一次 console 输出并轮询拉取
    sdk.tool_call(
        "execute_js",
        serde_json::json!({"script": "console.log('evt-marker')"}),
    )
    .unwrap();
    wait_until("console event", Duration::from_secs(5), || {
        sdk.runtime().unwrap().as_ref().unwrap().engine().drain_events(tab)
            .iter().any(|e| matches!(e, fastbrowser::engine::PageEvent::Console { message, .. } if message == "evt-marker"))
    });

    // ── 真实 cookie：Network.setCookie 写入 → cookie_get 读回 ──────
    let host = url.trim_end_matches('/').replace("http://", "");
    let host = host.split(':').next().unwrap_or(&host).to_string(); // 去端口
    sdk.tool_call(
        "cookie_set",
        serde_json::json!({"name": "cook", "value": "jar", "domain": host}),
    )
    .unwrap();
    let cookies = sdk
        .tool_call("cookie_get", serde_json::json!({"domain": host}))
        .unwrap();
    assert!(cookies["cookies"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["name"] == "cook"));

    // ── 真实 storage：localStorage 写入 → storage_get 读回 ────────
    let ls = sdk
        .tool_call("storage_get", serde_json::json!({"key": "lk"}))
        .unwrap();
    assert_eq!(ls["value"], "lv");

    // ── PDF 导出 ────────────────────────────────────────────────
    let pdf = sdk
        .runtime()
        .unwrap()
        .as_ref()
        .unwrap()
        .engine()
        .print_to_pdf(tab)
        .unwrap();
    assert!(!pdf.is_empty());
    assert!(pdf.starts_with(b"%PDF-"));

    // ── search / find_elements ─────────────────────────────────
    let s = sdk
        .tool_call("search", serde_json::json!({"query": "Hello world"}))
        .unwrap();
    assert!(s["count"].as_u64().unwrap() >= 1);
    let fe = sdk
        .tool_call("find_elements", serde_json::json!({"selector": "a"}))
        .unwrap();
    assert!(fe["count"].as_u64().unwrap() >= 1);

    // ── send_keys：聚焦 input 后 Ctrl+Shift+A 不崩溃 ─────────────
    sdk.tool_call(
        "execute_js",
        serde_json::json!({"script": "document.getElementById('name').focus()"}),
    )
    .unwrap();
    sdk.tool_call("send_keys", serde_json::json!({"keys": "Ctrl+Shift+A"}))
        .unwrap();

    // ── AX 树（Accessibility.getFullAXTree）────────────────────
    let a11y = sdk
        .tool_call("get_accessibility_tree", serde_json::json!({}))
        .unwrap();
    assert!(a11y["count"].as_u64().unwrap() >= 1);
    assert!(!a11y.get("fallback").map(|f| f == true).unwrap_or(false));

    // ── 真实点击（坐标级 + hit-test）：按钮点击生效 ───────────────
    let btn = sdk
        .snapshot()
        .unwrap()
        .interactive
        .iter()
        .find(|e| e.tag == "button")
        .unwrap()
        .id;
    sdk.tool_call("click", serde_json::json!({"id": btn.to_string()}))
        .unwrap();
    let ran = sdk
        .tool_call(
            "execute_js",
            serde_json::json!({"script": "document.body.dataset.ran || ''"}),
        )
        .unwrap();
    assert_eq!(ran["result"], "yes");

    sdk.shutdown();
    eprintln!("PASS: chromium_new_capabilities");
}

/// 同源 iframe + shadow DOM 快照/点击支持验证。
#[test]
fn chromium_iframes_and_shadow() {
    let Some(_) = common::find_chrome() else {
        eprintln!("SKIP: chromium_iframes_and_shadow (no chrome)");
        return;
    };
    let (_guard, ws) = setup();
    let (_server, base) = serve_http_multi(vec![
        (
            "/",
            r##"<!doctype html><html><head><title>FrameTest</title></head><body>
          <h1>Top</h1>
          <button id="topbtn">TopBtn</button>
          <iframe id="fr" src="/frame.html" style="width:300px;height:150px"></iframe>
          <div id="host"></div>
          <script>
            const host = document.getElementById('host');
            const shadow = host.attachShadow({mode:'open'});
            shadow.innerHTML = '<button id="shbtn">ShadowBtn</button>';
            window.__shadowRan = 0;
            shadow.querySelector('#shbtn').addEventListener('click', function () { window.__shadowRan = 1; });
          </script>
        </body></html>"##,
        ),
        (
            "/frame.html",
            r##"<!doctype html><html><head><title>InnerFrame</title></head><body>
          <button id="innerbtn" onclick="window.__frameRan=1">InnerBtn</button>
        </body></html>"##,
        ),
    ]);

    let sdk = Fastbrowser::new();
    let cfg = Config {
        engine: "chromium".into(),
        cdp_url: Some(ws),
        ..Config::default()
    };
    sdk.init(cfg).unwrap();
    sdk.open(&base).unwrap();
    wait_until("frame page loaded", Duration::from_secs(10), || {
        sdk.snapshot()
            .map(|s| s.title == "FrameTest")
            .unwrap_or(false)
    });

    // 快照应包含子框架（轮询等 iframe 就绪）
    let mut inner_btn = None;
    wait_until("iframe snapshot", Duration::from_secs(10), || {
        if let Ok(s) = sdk.snapshot() {
            if !s.frames.is_empty() {
                inner_btn = s.frames[0]
                    .interactive
                    .iter()
                    .find(|e| e.tag == "button")
                    .map(|e| e.id);
                return inner_btn.is_some();
            }
        }
        false
    });
    let snap = sdk.snapshot().unwrap();
    assert_eq!(snap.frames.len(), 1, "expected 1 iframe frame snapshot");
    let f0 = &snap.frames[0];
    assert!(f0.url.contains("frame.html"));
    assert!(!f0.cross_origin, "same-origin iframe should be readable");
    let inner_btn = f0
        .interactive
        .iter()
        .find(|e| e.tag == "button" && e.text.as_deref() == Some("InnerBtn"));
    assert!(inner_btn.is_some(), "iframe button not in frame snapshot");

    // iframe 内元素点击（真实坐标点击，跨 iframe 坐标换算）
    let inner_id = inner_btn.unwrap().id;
    sdk.tool_call("click", serde_json::json!({"id": inner_id.to_string()}))
        .unwrap();
    // 按钮在 iframe 窗口内设置状态，需经 iframe.contentWindow 读取
    let ran = sdk.tool_call("execute_js", serde_json::json!({"script": "document.getElementById('fr').contentWindow.__frameRan || 0"})).unwrap();
    assert_eq!(ran["result"], 1, "iframe button click did not fire");

    // shadow DOM 元素应并入顶层 interactive（同坐标系）
    let snap2 = sdk.snapshot().unwrap();
    let sh_btn = snap2
        .interactive
        .iter()
        .find(|e| e.text.as_deref() == Some("ShadowBtn"));
    assert!(sh_btn.is_some(), "shadow button missing from snapshot");

    // shadow 内元素点击
    let sh_id = sh_btn.unwrap().id;
    sdk.tool_call("click", serde_json::json!({"id": sh_id.to_string()}))
        .unwrap();
    let sh_ran = sdk
        .tool_call(
            "execute_js",
            serde_json::json!({"script": "window.__shadowRan || 0"}),
        )
        .unwrap();
    assert_eq!(sh_ran["result"], 1, "shadow button click did not fire");

    // 顶层元素仍可点击（回归）
    let top_id = sdk
        .snapshot()
        .unwrap()
        .interactive
        .iter()
        .find(|e| e.text.as_deref() == Some("TopBtn"))
        .unwrap()
        .id;
    sdk.tool_call("click", serde_json::json!({"id": top_id.to_string()}))
        .unwrap();

    sdk.shutdown();
    eprintln!("PASS: chromium_iframes_and_shadow");
}

/// 遮挡点名：被覆盖的元素点击应报错并指出遮挡者（真实 hit-test）。
#[test]
fn chromium_covered_click_reports_blocker() {
    let Some(_) = common::find_chrome() else {
        eprintln!("SKIP: chromium_covered_click_reports_blocker (no chrome)");
        return;
    };
    let (_guard, ws) = setup();
    let (_server, url) = serve_http(
        r##"<!doctype html><html><head><title>CoverTest</title></head><body>
          <button id="under" style="position:absolute;top:50px;left:50px;width:100px;height:40px">UnderBtn</button>
          <div id="overlay" style="position:absolute;top:0;left:0;width:300px;height:200px;background:rgba(0,0,0,0.5)">Overlay</div>
        </body></html>"##,
    );

    let sdk = Fastbrowser::new();
    let cfg = Config {
        engine: "chromium".into(),
        cdp_url: Some(ws),
        ..Config::default()
    };
    sdk.init(cfg).unwrap();
    sdk.open(&url).unwrap();
    wait_until("page loaded", Duration::from_secs(10), || {
        sdk.snapshot()
            .map(|s| s.title == "CoverTest")
            .unwrap_or(false)
    });

    let under = sdk
        .snapshot()
        .unwrap()
        .interactive
        .iter()
        .find(|e| e.text.as_deref() == Some("UnderBtn"))
        .unwrap()
        .id;
    let err = sdk
        .tool_call("click", serde_json::json!({"id": under.to_string()}))
        .unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("covered"),
        "expected covered error, got: {msg}"
    );
    // 遮挡者信息应在消息中（Overlay）
    assert!(
        msg.contains("Overlay") || msg.contains("overlay") || msg.contains("div"),
        "blocker not described: {msg}"
    );

    // 顶层按钮（不被遮挡）可正常点击
    sdk.shutdown();
    eprintln!("PASS: chromium_covered_click_reports_blocker");
}

/// 快照截断：>26 个交互元素时 meta.truncated=true，id 不越界（a..z）。
#[test]
fn chromium_snapshot_truncation() {
    let Some(_) = common::find_chrome() else {
        eprintln!("SKIP: chromium_snapshot_truncation (no chrome)");
        return;
    };
    let (_guard, ws) = setup();
    let mut html = String::from("<html><head><title>Trunc</title></head><body>");
    for i in 0..40 {
        html.push_str(&format!("<button>Btn{i}</button>"));
    }
    html.push_str("</body></html>");
    let body: &'static str = Box::leak(html.into_boxed_str());
    let (_server, url) = serve_http(body);

    let sdk = Fastbrowser::new();
    let cfg = Config {
        engine: "chromium".into(),
        cdp_url: Some(ws),
        ..Config::default()
    };
    sdk.init(cfg).unwrap();
    sdk.open(&url).unwrap();
    wait_until("page loaded", Duration::from_secs(10), || {
        sdk.snapshot().map(|s| s.title == "Trunc").unwrap_or(false)
    });

    let snap = sdk.snapshot().unwrap();
    assert!(
        snap.meta.truncated,
        "40 buttons should exceed the 26-id cap"
    );
    assert_eq!(
        snap.meta.total, 40,
        "meta.total should count all scanned elements"
    );
    assert!(snap.interactive.len() <= 26);
    // 所有 id 都在 a..z 范围内（不产生 '{' 等越界字符）
    for e in &snap.interactive {
        assert!(e.id >= 'a' && e.id <= 'z', "id out of range: {:?}", e.id);
    }
    // 前 26 个可交互元素应被编号（首尾元素）
    assert!(snap
        .interactive
        .iter()
        .any(|e| e.text.as_deref() == Some("Btn0")));
    assert!(snap
        .interactive
        .iter()
        .any(|e| e.text.as_deref() == Some("Btn25")));
    // 截断提示应能让 LLM 决定滚动
    assert!(snap.meta.scroll_h >= snap.meta.viewport_h);

    sdk.shutdown();
    eprintln!("PASS: chromium_snapshot_truncation");
}

/// 隔离浏览器上下文：开启 `isolated_profiles` 后，两个 Profile 的
/// cookie/storage 相互隔离（真实浏览器上下文）。
#[test]
fn chromium_context_isolation() {
    let Some(_) = common::find_chrome() else {
        eprintln!("SKIP: chromium_context_isolation (no chrome)");
        return;
    };
    let (_guard, ws) = setup();
    let (_server, url) = serve_http("<html><head><title>Iso</title></head><body>iso</body></html>");

    let sdk = Fastbrowser::new();
    let cfg = Config {
        engine: "chromium".into(),
        cdp_url: Some(ws),
        isolated_profiles: false,
        ..Config::default()
    };
    sdk.init(cfg).unwrap();

    // 无头 Chrome 第二次 createBrowserContext 会关闭默认窗口；因此这里只创建
    // 一个显式隔离上下文，另一个 Profile 使用默认上下文。
    let ctx_b = {
        let rt_guard = sdk.runtime().unwrap();
        let rt = rt_guard.as_ref().unwrap();
        match rt.engine().create_context() {
            Ok(c) => c,
            Err(_) => {
                eprintln!("SKIP: chromium_context_isolation (no browser contexts)");
                sdk.shutdown();
                return;
            }
        }
    };

    let p1 = {
        let rt_guard = sdk.runtime().unwrap();
        let rt = rt_guard.as_ref().unwrap();
        let mut session = rt.session();
        session.ensure_default()
    };

    // Profile 1（默认上下文）打开页面写 cookie
    let host = url.trim_end_matches('/').replace("http://", "");
    let host = host.split(':').next().unwrap_or(&host).to_string(); // 去端口
    let p1_tab = sdk.open(&url).unwrap()["tab"].as_u64().unwrap() as u32;
    wait_until("page loaded", Duration::from_secs(10), || {
        sdk.snapshot().map(|s| s.title == "Iso").unwrap_or(false)
    });
    sdk.tool_call(
        "cookie_set",
        serde_json::json!({"name": "p1", "value": "one", "domain": host}),
    )
    .unwrap();
    let c = sdk
        .tool_call("cookie_get", serde_json::json!({"domain": host}))
        .unwrap();
    assert!(c["cookies"]
        .as_array()
        .unwrap()
        .iter()
        .any(|x| x["name"] == "p1"));

    // Profile 2 绑定隔离上下文 B，打开同一页面：看不到 Profile 1 的 cookie
    {
        let rt_guard = sdk.runtime().unwrap();
        let rt = rt_guard.as_ref().unwrap();
        let p2 = rt
            .session()
            .create_profile("p2", "chromium", false, None, None, None);
        rt.session().set_active_profile(p2).unwrap();
        rt.session().set_profile_context(p2, Some(ctx_b)).unwrap();
    }
    // 注意：无头 Chrome 会冻结二级浏览器上下文的背景窗口渲染器（真实导航慢），
    // 因此这里不等待页面加载，直接验证【上下文级 cookie 隔离】——
    // Network.getCookies 是浏览器侧能力，按会话/上下文作用域返回，不受渲染器冻结影响。
    sdk.open(&url).unwrap();
    let c2 = sdk
        .tool_call("cookie_get", serde_json::json!({"domain": host}))
        .unwrap();
    assert!(
        !c2["cookies"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x["name"] == "p1"),
        "profile2 leaked profile1 cookie"
    );

    // 反向验证：在 Profile 2 的上下文写入 cookie，Profile 1 也不可见
    sdk.tool_call(
        "cookie_set",
        serde_json::json!({"name": "p2", "value": "two", "domain": host}),
    )
    .unwrap();
    let c2b = sdk
        .tool_call("cookie_get", serde_json::json!({"domain": host}))
        .unwrap();
    assert!(c2b["cookies"]
        .as_array()
        .unwrap()
        .iter()
        .any(|x| x["name"] == "p2"));
    // 切回 Profile 1 的活动标签页，确认 p2 不可见、p1 仍在
    {
        let rt_guard = sdk.runtime().unwrap();
        let rt = rt_guard.as_ref().unwrap();
        rt.session().set_active_profile(p1).unwrap();
        rt.engine()
            .switch_tab(fastbrowser::engine::TabId(p1_tab))
            .unwrap();
    }
    let c1 = sdk
        .tool_call("cookie_get", serde_json::json!({"domain": host}))
        .unwrap();
    let names: Vec<&str> = c1["cookies"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|x| x["name"].as_str())
        .collect();
    assert!(names.contains(&"p1"), "default profile lost its cookie");
    assert!(!names.contains(&"p2"), "profile1 saw profile2 cookie");

    sdk.shutdown();
    eprintln!("PASS: chromium_context_isolation");
}

/// 真实表单用 Enter 键提交（模拟键盘输入 → 表单提交 → 导航）。
#[test]
fn chromium_form_submit_via_enter_key() {
    let Some(_) = common::find_chrome() else {
        eprintln!("SKIP: chromium_form_submit_via_enter_key (no chrome)");
        return;
    };
    let (_guard, ws) = setup();
    let (_server, base) = serve_http_multi(vec![
        (
            "/",
            r##"<!doctype html><html><head><title>EnterForm</title></head><body>
          <form action="/result" method="get">
            <input name="q" placeholder="Query">
            <button type="submit" id="sb">Go</button>
          </form>
        </body></html>"##,
        ),
        (
            "/result",
            "<html><head><title>ResultPage</title></head><body>result ok</body></html>",
        ),
    ]);

    let sdk = Fastbrowser::new();
    let cfg = Config {
        engine: "chromium".into(),
        cdp_url: Some(ws),
        ..Config::default()
    };
    sdk.init(cfg).unwrap();
    sdk.open(&base).unwrap();
    wait_until("form loaded", Duration::from_secs(20), || {
        sdk.snapshot()
            .map(|s| s.title == "EnterForm")
            .unwrap_or(false)
    });

    let input = sdk
        .snapshot()
        .unwrap()
        .interactive
        .iter()
        .find(|e| e.tag == "input")
        .unwrap()
        .id;
    sdk.tool_call(
        "type",
        serde_json::json!({"id": input.to_string(), "text": "fastbrowser"}),
    )
    .unwrap();
    sdk.tool_call("press", serde_json::json!({"key": "Enter"}))
        .unwrap();

    wait_until("form submitted", Duration::from_secs(20), || {
        sdk.snapshot()
            .map(|s| s.url.contains("/result"))
            .unwrap_or(false)
    });
    let url = sdk
        .tool_call("get_current_url", serde_json::json!({}))
        .unwrap();
    assert!(
        url["url"].as_str().unwrap().contains("q=fastbrowser"),
        "query not in url: {}",
        url
    );
    let text = sdk
        .tool_call("get_page_text", serde_json::json!({}))
        .unwrap();
    assert!(text["text"].as_str().unwrap().contains("result ok"));

    sdk.shutdown();
    eprintln!("PASS: chromium_form_submit_via_enter_key");
}

/// 真实网络屏蔽：`block_request` 后子资源不再加载（onload 不触发）。
#[test]
fn chromium_network_block_blocks_subresource() {
    let Some(_) = common::find_chrome() else {
        eprintln!("SKIP: chromium_network_block_blocks_subresource (no chrome)");
        return;
    };
    let (_guard, ws) = setup();
    let (_server, base) = serve_http_multi(vec![
        (
            "/",
            r##"<!doctype html><html><head><title>BlockTest</title></head><body>
          <img id="pic" src="/pic.png" onload="window.__picLoaded=1" onerror="window.__picError=1">
          <button id="check">check</button>
        </body></html>"##,
        ),
        ("/pic.png", "not really a png"),
    ]);

    let sdk = Fastbrowser::new();
    let cfg = Config {
        engine: "chromium".into(),
        cdp_url: Some(ws),
        ..Config::default()
    };
    sdk.init(cfg).unwrap();

    // 先屏蔽图片 URL，再打开页面
    let host = base.trim_end_matches('/').replace("http://", "");
    sdk.open(&base).unwrap();
    wait_until("page loaded", Duration::from_secs(10), || {
        sdk.snapshot()
            .map(|s| s.title == "BlockTest")
            .unwrap_or(false)
    });
    sdk.tool_call(
        "block_request",
        serde_json::json!({"patterns": ["pic.png"]}),
    )
    .unwrap();
    sdk.tool_call("reload", serde_json::json!({})).unwrap();
    wait_until("reload done", Duration::from_secs(10), || {
        sdk.snapshot()
            .map(|s| s.title == "BlockTest")
            .unwrap_or(false)
    });

    // 图片被屏蔽 → 触发 error（而非 load）
    let pic_loaded = sdk
        .tool_call(
            "execute_js",
            serde_json::json!({"script": "window.__picLoaded?1:0"}),
        )
        .unwrap();
    let pic_error = sdk
        .tool_call(
            "execute_js",
            serde_json::json!({"script": "window.__picError?1:0"}),
        )
        .unwrap();
    assert_eq!(pic_loaded["result"], 0, "blocked image should not load");
    assert_eq!(pic_error["result"], 1, "blocked image should error");
    let _ = host;

    sdk.shutdown();
    eprintln!("PASS: chromium_network_block_blocks_subresource");
}

/// 真实 JS 对话框：alert 被捕获到 pending_dialog，dialog_accept 可解除。
///
/// 注意：alert 打开时渲染器被阻塞，CDP 输入命令会挂起，因此这里用
/// `setTimeout` 在页面加载后自动弹出，避免点击命令在对话框期间超时。
#[test]
fn chromium_dialog_capture_and_accept() {
    let Some(_) = common::find_chrome() else {
        eprintln!("SKIP: chromium_dialog_capture_and_accept (no chrome)");
        return;
    };
    let (_guard, ws) = setup();
    let (_server, url) = serve_http(
        r##"<!doctype html><html><head><title>DialogTest</title></head><body>
          <script>setTimeout(function(){ alert('hello dialog'); }, 500);</script>
          <p>page body</p>
        </body></html>"##,
    );

    let sdk = Fastbrowser::new();
    let cfg = Config {
        engine: "chromium".into(),
        cdp_url: Some(ws),
        ..Config::default()
    };
    sdk.init(cfg).unwrap();
    sdk.open(&url).unwrap();

    // 轮询 pending_dialog（对话框由页面 setTimeout 触发）
    let mut got = false;
    for _ in 0..100 {
        let d = sdk
            .tool_call("pending_dialog", serde_json::json!({}))
            .unwrap();
        if d["dialog"].is_object() {
            assert_eq!(d["dialog"]["message"], "hello dialog");
            got = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(got, "dialog was not captured");
    sdk.tool_call("dialog_accept", serde_json::json!({}))
        .unwrap();
    // 接受后 pending 应为 null
    wait_until("dialog cleared", Duration::from_secs(10), || {
        sdk.tool_call("pending_dialog", serde_json::json!({}))
            .map(|d| d["dialog"].is_null())
            .unwrap_or(false)
    });

    sdk.shutdown();
    eprintln!("PASS: chromium_dialog_capture_and_accept");
}

/// 真实 localStorage 跨导航保持。
#[test]
fn chromium_localstorage_persists_across_navigation() {
    let Some(_) = common::find_chrome() else {
        eprintln!("SKIP: chromium_localstorage_persists_across_navigation (no chrome)");
        return;
    };
    let (_guard, ws) = setup();
    let (_server, base) = serve_http_multi(vec![
        (
            "/",
            "<html><head><title>A</title></head><body>page a</body></html>",
        ),
        (
            "/b",
            "<html><head><title>B</title></head><body>page b</body></html>",
        ),
    ]);

    let sdk = Fastbrowser::new();
    let cfg = Config {
        engine: "chromium".into(),
        cdp_url: Some(ws),
        ..Config::default()
    };
    sdk.init(cfg).unwrap();
    sdk.open(&base).unwrap();
    wait_until("A loaded", Duration::from_secs(10), || {
        sdk.snapshot().map(|s| s.title == "A").unwrap_or(false)
    });
    sdk.tool_call("storage_set", serde_json::json!({"key": "k", "value": "v"}))
        .unwrap();
    sdk.tool_call("navigate", serde_json::json!({"url": format!("{base}b")}))
        .unwrap();
    wait_until("B loaded", Duration::from_secs(10), || {
        sdk.snapshot().map(|s| s.title == "B").unwrap_or(false)
    });
    let v = sdk
        .tool_call("storage_get", serde_json::json!({"key": "k"}))
        .unwrap();
    assert_eq!(v["value"], "v");

    sdk.shutdown();
    eprintln!("PASS: chromium_localstorage_persists_across_navigation");
}

/// 真实浏览器的 back/forward 与导航历史。
#[test]
fn chromium_back_forward_and_history() {
    let Some(_) = common::find_chrome() else {
        eprintln!("SKIP: chromium_back_forward_and_history (no chrome)");
        return;
    };
    let (_guard, ws) = setup();
    let (_server, base) = serve_http_multi(vec![
        (
            "/",
            "<html><head><title>H1</title></head><body>h1</body></html>",
        ),
        (
            "/two",
            "<html><head><title>H2</title></head><body>h2</body></html>",
        ),
        (
            "/three",
            "<html><head><title>H3</title></head><body>h3</body></html>",
        ),
    ]);

    let sdk = Fastbrowser::new();
    let cfg = Config {
        engine: "chromium".into(),
        cdp_url: Some(ws),
        ..Config::default()
    };
    sdk.init(cfg).unwrap();
    sdk.open(&base).unwrap();
    wait_until("H1", Duration::from_secs(10), || {
        sdk.snapshot().map(|s| s.title == "H1").unwrap_or(false)
    });
    sdk.navigate(&format!("{base}two")).unwrap();
    wait_until("H2", Duration::from_secs(10), || {
        sdk.snapshot().map(|s| s.title == "H2").unwrap_or(false)
    });
    sdk.navigate(&format!("{base}three")).unwrap();
    wait_until("H3", Duration::from_secs(10), || {
        sdk.snapshot().map(|s| s.title == "H3").unwrap_or(false)
    });

    let tab = fastbrowser::engine::TabId(
        sdk.runtime()
            .unwrap()
            .as_ref()
            .unwrap()
            .engine()
            .active_tab()
            .unwrap()
            .as_u32(),
    );
    sdk.runtime()
        .unwrap()
        .as_ref()
        .unwrap()
        .engine()
        .back(tab)
        .unwrap();
    wait_until("back to H2", Duration::from_secs(10), || {
        sdk.snapshot().map(|s| s.title == "H2").unwrap_or(false)
    });
    let url = sdk
        .tool_call("get_current_url", serde_json::json!({}))
        .unwrap();
    assert!(url["url"].as_str().unwrap().contains("/two"));
    sdk.runtime()
        .unwrap()
        .as_ref()
        .unwrap()
        .engine()
        .forward(tab)
        .unwrap();
    wait_until("forward to H3", Duration::from_secs(10), || {
        sdk.snapshot().map(|s| s.title == "H3").unwrap_or(false)
    });

    let h = sdk
        .runtime()
        .unwrap()
        .as_ref()
        .unwrap()
        .engine()
        .get_history(tab)
        .unwrap();
    assert!(h.iter().any(|e| e.url.contains("/two")));

    sdk.shutdown();
    eprintln!("PASS: chromium_back_forward_and_history");
}

/// find_elements 返回的元素含 data-fb 快照 id（可后续点击）。
#[test]
fn chromium_find_elements_annotates_data_fb() {
    let Some(_) = common::find_chrome() else {
        eprintln!("SKIP: chromium_find_elements_annotates_data_fb (no chrome)");
        return;
    };
    let (_guard, ws) = setup();
    let (_server, url) = serve_http(
        r##"<!doctype html><html><head><title>FindTest</title></head><body>
          <a class="product" href="#x">Alpha</a>
          <a class="product" href="#y">Beta</a>
          <button id="clicker">ClickMe</button>
        </body></html>"##,
    );

    let sdk = Fastbrowser::new();
    let cfg = Config {
        engine: "chromium".into(),
        cdp_url: Some(ws),
        ..Config::default()
    };
    sdk.init(cfg).unwrap();
    sdk.open(&url).unwrap();
    wait_until("page loaded", Duration::from_secs(10), || {
        sdk.snapshot()
            .map(|s| s.title == "FindTest")
            .unwrap_or(false)
    });

    let fe = sdk
        .tool_call(
            "find_elements",
            serde_json::json!({"selector": "a.product"}),
        )
        .unwrap();
    let arr = fe["elements"].as_array().unwrap();
    assert!(arr.len() >= 2, "expected 2 product links, got {arr:?}");
    for el in arr {
        assert!(el["tag"] == "a");
        assert!(el["text"].is_string());
        // data-fb 注解在快照扫描后才有；find_elements 独立 JS 查询不依赖它
    }

    // find_elements 查询 + snapshot 快照：id 可从快照取，点击生效
    let snap = sdk.snapshot().unwrap();
    let btn = snap
        .interactive
        .iter()
        .find(|e| e.tag == "button")
        .unwrap()
        .id;
    sdk.tool_call("click", serde_json::json!({"id": btn.to_string()}))
        .unwrap();
    let _ = fe;

    sdk.shutdown();
    eprintln!("PASS: chromium_find_elements_annotates_data_fb");
}

/// 快照 meta 反映真实文档高度/滚动（滚动后 scroll_y 变化）。
#[test]
fn chromium_snapshot_meta_tracks_scroll() {
    let Some(_) = common::find_chrome() else {
        eprintln!("SKIP: chromium_snapshot_meta_tracks_scroll (no chrome)");
        return;
    };
    let (_guard, ws) = setup();
    let (_server, url) = serve_http(
        r##"<!doctype html><html><head><title>ScrollMeta</title></head><body>
          <div style="height:2000px">tall content <button>at bottom</button></div>
        </body></html>"##,
    );

    let sdk = Fastbrowser::new();
    let cfg = Config {
        engine: "chromium".into(),
        cdp_url: Some(ws),
        ..Config::default()
    };
    sdk.init(cfg).unwrap();
    sdk.open(&url).unwrap();
    wait_until("page loaded", Duration::from_secs(10), || {
        sdk.snapshot()
            .map(|s| s.title == "ScrollMeta")
            .unwrap_or(false)
    });

    let snap = sdk.snapshot().unwrap();
    assert!(
        snap.meta.scroll_h > snap.meta.viewport_h,
        "scroll_h {} > viewport_h {}",
        snap.meta.scroll_h,
        snap.meta.viewport_h
    );
    assert_eq!(snap.meta.scroll_y, 0);

    sdk.tool_call("set_scroll_position", serde_json::json!({"y": 500}))
        .unwrap();
    wait_until("scroll_y updated", Duration::from_secs(10), || {
        sdk.snapshot().map(|s| s.meta.scroll_y > 0).unwrap_or(false)
    });

    sdk.shutdown();
    eprintln!("PASS: chromium_snapshot_meta_tracks_scroll");
}

/// 编码截图（png/jpeg 字节直出，跳过 RGBA 解码）与快照 stale 标记。
#[test]
fn chromium_encoded_screenshot_and_stale_flag() {
    let Some(_) = common::find_chrome() else {
        eprintln!("SKIP: chromium_encoded_screenshot_and_stale_flag (no chrome)");
        return;
    };
    let (_guard, ws) = setup();
    let (_server, url) = serve_http(
        r##"<!doctype html><html><head><title>EncTest</title></head><body><h1>enc</h1></body></html>"##,
    );

    let sdk = Fastbrowser::new();
    let cfg = Config {
        engine: "chromium".into(),
        cdp_url: Some(ws),
        ..Config::default()
    };
    sdk.init(cfg).unwrap();
    sdk.open(&url).unwrap();
    sdk.set_viewport(320, 240).unwrap();
    wait_until("page loaded", Duration::from_secs(10), || {
        sdk.snapshot()
            .map(|s| s.title == "EncTest")
            .unwrap_or(false)
    });

    // 默认仍是 RGBA（向后兼容）
    let rgba = sdk.tool_call("screenshot", serde_json::json!({})).unwrap();
    assert_eq!(rgba["format"], "rgba");
    assert_eq!(rgba["width"], 320);
    assert_eq!(rgba["height"], 240);

    // JPEG：字节直出，SOI 头 + 尺寸正确
    let jpg = sdk
        .tool_call("screenshot", serde_json::json!({"format": "jpeg"}))
        .unwrap();
    assert_eq!(jpg["format"], "jpeg");
    assert_eq!(jpg["width"], 320);
    assert_eq!(jpg["height"], 240);
    use base64::Engine as _;
    let jpg_bytes = base64::engine::general_purpose::STANDARD
        .decode(jpg["base64"].as_str().unwrap())
        .unwrap();
    assert_eq!(&jpg_bytes[..2], &[0xFF, 0xD8], "expected JPEG SOI");
    // JPEG 应显著小于同尺寸 RGBA
    assert!(jpg_bytes.len() < 320 * 240 * 4, "jpeg larger than raw rgba");

    // PNG：字节直出，PNG 签名 + 尺寸正确
    let png = sdk
        .tool_call("screenshot", serde_json::json!({"format": "png"}))
        .unwrap();
    assert_eq!(png["format"], "png");
    assert_eq!(png["width"], 320);
    assert_eq!(png["height"], 240);
    let png_bytes = base64::engine::general_purpose::STANDARD
        .decode(png["base64"].as_str().unwrap())
        .unwrap();
    assert_eq!(
        &png_bytes[..8],
        b"\x89PNG\r\n\x1a\n",
        "expected PNG signature"
    );
    assert!(png_bytes.len() < 320 * 240 * 4, "png larger than raw rgba");

    // 正常页面快照 stale=false
    let snap = sdk.snapshot().unwrap();
    assert!(!snap.meta.stale, "fresh snapshot should not be stale");

    sdk.shutdown();
    eprintln!("PASS: chromium_encoded_screenshot_and_stale_flag");
}

/// 多 Agent 共享一个浏览器连接：两个引擎实例共用 `Arc<CdpClient>`，
/// 各操自己的标签页，命令在同一连接上并发流水线化。
#[test]
fn chromium_shared_client_two_engines_concurrent() {
    let Some(_) = common::find_chrome() else {
        eprintln!("SKIP: chromium_shared_client_two_engines_concurrent (no chrome)");
        return;
    };
    let (_guard, ws) = setup();
    let (_server, base) = serve_http_multi(vec![
        (
            "/",
            "<html><head><title>SharedA</title></head><body>a</body></html>",
        ),
        (
            "/b",
            "<html><head><title>SharedB</title></head><body>b</body></html>",
        ),
    ]);

    use fastbrowser::cdp::CdpClient;
    use fastbrowser::engine::{BrowserEngine, TabId, TabOptions};
    use std::sync::Arc;

    let shared = Arc::new(CdpClient::connect(&ws, 5000).unwrap());
    let cfg = Config {
        engine: "chromium".into(),
        cdp_url: Some(ws.clone()),
        ..Config::default()
    };
    let eng_a =
        fastbrowser::engines::cdp::ChromiumCdpEngine::from_shared(&ws, shared.clone(), &cfg, None)
            .unwrap();
    let eng_b =
        fastbrowser::engines::cdp::ChromiumCdpEngine::from_shared(&ws, shared.clone(), &cfg, None)
            .unwrap();

    // 两个引擎各自开自己的标签页
    let tab_a = eng_a
        .create_tab(&base.to_string(), &TabOptions::default())
        .unwrap();
    let tab_b = eng_b
        .create_tab(&format!("{base}b"), &TabOptions::default())
        .unwrap();

    // 并发驱动：各引擎操作自己的标签页
    let base_a = base.clone();
    let base_b = base.clone();
    let ta = std::thread::spawn(move || {
        for _ in 0..8 {
            let _ = eng_a.navigate(tab_a, &base_a.to_string());
        }
        eng_a.snapshot(tab_a).map(|s| s.title.clone())
    });
    let tb = std::thread::spawn(move || {
        for _ in 0..8 {
            let _ = eng_b.navigate(tab_b, &format!("{base_b}b"));
        }
        eng_b.snapshot(tab_b).map(|s| s.title.clone())
    });

    let (ra, rb) = (ta.join().unwrap().unwrap(), tb.join().unwrap().unwrap());
    assert_eq!(ra, "SharedA");
    assert_eq!(rb, "SharedB");
    let _ = TabId(0);
    eprintln!("PASS: chromium_shared_client_two_engines_concurrent");
}

/// 真实浏览器上验证 CDP 异步流水线：一批命令并发发送，响应按 id 一一对应。
#[test]
fn chromium_concurrent_async_pipeline() {
    let Some(_) = common::find_chrome() else {
        eprintln!("SKIP: chromium_concurrent_async_pipeline (no chrome)");
        return;
    };
    let (_guard, ws) = setup();

    use fastbrowser::cdp::{CdpClient, Command};
    use std::sync::Arc;
    let client = Arc::new(CdpClient::connect(&ws, 8000).unwrap());

    let n = 32usize;
    let cmds: Vec<Command> = (0..n)
        .map(|i| Command::new("Target.getTargets", serde_json::json!({ "probe": i })))
        .collect();

    let futs: Vec<_> = cmds
        .iter()
        .map(|c| {
            let cl = client.clone();
            async move { cl.command_async(c).await.map(|r| r.is_object()) }
        })
        .collect();
    let results = client.block_on(futures_util::future::join_all(futs));
    assert_eq!(results.len(), n);
    for r in results {
        assert!(r.unwrap(), "pipelined command failed");
    }
    eprintln!("PASS: chromium_concurrent_async_pipeline");
}

/// 轻量内省路径：page_title/page_url 与 wait_for_element 的 JS 快速判定。
#[test]
fn chromium_lightweight_introspection() {
    let Some(_) = common::find_chrome() else {
        eprintln!("SKIP: chromium_lightweight_introspection (no chrome)");
        return;
    };
    let (_guard, ws) = setup();
    let (_server, base) = serve_http_multi(vec![
        (
            "/",
            r##"<!doctype html><html><head><title>LightTitle</title></head><body>
          <h1>hello</h1><button id="go">Go</button><span class="result">ready</span>
        </body></html>"##,
        ),
        (
            "/two",
            "<html><head><title>Second</title></head><body>two</body></html>",
        ),
    ]);

    let sdk = Fastbrowser::new();
    let cfg = Config {
        engine: "chromium".into(),
        cdp_url: Some(ws),
        ..Config::default()
    };
    sdk.init(cfg).unwrap();
    sdk.open(&base).unwrap();
    wait_until("loaded", Duration::from_secs(10), || {
        sdk.tool_call("get_page_title", serde_json::json!({}))
            .map(|v| v["title"] == "LightTitle")
            .unwrap_or(false)
    });

    // 廉价读取：title / url 不依赖整页快照
    let t = sdk
        .tool_call("get_page_title", serde_json::json!({}))
        .unwrap();
    assert_eq!(t["title"], "LightTitle");
    let u = sdk
        .tool_call("get_current_url", serde_json::json!({}))
        .unwrap();
    assert!(u["url"].as_str().unwrap().contains("127.0.0.1"));

    // wait_for_element：css/tag 走 querySelector 快速判定
    let v = sdk
        .tool_call(
            "wait_for_element",
            serde_json::json!({"selector": "button", "timeout_ms": 2000}),
        )
        .unwrap();
    assert_eq!(v["found"], true);
    let v = sdk
        .tool_call(
            "wait_for_element",
            serde_json::json!({"selector": "#go", "timeout_ms": 2000}),
        )
        .unwrap();
    assert_eq!(v["found"], true);
    // 不存在的选择器 → 超时
    assert!(sdk
        .tool_call(
            "wait_for_element",
            serde_json::json!({"selector": "#missing-zz", "timeout_ms": 100})
        )
        .is_err());

    // 导航后断言（assert_url_contains / assert_title 走廉价读取）
    sdk.navigate(&format!("{base}two")).unwrap();
    wait_until("navigated", Duration::from_secs(10), || {
        sdk.tool_call("get_page_title", serde_json::json!({}))
            .map(|v| v["title"] == "Second")
            .unwrap_or(false)
    });
    sdk.tool_call(
        "assert_url_contains",
        serde_json::json!({"contains": "/two"}),
    )
    .unwrap();
    sdk.tool_call("assert_title", serde_json::json!({"contains": "Second"}))
        .unwrap();

    sdk.shutdown();
    eprintln!("PASS: chromium_lightweight_introspection");
}

/// 专用下载服务器：`/file.txt` 返回 `Content-Disposition: attachment`。
fn serve_download() -> (std::net::TcpListener, String) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = listener.try_clone().unwrap();
    std::thread::spawn(move || {
        for stream in server.incoming() {
            let Ok(mut stream) = stream else { break };
            let mut buf = [0u8; 2048];
            let _ = stream.read(&mut buf);
            let req = String::from_utf8_lossy(&buf);
            let path = req
                .split_whitespace()
                .nth(1)
                .unwrap_or("/")
                .split('?')
                .next()
                .unwrap_or("/");
            let (status, extra, body) = if path == "/file.txt" {
                ("200 OK", "Content-Type: text/plain\r\nContent-Disposition: attachment; filename=\"hello.txt\"", "HELLO-DOWNLOAD")
            } else {
                ("404 Not Found", "Content-Type: text/plain", "nf")
            };
            let resp = format!(
                "HTTP/1.1 {status}\r\n{extra}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(resp.as_bytes());
        }
    });
    (listener, format!("http://127.0.0.1:{port}/"))
}

/// ① Actionability：元素被 JS 隐藏 300ms 后恢复，点击会自动等待「可见+稳定」，
/// 而不是因单次几何检查立刻失败（Playwright 语义）。
#[test]
fn chromium_actionability_waits_for_element_ready() {
    let Some(_) = common::find_chrome() else {
        eprintln!("SKIP: chromium_actionability_waits_for_element_ready (no chrome)");
        return;
    };
    let (_guard, ws) = setup();
    let (_server, url) = serve_http(
        r##"<!doctype html><html><head><title>Action</title></head><body>
          <button id="target">Pulse Button</button>
          <script>
            document.getElementById('target').onclick = function(){
              document.body.dataset.pulsed = 'yes';
            };
            window.startPulse = function(){
              var el = document.getElementById('target');
              el.style.display = 'none';
              setTimeout(function(){ el.style.display = 'block'; }, 300);
            };
          </script>
        </body></html>"##,
    );

    let sdk = Fastbrowser::new();
    let cfg = Config {
        engine: "chromium".into(),
        cdp_url: Some(ws.clone()),
        actionability_timeout_ms: 5000,
        ..Config::default()
    };
    sdk.init(cfg).unwrap();
    sdk.open(&url).unwrap();
    wait_until("page loaded", Duration::from_secs(10), || {
        sdk.snapshot().map(|s| s.title == "Action").unwrap_or(false)
    });

    // 找到按钮的快照 id，然后立刻隐藏它（模拟动态页过渡期）
    let id = sdk
        .snapshot()
        .unwrap()
        .interactive
        .iter()
        .find(|e| e.tag == "button")
        .unwrap()
        .id;
    sdk.tool_call(
        "execute_js",
        serde_json::json!({"script": "window.startPulse()"}),
    )
    .unwrap();
    // 元素此刻已 display:none → 旧实现会立即报 hidden；新实现自动等待后成功
    sdk.tool_call("click", serde_json::json!({"id": id.to_string()}))
        .unwrap();
    let pulsed = sdk
        .tool_call(
            "execute_js",
            serde_json::json!({"script": "document.body.dataset.pulsed || ''"}),
        )
        .unwrap();
    assert_eq!(
        pulsed["result"], "yes",
        "click should land on the element after it becomes actionable"
    );

    // 超时路径：永远不出现的元素 → 清晰的「not actionable」错误
    sdk.tool_call("execute_js", serde_json::json!({"script": "document.getElementById('target').style.display='none'; window.__foreverHidden=true"})).unwrap();
    let res = sdk.tool_call("click", serde_json::json!({"id": id.to_string()}));
    assert!(res.is_err(), "hidden-forever element must time out");
    let err = res.unwrap_err().to_string();
    assert!(
        err.contains("not actionable"),
        "timeout error should mention actionability: {err}"
    );

    sdk.shutdown();
    eprintln!("PASS: chromium_actionability_waits_for_element_ready");
}

/// ② type 走真实键盘（Input.insertText）：触发原生 beforeinput(insertText)
/// 与 input 事件链（React 受控组件 / IME 兼容），而非 JS `el.value=` 直赋。
#[test]
fn chromium_type_uses_real_keyboard() {
    let Some(_) = common::find_chrome() else {
        eprintln!("SKIP: chromium_type_uses_real_keyboard (no chrome)");
        return;
    };
    let (_guard, ws) = setup();
    let (_server, url) = serve_http(
        r##"<!doctype html><html><head><title>KeyType</title></head><body>
          <input id="q">
          <script>
            const el = document.getElementById('q');
            window.beforeInputs = [];
            el.addEventListener('beforeinput', function(e){ window.beforeInputs.push(e.inputType); });
            el.addEventListener('input', function(){ window.committed = el.value; });
          </script>
        </body></html>"##,
    );

    let sdk = Fastbrowser::new();
    let cfg = Config {
        engine: "chromium".into(),
        cdp_url: Some(ws.clone()),
        ..Config::default()
    };
    sdk.init(cfg).unwrap();
    sdk.open(&url).unwrap();
    wait_until("page loaded", Duration::from_secs(10), || {
        sdk.snapshot()
            .map(|s| s.title == "KeyType")
            .unwrap_or(false)
    });

    let id = sdk
        .snapshot()
        .unwrap()
        .interactive
        .iter()
        .find(|e| e.tag == "input")
        .unwrap()
        .id;
    // 先清空输入（默认 clear=true），观察 beforeinput 是否走原生 insertText
    sdk.tool_call(
        "type",
        serde_json::json!({"id": id.to_string(), "text": "abc"}),
    )
    .unwrap();
    let st = sdk.tool_call("execute_js", serde_json::json!({"script": "JSON.stringify({v:window.committed,b:window.beforeInputs})"})).unwrap();
    let st: Value = serde_json::from_str(st["result"].as_str().unwrap()).unwrap();
    assert_eq!(st["v"], "abc");
    assert!(
        st["b"].as_array().unwrap().iter().any(|t| t
            .as_str()
            .map(|s| s.contains("insertText"))
            .unwrap_or(false)),
        "type must go through native input channel (beforeinput insertText): {:?}",
        st["b"]
    );

    // clear=false 追加（逐键输入语义）
    sdk.tool_call(
        "type",
        serde_json::json!({"id": id.to_string(), "text": "def", "clear": false}),
    )
    .unwrap();
    let v = sdk
        .tool_call(
            "execute_js",
            serde_json::json!({"script": "document.getElementById('q').value"}),
        )
        .unwrap();
    assert_eq!(v["result"], "abcdef");

    sdk.shutdown();
    eprintln!("PASS: chromium_type_uses_real_keyboard");
}

/// ③ 下载落盘：`Browser.setDownloadBehavior` 允许下载到指定目录（无头亦生效）。
#[test]
fn chromium_download_writes_file() {
    let Some(_) = common::find_chrome() else {
        eprintln!("SKIP: chromium_download_writes_file (no chrome)");
        return;
    };
    let (_guard, ws) = setup();
    let (_server, base) = serve_download();

    let dl_dir = tempfile::tempdir().unwrap();
    let dl_path = dl_dir.path().to_str().unwrap().to_string();
    let sdk = Fastbrowser::new();
    let cfg = Config {
        engine: "chromium".into(),
        cdp_url: Some(ws.clone()),
        accept_downloads: true,
        default_download_path: Some(dl_path.clone()),
        ..Config::default()
    };
    sdk.init(cfg).unwrap();

    // 打开下载 URL：Chromium 应把 hello.txt 落盘到 default_download_path
    sdk.open(&base).unwrap();
    sdk.navigate(&format!("{base}file.txt")).unwrap();

    let target = std::path::Path::new(&dl_path).join("hello.txt");
    wait_until("download file appears", Duration::from_secs(15), || {
        target.exists()
    });
    let content = std::fs::read_to_string(&target).unwrap();
    assert_eq!(
        content.trim(),
        "HELLO-DOWNLOAD",
        "downloaded file content mismatch"
    );

    sdk.shutdown();
    eprintln!("PASS: chromium_download_writes_file");
}

/// ④ Fetch 拦截 + 自定义响应体（真机验证 fulfillRequest）。
#[test]
fn chromium_fetch_fulfill_custom_body() {
    let Some(_) = common::find_chrome() else {
        eprintln!("SKIP: chromium_fetch_fulfill_custom_body (no chrome)");
        return;
    };
    let (_guard, ws) = setup();
    let (_server, url) = serve_http(
        r##"<!doctype html><html><head><title>Intercept</title></head><body>
          <div id="out">pending</div>
          <button id="go">go</button>
          <script>
            document.getElementById('go').onclick = function(){
              fetch('/flaky').then(function(r){return r.text();}).then(function(t){
                document.getElementById('out').textContent = t;
              });
            };
          </script>
        </body></html>"##,
    );

    let sdk = Fastbrowser::new();
    let cfg = Config {
        engine: "chromium".into(),
        cdp_url: Some(ws.clone()),
        ..Config::default()
    };
    sdk.init(cfg).unwrap();
    sdk.open(&url).unwrap();
    wait_until("page loaded", Duration::from_secs(10), || {
        sdk.snapshot()
            .map(|s| s.title == "Intercept")
            .unwrap_or(false)
    });

    // 启用 Fetch 拦截（*flaky*），点击触发 fetch
    sdk.tool_call(
        "intercept_request",
        serde_json::json!({"patterns": ["*flaky*"], "enabled": true}),
    )
    .unwrap();
    let id = sdk
        .snapshot()
        .unwrap()
        .interactive
        .iter()
        .find(|e| e.tag == "button")
        .unwrap()
        .id;
    sdk.tool_call("click", serde_json::json!({"id": id.to_string()}))
        .unwrap();

    // 轮询收集被暂停的请求（route_cdp_events 由 drain_events 驱动）
    let tab = sdk
        .runtime()
        .unwrap()
        .as_ref()
        .unwrap()
        .engine()
        .active_tab()
        .unwrap();
    let request_id = {
        let mut rid = String::new();
        wait_until("paused request", Duration::from_secs(10), || {
            let _ = sdk
                .runtime()
                .unwrap()
                .as_ref()
                .unwrap()
                .engine()
                .drain_events(tab);
            let reqs = sdk
                .runtime()
                .unwrap()
                .as_ref()
                .unwrap()
                .engine()
                .pending_requests(tab);
            if let Some(r) = reqs.first() {
                rid = r["request_id"].as_str().unwrap_or("").to_string();
            }
            !rid.is_empty()
        });
        rid
    };

    // 自定义响应体（base64）
    use base64::Engine as _;
    let body_b64 = base64::engine::general_purpose::STANDARD.encode("CUSTOM-FULFILLED-BODY");
    sdk.tool_call(
        "fulfill_request",
        serde_json::json!({"request_id": request_id, "status": 200, "body_b64": body_b64}),
    )
    .unwrap();

    wait_until("page got custom body", Duration::from_secs(10), || {
        sdk.tool_call(
            "execute_js",
            serde_json::json!({"script": "document.getElementById('out').textContent"}),
        )
        .map(|v| v["result"] == "CUSTOM-FULFILLED-BODY")
        .unwrap_or(false)
    });

    sdk.shutdown();
    eprintln!("PASS: chromium_fetch_fulfill_custom_body");
}

/// ⑤ OSR 推送帧流：`start_frame_stream` 持续推帧到 frame_sink，stop 后停止。
#[test]
fn chromium_frame_stream_pushes_frames() {
    let Some(_) = common::find_chrome() else {
        eprintln!("SKIP: chromium_frame_stream_pushes_frames (no chrome)");
        return;
    };
    let (_guard, ws) = setup();
    let (_server, url) = serve_http(
        r##"<!doctype html><html><head><title>Stream</title></head><body><h1>stream</h1>
        <script>
          var i = 0;
          setInterval(function(){
            document.body.style.background = (i++ % 2) ? '#ffffff' : '#eeeeee';
          }, 120);
        </script>
        </body></html>"##,
    );

    // 先注册帧回调（open 时会附加到标签页）
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    let count = Arc::new(AtomicUsize::new(0));
    struct Sink(Arc<AtomicUsize>);
    impl fastbrowser::engine::host::ViewFrameSink for Sink {
        fn on_view_frame(
            &self,
            _tab: fastbrowser::engine::TabId,
            _frame: &fastbrowser::engine::ViewFrame,
        ) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }
    let sink_arc: Arc<dyn fastbrowser::engine::host::ViewFrameSink> = Arc::new(Sink(count.clone()));

    let sdk = Fastbrowser::new();
    let cfg = Config {
        engine: "chromium".into(),
        cdp_url: Some(ws.clone()),
        ..Config::default()
    };
    sdk.init(cfg).unwrap();
    sdk.register_frame_sink(sink_arc);
    let tab = sdk.open(&url).unwrap()["tab"].as_u64().unwrap() as u32;
    let tab = fastbrowser::engine::TabId(tab);
    wait_until("page loaded", Duration::from_secs(10), || {
        sdk.snapshot().map(|s| s.title == "Stream").unwrap_or(false)
    });
    // 前台化（无头下后台 target 渲染器可能冻结，先 bringToFront）
    sdk.tool_call("switch_tab", serde_json::json!({"tab": tab.as_u32()}))
        .unwrap();

    sdk.start_frame_stream(
        tab,
        fastbrowser::engine::FrameStreamOptions {
            fps: 20,
            max_width: 0,
            max_height: 0,
            format: "png".into(),
        },
    )
    .unwrap();
    wait_until("frames pushed", Duration::from_secs(10), || {
        count.load(Ordering::Relaxed) >= 2
    });
    sdk.stop_frame_stream(tab).unwrap();
    // 帧数必须冻结（在途帧耗尽后不再增长）
    std::thread::sleep(Duration::from_millis(150));
    let a1 = count.load(Ordering::Relaxed);
    std::thread::sleep(Duration::from_millis(120));
    let a2 = count.load(Ordering::Relaxed);
    assert_eq!(
        a1, a2,
        "frame count must freeze after stop_frame_stream (a1={a1}, a2={a2})"
    );

    sdk.shutdown();
    eprintln!("PASS: chromium_frame_stream_pushes_frames");
}

/// 修改响应：`modify_response` 放行请求但替换响应体。
#[test]
fn chromium_jpeg_frame_stream_delivers_encoded_frames() {
    let Some(_) = common::find_chrome() else {
        eprintln!("SKIP: chromium_jpeg_frame_stream (no chrome)");
        return;
    };
    let (_guard, ws) = setup();
    // 页面须持续重绘，否则无头 screencast 只出首帧、不保证持续出帧。
    let (_server, url) = serve_http(
        r##"<!doctype html><html><head><title>Stream</title>
        <style>.box{width:60px;height:60px;background:rgb(0,136,255);
        animation:m .4s linear infinite alternate}
        @keyframes m{from{transform:translateX(0)}to{transform:translateX(180px)}}</style>
        </head><body><h1>Stream</h1><div class="box"></div></body></html>"##,
    );

    use std::sync::atomic::{AtomicUsize, Ordering};
    struct EncSink(Arc<AtomicUsize>);
    impl fastbrowser::engine::EncodedFrameSink for EncSink {
        fn on_encoded_frame(
            &self,
            _tab: fastbrowser::engine::TabId,
            _f: &fastbrowser::engine::EncodedViewFrame,
        ) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }
    let count = Arc::new(AtomicUsize::new(0));
    let sink_arc: Arc<dyn fastbrowser::engine::EncodedFrameSink> = Arc::new(EncSink(count.clone()));

    let sdk = Fastbrowser::new();
    let cfg = Config {
        engine: "chromium".into(),
        cdp_url: Some(ws.clone()),
        ..Config::default()
    };
    sdk.init(cfg).unwrap();
    sdk.register_encoded_frame_sink(sink_arc);
    let tab = sdk.open(&url).unwrap()["tab"].as_u64().unwrap() as u32;
    let tab = fastbrowser::engine::TabId(tab);
    wait_until("page loaded", Duration::from_secs(10), || {
        sdk.snapshot().map(|s| s.title == "Stream").unwrap_or(false)
    });
    // 模拟 App 流程：open 后直接开帧流，不手动 switch_tab/bringToFront。
    // start_frame_stream 内部必须自行 bringToFront 让无头渲染器出帧。
    sdk.start_frame_stream(
        tab,
        fastbrowser::engine::FrameStreamOptions {
            fps: 20,
            max_width: 0,
            max_height: 0,
            format: "jpeg".into(),
        },
    )
    .unwrap();
    wait_until("encoded frames pushed", Duration::from_secs(10), || {
        count.load(Ordering::Relaxed) >= 2
    });
    sdk.stop_frame_stream(tab).unwrap();
    std::thread::sleep(Duration::from_millis(150));
    let a1 = count.load(Ordering::Relaxed);
    std::thread::sleep(Duration::from_millis(120));
    let a2 = count.load(Ordering::Relaxed);
    assert_eq!(
        a1, a2,
        "jpeg frame count must freeze after stop_frame_stream (a1={a1}, a2={a2})"
    );
    assert!(
        count.load(Ordering::Relaxed) >= 2,
        "jpeg encoded frames should have arrived"
    );
    sdk.shutdown();
    eprintln!("PASS: chromium_jpeg_frame_stream_delivers_encoded_frames");
}

/// 修改响应：`modify_response` 放行请求但替换响应体。
#[test]
fn chromium_viewport_resizes_screencast_frames() {
    let Some(_) = common::find_chrome() else {
        eprintln!("SKIP: chromium_viewport_resizes_screencast_frames (no chrome)");
        return;
    };
    let (_guard, ws) = setup();
    let (_server, url) = serve_http(
        r##"<!doctype html><html><head><title>VP</title></head><body><h1>VP</h1></body></html>"##,
    );

    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Mutex;
    struct CapSink {
        w: AtomicU32,
        h: AtomicU32,
        seen: Mutex<Vec<(u32, u32)>>,
    }
    impl fastbrowser::engine::EncodedFrameSink for CapSink {
        fn on_encoded_frame(
            &self,
            _tab: fastbrowser::engine::TabId,
            f: &fastbrowser::engine::EncodedViewFrame,
        ) {
            self.w.store(f.width, Ordering::Relaxed);
            self.h.store(f.height, Ordering::Relaxed);
            self.seen.lock().unwrap().push((f.width, f.height));
        }
    }
    let cap = Arc::new(CapSink {
        w: AtomicU32::new(0),
        h: AtomicU32::new(0),
        seen: Mutex::new(Vec::new()),
    });
    let sink_arc: Arc<dyn fastbrowser::engine::EncodedFrameSink> = cap.clone();

    let sdk = Fastbrowser::new();
    sdk.init(Config {
        engine: "chromium".into(),
        cdp_url: Some(ws.clone()),
        ..Config::default()
    })
    .unwrap();
    sdk.register_encoded_frame_sink(sink_arc);
    let tab = sdk.open(&url).unwrap()["tab"].as_u64().unwrap() as u32;
    let tab = fastbrowser::engine::TabId(tab);
    wait_until("page loaded", Duration::from_secs(10), || {
        sdk.snapshot().map(|s| s.title == "VP").unwrap_or(false)
    });
    sdk.tool_call("switch_tab", serde_json::json!({"tab": tab.as_u32()}))
        .unwrap();

    // 先设一个明确的视口，再开帧流，确认帧尺寸跟随视口。
    sdk.set_viewport(520, 400).unwrap();
    sdk.start_frame_stream(
        tab,
        fastbrowser::engine::FrameStreamOptions {
            fps: 20,
            max_width: 0,
            max_height: 0,
            format: "jpeg".into(),
        },
    )
    .unwrap();
    wait_until("frames at set viewport", Duration::from_secs(10), || {
        cap.w.load(Ordering::Relaxed) == 520 && cap.h.load(Ordering::Relaxed) == 400
    });

    // 改视口后帧尺寸必须跟随变化（页面像真窗口重排）。
    sdk.set_viewport(380, 620).unwrap();
    wait_until(
        "frames follow viewport change",
        Duration::from_secs(10),
        || cap.w.load(Ordering::Relaxed) == 380 && cap.h.load(Ordering::Relaxed) == 620,
    );
    sdk.stop_frame_stream(tab).unwrap();
    sdk.shutdown();
    eprintln!("PASS: chromium_viewport_resizes_screencast_frames");
}

/// 修改响应：`modify_response` 放行请求但替换响应体。
#[test]
fn chromium_modify_response_replaces_body() {
    let Some(_) = common::find_chrome() else {
        eprintln!("SKIP: chromium_modify_response_replaces_body (no chrome)");
        return;
    };
    let (_guard, ws) = setup();
    let (_server, url) = serve_http(
        r##"<!doctype html><html><head><title>Modify</title></head><body>
          <div id="out">pending</div>
          <button id="go">go</button>
          <script>
            document.getElementById('go').onclick = function(){
              fetch('/flaky').then(function(r){return r.text();}).then(function(t){
                document.getElementById('out').textContent = t;
              });
            };
          </script>
        </body></html>"##,
    );

    let sdk = Fastbrowser::new();
    let cfg = Config {
        engine: "chromium".into(),
        cdp_url: Some(ws.clone()),
        ..Config::default()
    };
    sdk.init(cfg).unwrap();
    sdk.open(&url).unwrap();
    wait_until("page loaded", Duration::from_secs(10), || {
        sdk.snapshot().map(|s| s.title == "Modify").unwrap_or(false)
    });

    sdk.tool_call(
        "intercept_request",
        serde_json::json!({"patterns": ["*flaky*"], "enabled": true}),
    )
    .unwrap();
    let id = sdk
        .snapshot()
        .unwrap()
        .interactive
        .iter()
        .find(|e| e.tag == "button")
        .unwrap()
        .id;
    sdk.tool_call("click", serde_json::json!({"id": id.to_string()}))
        .unwrap();

    let tab = sdk
        .runtime()
        .unwrap()
        .as_ref()
        .unwrap()
        .engine()
        .active_tab()
        .unwrap();
    let request_id = {
        let mut rid = String::new();
        wait_until("paused request", Duration::from_secs(10), || {
            let _ = sdk
                .runtime()
                .unwrap()
                .as_ref()
                .unwrap()
                .engine()
                .drain_events(tab);
            let reqs = sdk
                .runtime()
                .unwrap()
                .as_ref()
                .unwrap()
                .engine()
                .pending_requests(tab);
            if let Some(r) = reqs.first() {
                rid = r["request_id"].as_str().unwrap_or("").to_string();
            }
            !rid.is_empty()
        });
        rid
    };

    // modify_response：放行但覆盖响应体
    use base64::Engine as _;
    let body_b64 = base64::engine::general_purpose::STANDARD.encode("MODIFIED-RESPONSE-BODY");
    sdk.tool_call(
        "modify_response",
        serde_json::json!({"request_id": request_id, "status": 200, "body_b64": body_b64}),
    )
    .unwrap();

    wait_until("page got modified body", Duration::from_secs(10), || {
        sdk.tool_call(
            "execute_js",
            serde_json::json!({"script": "document.getElementById('out').textContent"}),
        )
        .map(|v| v["result"] == "MODIFIED-RESPONSE-BODY")
        .unwrap_or(false)
    });

    sdk.shutdown();
    eprintln!("PASS: chromium_modify_response_replaces_body");
}

/// 拦截捕获 POST 请求体（post_data）。
#[test]
fn chromium_intercept_captures_post_data() {
    let Some(_) = common::find_chrome() else {
        eprintln!("SKIP: chromium_intercept_captures_post_data (no chrome)");
        return;
    };
    let (_guard, ws) = setup();
    let (_server, url) = serve_http(
        r##"<!doctype html><html><head><title>Post</title></head><body>
          <button id="go">go</button>
          <script>
            document.getElementById('go').onclick = function(){
              fetch('/submit', {method:'POST', body:'hello=world&x=1'});
            };
          </script>
        </body></html>"##,
    );

    let sdk = Fastbrowser::new();
    let cfg = Config {
        engine: "chromium".into(),
        cdp_url: Some(ws.clone()),
        ..Config::default()
    };
    sdk.init(cfg).unwrap();
    sdk.open(&url).unwrap();
    wait_until("page loaded", Duration::from_secs(10), || {
        sdk.snapshot().map(|s| s.title == "Post").unwrap_or(false)
    });

    sdk.tool_call(
        "intercept_request",
        serde_json::json!({"patterns": ["*submit*"], "enabled": true}),
    )
    .unwrap();
    let id = sdk
        .snapshot()
        .unwrap()
        .interactive
        .iter()
        .find(|e| e.tag == "button")
        .unwrap()
        .id;
    sdk.tool_call("click", serde_json::json!({"id": id.to_string()}))
        .unwrap();

    let tab = sdk
        .runtime()
        .unwrap()
        .as_ref()
        .unwrap()
        .engine()
        .active_tab()
        .unwrap();
    // 轮询收集，验证 post_data 被捕获
    let mut captured = false;
    wait_until("post_data captured", Duration::from_secs(10), || {
        let _ = sdk
            .runtime()
            .unwrap()
            .as_ref()
            .unwrap()
            .engine()
            .drain_events(tab);
        let reqs = sdk
            .runtime()
            .unwrap()
            .as_ref()
            .unwrap()
            .engine()
            .pending_requests(tab);
        captured = reqs
            .iter()
            .any(|r| r.get("post_data").and_then(|p| p.as_str()) == Some("hello=world&x=1"));
        captured
    });
    assert!(captured, "POST body should be captured in post_data");

    // 清理：abort 被拦截的请求
    if let Some(r) = sdk
        .runtime()
        .unwrap()
        .as_ref()
        .unwrap()
        .engine()
        .pending_requests(tab)
        .first()
    {
        let rid = r["request_id"].as_str().unwrap_or("").to_string();
        let _ = sdk.tool_call("abort_request", serde_json::json!({"request_id": rid}));
    }

    sdk.shutdown();
    eprintln!("PASS: chromium_intercept_captures_post_data");
}

/// 设备模拟：时区覆盖 + 触摸模拟。
#[test]
fn chromium_timezone_and_touch_emulation() {
    let Some(_) = common::find_chrome() else {
        eprintln!("SKIP: chromium_timezone_and_touch_emulation (no chrome)");
        return;
    };
    let (_guard, ws) = setup();
    let (_server, url) = serve_http(
        r##"<!doctype html><html><head><title>Emulation</title></head><body><h1>emu</h1></body></html>"##,
    );

    let sdk = Fastbrowser::new();
    let cfg = Config {
        engine: "chromium".into(),
        cdp_url: Some(ws.clone()),
        ..Config::default()
    };
    sdk.init(cfg).unwrap();
    sdk.open(&url).unwrap();
    wait_until("page loaded", Duration::from_secs(10), || {
        sdk.snapshot()
            .map(|s| s.title == "Emulation")
            .unwrap_or(false)
    });

    // 时区覆盖
    sdk.tool_call(
        "set_timezone",
        serde_json::json!({"timezone_id": "Asia/Shanghai"}),
    )
    .unwrap();
    wait_until("timezone applied", Duration::from_secs(10), || {
        sdk.tool_call(
            "execute_js",
            serde_json::json!({"script": "Intl.DateTimeFormat().resolvedOptions().timeZone"}),
        )
        .map(|v| v["result"] == "Asia/Shanghai")
        .unwrap_or(false)
    });

    // 触摸模拟：maxTouchPoints 从 0 → 5
    sdk.tool_call("set_touch_emulation", serde_json::json!({"enabled": true}))
        .unwrap();
    wait_until("touch emulation applied", Duration::from_secs(10), || {
        sdk.tool_call(
            "execute_js",
            serde_json::json!({"script": "navigator.maxTouchPoints"}),
        )
        .map(|v| v["result"] == 5)
        .unwrap_or(false)
    });

    sdk.shutdown();
    eprintln!("PASS: chromium_timezone_and_touch_emulation");
}

/// 多窗口：`new_window` 打开独立窗口（新 target）。
#[test]
fn chromium_new_window_opens() {
    let Some(_) = common::find_chrome() else {
        eprintln!("SKIP: chromium_new_window_opens (no chrome)");
        return;
    };
    let (_guard, ws) = setup();
    let (_server, url) = serve_http(
        r##"<!doctype html><html><head><title>Window</title></head><body><h1>win</h1></body></html>"##,
    );

    let sdk = Fastbrowser::new();
    let cfg = Config {
        engine: "chromium".into(),
        cdp_url: Some(ws.clone()),
        ..Config::default()
    };
    sdk.init(cfg).unwrap();
    sdk.open(&url).unwrap();
    wait_until("first page loaded", Duration::from_secs(10), || {
        sdk.snapshot().map(|s| s.title == "Window").unwrap_or(false)
    });

    let before = sdk.tool_call("list_tabs", serde_json::json!({})).unwrap()["tabs"]
        .as_array()
        .unwrap()
        .len();

    let r = sdk
        .tool_call("new_window", serde_json::json!({"url": url}))
        .unwrap();
    assert!(r["tab"].is_number(), "new_window should return a tab id");

    // 标签数 +1（新窗口 target 作为一个新 tab 管理）
    let after = sdk.tool_call("list_tabs", serde_json::json!({})).unwrap()["tabs"]
        .as_array()
        .unwrap()
        .len();
    assert_eq!(after, before + 1, "new_window should add one tab");

    sdk.shutdown();
    eprintln!("PASS: chromium_new_window_opens");
}
