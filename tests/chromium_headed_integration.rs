// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! 有头（headed）Chrome 集成测试（feature `engine-cdp`）。
//!
//! 无头测试（`chromium_integration.rs`）用 `--headless=new` 验证了协议链路；
//! 本文件**去掉 `--headless=new`**，启动真实窗口，验证「人类浏览器」形态：
//! 真实合成器渲染、`document.hidden` 可见性语义、前台/后台标签页、真实窗口下载、
//! 有头下的 Actionability / 真实键盘 / 帧流 / 上下文隔离等。
//!
//! 前置：本机存在 Chrome/Chromium（`CHROME_PATH` 可指定）且**有 GUI 会话**
//! （headed 需要 WindowServer）。无 GUI/无 Chrome 时打印 SKIP 并返回，不阻塞 CI。
//!
//! 注意：headed 测试会短暂弹出真实浏览器窗口——这是测试预期行为。

#![cfg(feature = "engine-cdp")]

mod common;

use std::io::{Read, Write};
use std::sync::Arc;
use std::time::{Duration, Instant};

use fastbrowser::sdk::Fastbrowser;
use fastbrowser::Config;
use serde_json::Value;

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

/// 有头环境准备：共享有头 Chrome（无 GUI 会话时返回 None，测试 SKIP）。
fn setup_headed() -> Option<(common::BrowserGuard, String)> {
    let b = common::shared_browser(false).ok()?;
    let guard = common::browser_guard(b);
    let ws = b.ws.clone();
    reset_browser(&ws);
    Some((guard, ws))
}

/// 清理上一个测试遗留的 tab/cookie/storage。
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

/// 迷你 HTTP 服务器（任意路径返回 body）。
fn serve_http(body: &'static str) -> (std::net::TcpListener, String) {
    serve_http_multi(vec![("/", body)])
}

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

/// 下载服务器：`/file.txt` 带 `Content-Disposition: attachment`。
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

/// 辅助：对指定 tab 截图。
fn shot(sdk: &Fastbrowser, tab: fastbrowser::engine::TabId) -> fastbrowser::engine::Image {
    sdk.runtime()
        .unwrap()
        .as_ref()
        .unwrap()
        .engine()
        .screenshot(tab)
        .unwrap()
}

fn is_nonblank(img: &fastbrowser::engine::Image) -> bool {
    img.is_valid()
        && img.width > 0
        && img.height > 0
        && img
            .rgba
            .chunks_exact(4)
            .take(8192)
            .any(|c| c[0] != 0 || c[1] != 0 || c[2] != 0)
}

fn new_sdk(ws: &str) -> Fastbrowser {
    let sdk = Fastbrowser::new();
    let cfg = Config {
        engine: "chromium".into(),
        cdp_url: Some(ws.to_string()),
        ..Config::default()
    };
    sdk.init(cfg).unwrap();
    sdk
}

#[allow(dead_code)]
fn tab_id(sdk: &Fastbrowser) -> fastbrowser::engine::TabId {
    sdk.runtime()
        .unwrap()
        .as_ref()
        .unwrap()
        .engine()
        .active_tab()
        .unwrap()
}

/// ① 有头 Agent 主循环全链路：导航→快照→点击→输入→提取→多标签。
#[test]
fn headed_basic_agent_flow() {
    let Some((_guard, ws)) = setup_headed() else {
        eprintln!("SKIP: headed_basic_agent_flow (no headed chrome / GUI session)");
        return;
    };
    let (_server, url) = serve_http(
        r##"<!doctype html><html><head><title>Headed E2E</title></head><body>
          <h1>Headed Test</h1>
          <input id="q" placeholder="search">
          <button id="go">Go</button>
          <a href="#next">Next link</a>
          <span>hello headed world</span>
          <script>
            document.getElementById('go').onclick = function(){ document.body.dataset.ran='yes'; };
          </script>
        </body></html>"##,
    );

    let sdk = new_sdk(&ws);
    let out = sdk.open(&url).unwrap();
    assert!(out["tab"].is_number());
    wait_until("page loaded", Duration::from_secs(10), || {
        sdk.snapshot()
            .map(|s| s.title == "Headed E2E")
            .unwrap_or(false)
    });

    // 快照：真实 DOM 交互元素
    let snap = sdk.snapshot().unwrap();
    assert!(snap.interactive.iter().any(|e| e.tag == "input"));
    let input = snap
        .interactive
        .iter()
        .find(|e| e.tag == "input")
        .unwrap()
        .id;
    // 输入（真实键盘）
    sdk.tool_call(
        "type",
        serde_json::json!({"id": input.to_string(), "text": "headed"}),
    )
    .unwrap();
    let v = sdk
        .tool_call(
            "execute_js",
            serde_json::json!({"script": "document.getElementById('q').value"}),
        )
        .unwrap();
    assert_eq!(v["result"], "headed");
    // 点击（真实鼠标）
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
    // 提取
    let text = sdk
        .tool_call("get_page_text", serde_json::json!({}))
        .unwrap();
    assert!(text["text"].as_str().unwrap().contains("headed world"));
    let links = sdk
        .tool_call("extract_links", serde_json::json!({}))
        .unwrap();
    assert!(links["links"].as_array().unwrap().iter().any(|l| l["url"]
        .as_str()
        .map(|u| u.contains("#next"))
        .unwrap_or(false)));
    // 截图（真实渲染）
    let img = sdk.screenshot().unwrap();
    assert!(is_nonblank(&img));
    // 多标签
    let t2 = sdk
        .tool_call("new_tab", serde_json::json!({"url": "about:blank"}))
        .unwrap();
    let t2id = t2["tab"].as_u64().unwrap() as u32;
    assert!(
        sdk.tool_call("list_tabs", serde_json::json!({})).unwrap()["tabs"]
            .as_array()
            .unwrap()
            .len()
            >= 2
    );
    sdk.tool_call("switch_tab", serde_json::json!({"tab": t2id}))
        .unwrap();
    assert_eq!(
        sdk.runtime()
            .unwrap()
            .as_ref()
            .unwrap()
            .engine()
            .active_tab()
            .unwrap()
            .as_u32(),
        t2id
    );
    sdk.tool_call("close_tab", serde_json::json!({"tab": t2id}))
        .unwrap();

    sdk.shutdown();
    eprintln!("PASS: headed_basic_agent_flow");
}

/// ② 有头窗口真实渲染：页面有纯色块 → 截图能取到对应像素；
/// 且前台标签页 `document.hidden === false`（无头下没有此语义）。
#[test]
fn headed_window_renders_real_content() {
    let Some((_guard, ws)) = setup_headed() else {
        eprintln!("SKIP: headed_window_renders_real_content (no headed chrome)");
        return;
    };
    let (_server, url) = serve_http(
        r##"<!doctype html><html><head><title>Color</title><style>
          html,body{margin:0;padding:0}
          #red{position:fixed;left:0;top:0;width:100%;height:100%;background:#ff2020}
        </style></head><body><div id="red"></div></body></html>"##,
    );
    let sdk = new_sdk(&ws);
    sdk.open(&url).unwrap();
    wait_until("page loaded", Duration::from_secs(10), || {
        sdk.snapshot().map(|s| s.title == "Color").unwrap_or(false)
    });
    sdk.set_viewport(320, 240).unwrap();

    // 前台标签页在 headed 下对用户可见（轮询，避免可见性状态异步）
    wait_until("tab visible", Duration::from_secs(5), || {
        sdk.tool_call(
            "execute_js",
            serde_json::json!({"script": "document.hidden"}),
        )
        .map(|v| v["result"] == false)
        .unwrap_or(false)
    });

    // 截图像素：应出现目标红色 #ff2020（真实合成器输出）
    let img = sdk.screenshot().unwrap();
    assert!(is_nonblank(&img));
    let has_red = img
        .rgba
        .chunks_exact(4)
        .take(16384)
        .any(|c| c[0] > 180 && c[1] < 90 && c[2] < 90);
    assert!(
        has_red,
        "headed screenshot must contain the red fill (#ff2020)"
    );
    sdk.shutdown();
    eprintln!("PASS: headed_window_renders_real_content");
}

/// ③ 有头多标签：切换后前台隐藏/后台渲染语义；后台标签仍可截图（有头不冻结）。
#[test]
fn headed_tabs_visibility_and_background_render() {
    let Some((_guard, ws)) = setup_headed() else {
        eprintln!("SKIP: headed_tabs_visibility_and_background_render (no headed chrome)");
        return;
    };
    let (_server, url_a) = serve_http_multi(vec![
        (
            "/",
            r##"<!doctype html><html><head><title>TabA</title></head><body><div id="a" style="position:fixed;inset:0;background:#00aaff"></div></body></html>"##,
        ),
        (
            "/two",
            r##"<!doctype html><html><head><title>TabB</title></head><body><div id="b" style="position:fixed;inset:0;background:#00cc66"></div></body></html>"##,
        ),
    ]);
    let sdk = new_sdk(&ws);
    let t_a = sdk.open(&url_a).unwrap()["tab"].as_u64().unwrap() as u32;
    let t_a = fastbrowser::engine::TabId(t_a);
    wait_until("tabA loaded", Duration::from_secs(10), || {
        sdk.runtime()
            .unwrap()
            .as_ref()
            .unwrap()
            .engine()
            .snapshot(t_a)
            .map(|s| s.title == "TabA")
            .unwrap_or(false)
    });
    // 开第二个标签（同窗口新标签）
    let t_b = sdk
        .tool_call("new_tab", serde_json::json!({"url": format!("{url_a}two")}))
        .unwrap()["tab"]
        .as_u64()
        .unwrap() as u32;
    let t_b = fastbrowser::engine::TabId(t_b);
    wait_until("tabB loaded", Duration::from_secs(10), || {
        sdk.runtime()
            .unwrap()
            .as_ref()
            .unwrap()
            .engine()
            .snapshot(t_b)
            .map(|s| s.title == "TabB")
            .unwrap_or(false)
    });

    // B 前台 → B 可见、A 后台隐藏（Chrome 可见性状态异步更新，轮询判定）
    let hidden_of = |sdk: &Fastbrowser, t: fastbrowser::engine::TabId| -> Option<bool> {
        sdk.runtime()
            .ok()?
            .as_ref()?
            .engine()
            .evaluate(t, "document.hidden")
            .ok()
            .and_then(|v| v.as_bool())
    };
    wait_until("B becomes visible", Duration::from_secs(5), || {
        hidden_of(&sdk, t_b) == Some(false)
    });
    wait_until("A becomes hidden", Duration::from_secs(5), || {
        hidden_of(&sdk, t_a) == Some(true)
    });

    // 后台标签仍可截图（有头合成器不冻结）
    let img_a = shot(&sdk, t_a);
    assert!(
        is_nonblank(&img_a),
        "background tab A screenshot should still render in headed mode"
    );
    // 切回 A → A 前台可见
    sdk.tool_call("switch_tab", serde_json::json!({"tab": t_a.as_u32()}))
        .unwrap();
    wait_until("A becomes visible", Duration::from_secs(5), || {
        hidden_of(&sdk, t_a) == Some(false)
    });
    sdk.shutdown();
    eprintln!("PASS: headed_tabs_visibility_and_background_render");
}

/// ④ 有头 Actionability + 真实键盘（与无头同语义，但走真实窗口输入管线）。
#[test]
fn headed_actionability_and_real_keyboard() {
    let Some((_guard, ws)) = setup_headed() else {
        eprintln!("SKIP: headed_actionability_and_real_keyboard (no headed chrome)");
        return;
    };
    let (_server, url) = serve_http(
        r##"<!doctype html><html><head><title>HeadAct</title></head><body>
          <input id="q">
          <button id="pulse">Pulse</button>
          <script>
            window.beforeInputs = [];
            var q = document.getElementById('q');
            q.addEventListener('beforeinput', function(e){ window.beforeInputs.push(e.inputType); });
            q.addEventListener('input', function(){ window.committed = q.value; });
            var b = document.getElementById('pulse');
            b.onclick = function(){ document.body.dataset.pulsed='yes'; };
            window.startPulse = function(){
              b.style.display = 'none';
              setTimeout(function(){ b.style.display = 'block'; }, 300);
            };
          </script>
        </body></html>"##,
    );
    let sdk = new_sdk(&ws);
    sdk.open(&url).unwrap();
    wait_until("page loaded", Duration::from_secs(10), || {
        sdk.snapshot()
            .map(|s| s.title == "HeadAct")
            .unwrap_or(false)
    });

    // 真实键盘：Input.insertText 触发 beforeinput(insertText)
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
        serde_json::json!({"id": input.to_string(), "text": "h1"}),
    )
    .unwrap();
    let st = sdk.tool_call("execute_js", serde_json::json!({"script": "JSON.stringify({v:window.committed,b:window.beforeInputs})"})).unwrap();
    let st: Value = serde_json::from_str(st["result"].as_str().unwrap()).unwrap();
    assert_eq!(st["v"], "h1");
    assert!(st["b"].as_array().unwrap().iter().any(|t| t
        .as_str()
        .map(|s| s.contains("insertText"))
        .unwrap_or(false)));

    // Actionability：元素被隐藏 300ms 后点击自动等待成功
    let btn = sdk
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
    sdk.tool_call("click", serde_json::json!({"id": btn.to_string()}))
        .unwrap();
    let pulsed = sdk
        .tool_call(
            "execute_js",
            serde_json::json!({"script": "document.body.dataset.pulsed || ''"}),
        )
        .unwrap();
    assert_eq!(pulsed["result"], "yes");
    sdk.shutdown();
    eprintln!("PASS: headed_actionability_and_real_keyboard");
}

/// ⑤ 有头下载落盘（真实窗口的下载链路）。
#[test]
fn headed_download_writes_file() {
    let Some((_guard, ws)) = setup_headed() else {
        eprintln!("SKIP: headed_download_writes_file (no headed chrome)");
        return;
    };
    let (_server, base) = serve_download();
    let dl = tempfile::tempdir().unwrap();
    let sdk = Fastbrowser::new();
    let cfg = Config {
        engine: "chromium".into(),
        cdp_url: Some(ws),
        accept_downloads: true,
        default_download_path: Some(dl.path().to_str().unwrap().to_string()),
        ..Config::default()
    };
    sdk.init(cfg).unwrap();
    sdk.open(&base).unwrap();
    sdk.navigate(&format!("{base}file.txt")).unwrap();
    let target = dl.path().join("hello.txt");
    wait_until("download file appears", Duration::from_secs(15), || {
        target.exists()
    });
    assert_eq!(
        std::fs::read_to_string(&target).unwrap().trim(),
        "HELLO-DOWNLOAD"
    );
    sdk.shutdown();
    eprintln!("PASS: headed_download_writes_file");
}

/// ⑥ 有头上下文隔离（Profile → 独立 BrowserContext，newWindow 路径）。
#[test]
fn headed_context_isolation() {
    let Some((_guard, ws)) = setup_headed() else {
        eprintln!("SKIP: headed_context_isolation (no headed chrome)");
        return;
    };
    let (_server, url) = serve_http(
        r##"<!doctype html><html><head><title>Iso</title></head><body><h1>iso</h1></body></html>"##,
    );
    let sdk = Fastbrowser::new();
    let cfg = Config {
        engine: "chromium".into(),
        cdp_url: Some(ws.clone()),
        isolated_profiles: true,
        ..Config::default()
    };
    sdk.init(cfg).unwrap();

    // 默认 profile 打开
    let t1 = sdk.open(&url).unwrap()["tab"].as_u64().unwrap() as u32;
    wait_until("page loaded", Duration::from_secs(10), || {
        sdk.tool_call("get_page_title", serde_json::json!({}))
            .map(|v| v["title"] == "Iso")
            .unwrap_or(false)
    });
    // 新 profile：独立上下文（newWindow 创建，headed 下自然前台）
    let created = sdk
        .runtime()
        .unwrap()
        .as_ref()
        .unwrap()
        .session()
        .create_profile("work", "chromium", false, None, None, None);
    sdk.runtime()
        .unwrap()
        .as_ref()
        .unwrap()
        .session()
        .set_active_profile(created)
        .unwrap();
    let t2 = sdk.open(&url).unwrap()["tab"].as_u64().unwrap() as u32;
    assert!(t2 != t1, "isolated context must create a distinct tab");

    // 两个上下文各自可截图/操作（互不干扰）
    let tab1 = fastbrowser::engine::TabId(t1);
    let tab2 = fastbrowser::engine::TabId(t2);
    wait_until("t2 loaded", Duration::from_secs(10), || {
        sdk.runtime()
            .unwrap()
            .as_ref()
            .unwrap()
            .engine()
            .snapshot(tab2)
            .map(|s| s.title == "Iso")
            .unwrap_or(false)
    });
    assert!(is_nonblank(&shot(&sdk, tab1)));
    assert!(is_nonblank(&shot(&sdk, tab2)));
    sdk.shutdown();
    eprintln!("PASS: headed_context_isolation");
}

/// ⑦ 有头 OSR 帧流（start_frame_stream → frame_sink）。
#[test]
fn headed_frame_stream_pushes_frames() {
    let Some((_guard, ws)) = setup_headed() else {
        eprintln!("SKIP: headed_frame_stream_pushes_frames (no headed chrome)");
        return;
    };
    let (_server, url) = serve_http(
        r##"<!doctype html><html><head><title>StreamH</title></head><body><h1>stream</h1>
        <script>
          var i=0; setInterval(function(){
            document.body.style.background = (i++%2)?'#fff':'#eee';
          }, 120);
        </script>
        </body></html>"##,
    );
    use std::sync::atomic::{AtomicUsize, Ordering};
    let count = Arc::new(AtomicUsize::new(0));
    struct Sink(Arc<AtomicUsize>);
    impl fastbrowser::engine::host::ViewFrameSink for Sink {
        fn on_view_frame(
            &self,
            _t: fastbrowser::engine::TabId,
            _f: &fastbrowser::engine::ViewFrame,
        ) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }
    let sink: Arc<dyn fastbrowser::engine::host::ViewFrameSink> = Arc::new(Sink(count.clone()));

    let sdk = Fastbrowser::new();
    let cfg = Config {
        engine: "chromium".into(),
        cdp_url: Some(ws.clone()),
        ..Config::default()
    };
    sdk.init(cfg).unwrap();
    sdk.register_frame_sink(sink);
    let tab = fastbrowser::engine::TabId(sdk.open(&url).unwrap()["tab"].as_u64().unwrap() as u32);
    wait_until("page loaded", Duration::from_secs(10), || {
        sdk.snapshot()
            .map(|s| s.title == "StreamH")
            .unwrap_or(false)
    });
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
    std::thread::sleep(Duration::from_millis(150));
    let a1 = count.load(Ordering::Relaxed);
    std::thread::sleep(Duration::from_millis(120));
    assert_eq!(
        a1,
        count.load(Ordering::Relaxed),
        "frame count must freeze after stop"
    );
    sdk.shutdown();
    eprintln!("PASS: headed_frame_stream_pushes_frames");
}

/// ⑧ 有头编码截图（png/jpeg 字节直出）+ 视口。
#[test]
fn headed_encoded_screenshot_and_viewport() {
    let Some((_guard, ws)) = setup_headed() else {
        eprintln!("SKIP: headed_encoded_screenshot_and_viewport (no headed chrome)");
        return;
    };
    let (_server, url) = serve_http(
        r##"<!doctype html><html><head><title>EncH</title></head><body><h1>enc</h1></body></html>"##,
    );
    let sdk = new_sdk(&ws);
    sdk.open(&url).unwrap();
    sdk.set_viewport(400, 300).unwrap();
    wait_until("page loaded", Duration::from_secs(10), || {
        sdk.snapshot().map(|s| s.title == "EncH").unwrap_or(false)
    });
    let rgba = sdk.tool_call("screenshot", serde_json::json!({})).unwrap();
    assert_eq!(rgba["format"], "rgba");
    assert_eq!(rgba["width"], 400);
    assert_eq!(rgba["height"], 300);
    let jpg = sdk
        .tool_call("screenshot", serde_json::json!({"format": "jpeg"}))
        .unwrap();
    assert_eq!(jpg["format"], "jpeg");
    use base64::Engine as _;
    let jb = base64::engine::general_purpose::STANDARD
        .decode(jpg["base64"].as_str().unwrap())
        .unwrap();
    assert_eq!(&jb[..2], &[0xFF, 0xD8], "JPEG SOI");
    sdk.shutdown();
    eprintln!("PASS: headed_encoded_screenshot_and_viewport");
}

/// ⑨ 有头历史导航（back/forward 经 getNavigationHistory）。
#[test]
fn headed_back_forward_history() {
    let Some((_guard, ws)) = setup_headed() else {
        eprintln!("SKIP: headed_back_forward_history (no headed chrome)");
        return;
    };
    let (_server, base) = serve_http_multi(vec![
        (
            "/",
            r##"<!doctype html><html><head><title>One</title></head><body>one</body></html>"##,
        ),
        (
            "/two",
            r##"<!doctype html><html><head><title>Two</title></head><body>two</body></html>"##,
        ),
        (
            "/three",
            r##"<!doctype html><html><head><title>Three</title></head><body>three</body></html>"##,
        ),
    ]);
    let sdk = new_sdk(&ws);
    sdk.open(&base).unwrap();
    wait_until("one loaded", Duration::from_secs(10), || {
        sdk.tool_call("get_page_title", serde_json::json!({}))
            .map(|v| v["title"] == "One")
            .unwrap_or(false)
    });
    sdk.navigate(&format!("{base}two")).unwrap();
    sdk.navigate(&format!("{base}three")).unwrap();
    wait_until("three loaded", Duration::from_secs(10), || {
        sdk.tool_call("get_page_title", serde_json::json!({}))
            .map(|v| v["title"] == "Three")
            .unwrap_or(false)
    });
    sdk.tool_call("back", serde_json::json!({})).unwrap();
    wait_until("back to two", Duration::from_secs(10), || {
        sdk.tool_call("get_page_title", serde_json::json!({}))
            .map(|v| v["title"] == "Two")
            .unwrap_or(false)
    });
    sdk.tool_call("forward", serde_json::json!({})).unwrap();
    wait_until("forward to three", Duration::from_secs(10), || {
        sdk.tool_call("get_page_title", serde_json::json!({}))
            .map(|v| v["title"] == "Three")
            .unwrap_or(false)
    });
    sdk.shutdown();
    eprintln!("PASS: headed_back_forward_history");
}
