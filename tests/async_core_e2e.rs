// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! 异步壳 + 真实 Chromium 集成测试（feature `engine-cdp` + `async-core`）。
//!
//! 验证「异步核心 + 异步壳」在真实浏览器上的行为：async 工具调用不阻塞执行器、
//! 事件推送、多标签并发编排（`run_concurrently` / `open_many` / `wait_any`）、
//! 异步导航等待、真实键盘/点击。

#![cfg(feature = "engine-cdp")]
#![cfg(feature = "async-core")]

mod common;

use std::io::{Read, Write};
use std::sync::Arc;
use std::time::Duration;

use fastbrowser::async_core::AsyncFastbrowser;
use fastbrowser::engine::PageEvent;
use fastbrowser::sdk::Fastbrowser;
use fastbrowser::Config;
use serde_json::json;

fn setup() -> (common::BrowserGuard, String) {
    let b = common::shared_browser(true).expect("launch headless chrome");
    let guard = common::browser_guard(b);
    let ws = b.ws.clone();
    (guard, ws)
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

async fn new_shell(ws: &str) -> AsyncFastbrowser {
    let fb = Arc::new(Fastbrowser::new());
    let s = AsyncFastbrowser::spawn(fb);
    let cfg = Config {
        engine: "chromium".into(),
        cdp_url: Some(ws.to_string()),
        ..Config::default()
    };
    s.init(cfg).await.unwrap();
    s
}

/// ① 异步壳 + 真实浏览器全链路：open → 等待加载 → 快照 → 输入 → 点击 → 提取。
#[tokio::test]
async fn async_basic_flow_on_chromium() {
    let Some(_) = common::find_chrome() else {
        eprintln!("SKIP: async_basic_flow_on_chromium (no chrome)");
        return;
    };
    let (_guard, ws) = setup();
    let (_server, url) = serve_http_multi(vec![(
        "/",
        r##"<!doctype html><html><head><title>AsyncOne</title></head><body>
          <input id="q" placeholder="q">
          <button id="go">Go</button>
          <script>
            document.getElementById('go').onclick = function(){ document.body.dataset.ran='yes'; };
          </script>
        </body></html>"##,
    )]);

    let s = new_shell(&ws).await;
    let tab = s.open(url.clone()).await.unwrap()["tab"].as_u64().unwrap() as u32;
    let tab = fastbrowser::engine::TabId(tab);
    // 异步等待真实导航（url 变化）
    s.wait_for_navigation(tab, Duration::from_secs(10))
        .await
        .unwrap();
    // 等待加载完成（readyState）
    s.wait_for_load_state(tab, "load", Duration::from_secs(10))
        .await
        .unwrap();

    // 输入（真实键盘）
    let input = s
        .snapshot()
        .await
        .unwrap()
        .interactive
        .iter()
        .find(|e| e.tag == "input")
        .unwrap()
        .id;
    s.tool_call(
        "type".into(),
        json!({"id": input.to_string(), "text": "hello", "tab": tab.as_u32()}),
    )
    .await
    .unwrap();
    // 点击（真实鼠标 + actionability）
    let btn = s
        .snapshot()
        .await
        .unwrap()
        .interactive
        .iter()
        .find(|e| e.tag == "button")
        .unwrap()
        .id;
    s.tool_call(
        "click".into(),
        json!({"id": btn.to_string(), "tab": tab.as_u32()}),
    )
    .await
    .unwrap();
    let ran = s
        .tool_call(
            "execute_js".into(),
            json!({"script": "document.body.dataset.ran || ''", "tab": tab.as_u32()}),
        )
        .await
        .unwrap();
    assert_eq!(ran["result"], "yes");
    shutdown(&s).await;
}

async fn shutdown(s: &AsyncFastbrowser) {
    s.shutdown().await;
}

/// ② 多标签并发编排：`open_many` 并发开标签 + `run_concurrently` 跨标签并行操作。
#[tokio::test]
#[allow(non_snake_case)]
async fn async_parallel_multi_tab_on_chromium() {
    let Some(_) = common::find_chrome() else {
        eprintln!("SKIP: async_parallel_multi_tab_on_chromium (no chrome)");
        return;
    };
    let (_guard, ws) = setup();
    let (_server, url) = serve_http_multi(vec![
        (
            "/",
            r##"<!doctype html><html><head><title>A</title></head><body>
          <input id="a" placeholder="a"><button>btn</button></body></html>"##,
        ),
        (
            "/two",
            r##"<!doctype html><html><head><title>B</title></head><body>
          <input id="b" placeholder="b"><button>btn2</button></body></html>"##,
        ),
    ]);

    let s = new_shell(&ws).await;
    // 并发打开两个标签
    let opened = s
        .open_many(vec![url.clone(), format!("{url}two")])
        .await
        .unwrap();
    assert_eq!(opened.len(), 2);
    let tabs: Vec<u32> = opened
        .iter()
        .map(|v| v["tab"].as_u64().unwrap() as u32)
        .collect();
    assert!(
        tabs[0] != tabs[1],
        "concurrent opens must produce distinct tabs"
    );

    // 等待两个标签都加载
    for t in &tabs {
        let t = fastbrowser::engine::TabId(*t);
        s.wait_for_load_state(t, "load", Duration::from_secs(10))
            .await
            .unwrap();
    }

    let tA = fastbrowser::engine::TabId(tabs[0]);
    let tB = fastbrowser::engine::TabId(tabs[1]);
    // 跨标签并行操作：先对每个标签做 per-tab 快照拿 input 的 data-fb id
    let idA = s
        .snapshot_on(tA)
        .await
        .unwrap()
        .interactive
        .iter()
        .find(|e| e.tag == "input")
        .unwrap()
        .id;
    let idB = s
        .snapshot_on(tB)
        .await
        .unwrap()
        .interactive
        .iter()
        .find(|e| e.tag == "input")
        .unwrap()
        .id;

    // 并发：tab A 输入 alpha、tab B 输入 beta（真实键盘，跨标签并行）
    let results = s
        .run_concurrently(vec![
            (
                "type".into(),
                json!({"id": idA.to_string(), "text": "alpha", "tab": tabs[0]}),
            ),
            (
                "type".into(),
                json!({"id": idB.to_string(), "text": "beta", "tab": tabs[1]}),
            ),
        ])
        .await
        .unwrap();
    assert_eq!(results.len(), 2);

    // 各自生效
    let va = s
        .tool_call(
            "execute_js".into(),
            json!({"script": "document.querySelector('input').value", "tab": tabs[0]}),
        )
        .await
        .unwrap();
    assert_eq!(va["result"], "alpha");
    let vb = s
        .tool_call(
            "execute_js".into(),
            json!({"script": "document.querySelector('input').value", "tab": tabs[1]}),
        )
        .await
        .unwrap();
    assert_eq!(vb["result"], "beta");
    shutdown(&s).await;
}

/// ③ 事件推送：订阅 → console.log → wait_for_event 收到。
#[tokio::test]
async fn async_event_push_on_chromium() {
    let Some(_) = common::find_chrome() else {
        eprintln!("SKIP: async_event_push_on_chromium (no chrome)");
        return;
    };
    let (_guard, ws) = setup();
    let (_server, url) = serve_http_multi(vec![(
        "/",
        r##"<!doctype html><html><head><title>Ev</title></head><body>ev</body></html>"##,
    )]);

    let s = new_shell(&ws).await;
    let tab = s.open(url).await.unwrap()["tab"].as_u64().unwrap() as u32;
    let tab = fastbrowser::engine::TabId(tab);
    s.wait_for_load_state(tab, "load", Duration::from_secs(10))
        .await
        .unwrap();

    // 订阅 + 触发 console
    let mut rx = s.subscribe_events();
    s.tool_call(
        "execute_js".into(),
        json!({"script": "console.log('async-marker')", "tab": tab.as_u32()}),
    )
    .await
    .unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    let mut found = false;
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(100), rx.recv()).await {
            Ok(Ok((t, PageEvent::Console { message, .. })))
                if t == tab && message.contains("async-marker") =>
            {
                found = true;
                break;
            }
            _ => {}
        }
    }
    assert!(found, "console event must be pushed via async event stream");
    shutdown(&s).await;
}

/// ④ wait_any：两个标签各自导航，任意一个先完成即返回。
#[tokio::test]
async fn async_wait_any_on_chromium() {
    let Some(_) = common::find_chrome() else {
        eprintln!("SKIP: async_wait_any_on_chromium (no chrome)");
        return;
    };
    let (_guard, ws) = setup();
    let (_server, url) = serve_http_multi(vec![
        (
            "/",
            r##"<!doctype html><html><head><title>W1</title></head><body>w1</body></html>"##,
        ),
        (
            "/two",
            r##"<!doctype html><html><head><title>W2</title></head><body>w2</body></html>"##,
        ),
        (
            "/three",
            r##"<!doctype html><html><head><title>W3</title></head><body>w3</body></html>"##,
        ),
    ]);

    let s = new_shell(&ws).await;
    let t1 = s.open(url.clone()).await.unwrap()["tab"].as_u64().unwrap() as u32;
    let t2 = s.open(format!("{url}two")).await.unwrap()["tab"]
        .as_u64()
        .unwrap() as u32;
    let t1 = fastbrowser::engine::TabId(t1);
    let t2 = fastbrowser::engine::TabId(t2);

    // 订阅后触发两路导航，wait_any 应命中其一
    let mut rx = s.subscribe_events();
    s.navigate(format!("{url}three")).await.unwrap();
    // 用事件等待：任一 tab 产生 NavigationCompleted
    let got = fastbrowser::async_core::orchestrate::wait_any(
        &mut rx,
        &[t1, t2],
        Duration::from_secs(10),
        |e| matches!(e, PageEvent::NavigationCompleted { .. }),
    )
    .await
    .unwrap();
    assert!(
        got == t1 || got == t2,
        "wait_any must return one of the target tabs"
    );
    shutdown(&s).await;
}

/// ⑤ 异步等待真实加载状态 + JS 谓词。
#[tokio::test]
async fn async_wait_state_and_condition_on_chromium() {
    let Some(_) = common::find_chrome() else {
        eprintln!("SKIP: async_wait_state_and_condition_on_chromium (no chrome)");
        return;
    };
    let (_guard, ws) = setup();
    let (_server, url) = serve_http_multi(vec![(
        "/",
        r##"<!doctype html><html><head><title>St</title></head><body>
          <a href="#">link</a><script>document.body.dataset.ready='1';</script>
        </body></html>"##,
    )]);

    let s = new_shell(&ws).await;
    let tab = s.open(url).await.unwrap()["tab"].as_u64().unwrap() as u32;
    let tab = fastbrowser::engine::TabId(tab);
    s.wait_for_load_state(tab, "load", Duration::from_secs(10))
        .await
        .unwrap();
    s.wait_for_condition(
        tab,
        "document.querySelectorAll('a').length >= 1".into(),
        Duration::from_secs(10),
    )
    .await
    .unwrap();
    shutdown(&s).await;
}

/// ⑥ 4 标签并发编排：open_many 开 4 个标签，跨标签并行输入各自生效。
#[tokio::test]
#[allow(non_snake_case)]
async fn async_parallel_four_tabs_on_chromium() {
    let Some(_) = common::find_chrome() else {
        eprintln!("SKIP: async_parallel_four_tabs_on_chromium (no chrome)");
        return;
    };
    let (_guard, ws) = setup();
    let leak = |s: String| -> &'static str { Box::leak(s.into_boxed_str()) };
    let routes: Vec<(&'static str, &'static str)> = (0..4)
        .map(|i| {
            let html = leak(format!(
                r##"<!doctype html><html><head><title>P{i}</title></head><body>
                <input id="in"><button>b</button></body></html>"##
            ));
            let path = if i == 0 { "/" } else { leak(format!("/p{i}")) };
            (path, html)
        })
        .collect();
    let (_server, base) = serve_http_multi(routes);

    let s = new_shell(&ws).await;
    let urls: Vec<String> = (0..4)
        .map(|i| {
            if i == 0 {
                base.clone()
            } else {
                format!("{base}p{i}")
            }
        })
        .collect();
    let opened = s.open_many(urls).await.unwrap();
    assert_eq!(opened.len(), 4);
    let tabs: Vec<fastbrowser::engine::TabId> = opened
        .iter()
        .map(|v| fastbrowser::engine::TabId(v["tab"].as_u64().unwrap() as u32))
        .collect();

    // 全部加载完成
    for t in &tabs {
        s.wait_for_load_state(*t, "load", Duration::from_secs(10))
            .await
            .unwrap();
    }

    // per-tab 快照拿 input id，然后跨标签并行输入
    let mut tasks = Vec::new();
    for (idx, t) in tabs.iter().enumerate() {
        let id = s
            .snapshot_on(*t)
            .await
            .unwrap()
            .interactive
            .iter()
            .find(|e| e.tag == "input")
            .unwrap()
            .id;
        tasks.push((
            "type".into(),
            json!({"id": id.to_string(), "text": format!("v{idx}"), "tab": t.as_u32()}),
        ));
    }
    let res = s.run_concurrently(tasks).await.unwrap();
    assert_eq!(res.len(), 4);

    for (idx, t) in tabs.iter().enumerate() {
        let v = s
            .tool_call(
                "execute_js".into(),
                json!({"script": "document.getElementById('in').value", "tab": t.as_u32()}),
            )
            .await
            .unwrap();
        assert_eq!(v["result"], format!("v{idx}"), "tab {idx} value mismatch");
    }
    shutdown(&s).await;
}

/// ⑦ 事件推送：订阅后触发 fetch，wait_for_event 收到网络请求事件。
#[tokio::test]
async fn async_event_network_push_on_chromium() {
    let Some(_) = common::find_chrome() else {
        eprintln!("SKIP: async_event_network_push_on_chromium (no chrome)");
        return;
    };
    let (_guard, ws) = setup();
    let (_server, url) = serve_http_multi(vec![(
        "/",
        r##"<!doctype html><html><head><title>NetEv</title></head><body>
          <button id="go">go</button>
          <script>
            document.getElementById('go').onclick = function(){
              fetch('/data').then(function(r){return r.text();}).then(function(){});
            };
          </script>
        </body></html>"##,
    )]);

    let s = new_shell(&ws).await;
    let tab = s.open(url).await.unwrap()["tab"].as_u64().unwrap() as u32;
    let tab = fastbrowser::engine::TabId(tab);
    s.wait_for_load_state(tab, "load", Duration::from_secs(10))
        .await
        .unwrap();

    // 订阅后触发一次网络请求
    let mut rx = s.subscribe_events();
    let btn = s
        .snapshot_on(tab)
        .await
        .unwrap()
        .interactive
        .iter()
        .find(|e| e.tag == "button")
        .unwrap()
        .id;
    s.tool_call(
        "click".into(),
        json!({"id": btn.to_string(), "tab": tab.as_u32()}),
    )
    .await
    .unwrap();

    let got = fastbrowser::async_core::orchestrate::wait_for_event(
        &mut rx,
        Some(tab),
        Duration::from_secs(10),
        |e| {
            matches!(e, PageEvent::Request { url, .. } if url.contains("/data"))
                || matches!(e, PageEvent::Response { url, .. } if url.contains("/data"))
        },
    )
    .await;
    assert!(got.is_ok(), "network event must be pushed: {:?}", got);
    shutdown(&s).await;
}

/// ⑧ wait_any 超时：无匹配事件 → 超时错误。
#[tokio::test]
async fn async_wait_any_timeout_on_chromium() {
    let Some(_) = common::find_chrome() else {
        eprintln!("SKIP: async_wait_any_timeout_on_chromium (no chrome)");
        return;
    };
    let (_guard, ws) = setup();
    let (_server, url) = serve_http_multi(vec![(
        "/",
        r##"<!doctype html><html><head><title>Wt</title></head><body>wt</body></html>"##,
    )]);

    let s = new_shell(&ws).await;
    let t1 = s.open(url.clone()).await.unwrap()["tab"].as_u64().unwrap() as u32;
    let t2 = s.open(url).await.unwrap()["tab"].as_u64().unwrap() as u32;
    let mut rx = s.subscribe_events();
    // 等待 Dialog 事件（页面不会产生）→ 超时
    let res = fastbrowser::async_core::orchestrate::wait_any(
        &mut rx,
        &[
            fastbrowser::engine::TabId(t1),
            fastbrowser::engine::TabId(t2),
        ],
        Duration::from_millis(300),
        |e| matches!(e, PageEvent::Dialog { .. }),
    )
    .await;
    assert!(res.is_err(), "must time out when no matching event");
    shutdown(&s).await;
}

/// ⑨ 每标签截图：截图 on 两个标签，均非空白。
#[tokio::test]
async fn async_screenshot_on_per_tab_chromium() {
    let Some(_) = common::find_chrome() else {
        eprintln!("SKIP: async_screenshot_on_per_tab_chromium (no chrome)");
        return;
    };
    let (_guard, ws) = setup();
    let (_server, url) = serve_http_multi(vec![
        (
            "/",
            r##"<!doctype html><html><head><title>S1</title></head><body>
          <div style="position:fixed;inset:0;background:#ff0000"></div></body></html>"##,
        ),
        (
            "/two",
            r##"<!doctype html><html><head><title>S2</title></head><body>
          <div style="position:fixed;inset:0;background:#00ff00"></div></body></html>"##,
        ),
    ]);

    let s = new_shell(&ws).await;
    let t1 = fastbrowser::engine::TabId(
        s.open(url.clone()).await.unwrap()["tab"].as_u64().unwrap() as u32,
    );
    let t2 = fastbrowser::engine::TabId(
        s.open(format!("{url}two")).await.unwrap()["tab"]
            .as_u64()
            .unwrap() as u32,
    );
    for t in [t1, t2] {
        s.wait_for_load_state(t, "load", Duration::from_secs(10))
            .await
            .unwrap();
    }
    s.set_viewport(320, 240).await.unwrap();
    let img1 = s.screenshot_on(t1).await.unwrap();
    let img2 = s.screenshot_on(t2).await.unwrap();
    assert!(img1.is_valid() && img1.width > 0);
    assert!(img2.is_valid() && img2.width > 0);
    let red = img1
        .rgba
        .chunks_exact(4)
        .take(8192)
        .any(|c| c[0] > 180 && c[1] < 90 && c[2] < 90);
    let green = img2
        .rgba
        .chunks_exact(4)
        .take(8192)
        .any(|c| c[1] > 180 && c[0] < 90 && c[2] < 90);
    assert!(red, "tab1 screenshot should be red");
    assert!(green, "tab2 screenshot should be green");
    shutdown(&s).await;
}

/// ⑩ 异步对话框：页面 setTimeout alert → pending → accept → cleared。
#[tokio::test]
async fn async_dialog_accept_on_chromium() {
    let Some(_) = common::find_chrome() else {
        eprintln!("SKIP: async_dialog_accept_on_chromium (no chrome)");
        return;
    };
    let (_guard, ws) = setup();
    let (_server, url) = serve_http_multi(vec![(
        "/",
        r##"<!doctype html><html><head><title>Dlg</title></head><body>
          <script>setTimeout(function(){ alert('hello async'); }, 300);</script>
          <p>body</p>
        </body></html>"##,
    )]);

    let s = new_shell(&ws).await;
    let tab =
        fastbrowser::engine::TabId(s.open(url).await.unwrap()["tab"].as_u64().unwrap() as u32);
    // 轮询 pending_dialog（异步）
    let mut found = false;
    for _ in 0..30 {
        if let Ok(v) = s
            .tool_call("pending_dialog".into(), json!({"tab": tab.as_u32()}))
            .await
        {
            if v["dialog"].is_object() {
                assert_eq!(v["dialog"]["message"], "hello async");
                found = true;
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(found, "dialog was not captured");
    s.tool_call("dialog_accept".into(), json!({"tab": tab.as_u32()}))
        .await
        .unwrap();
    // 接受后 pending 应为空
    let cleared = fastbrowser::async_core::orchestrate::wait_until_poll(
        || {
            let s = &s;
            async move {
                s.tool_call("pending_dialog".into(), json!({"tab": tab.as_u32()}))
                    .await
                    .map(|v| v["dialog"].is_null())
                    .unwrap_or(false)
            }
        },
        Duration::from_secs(5),
        "dialog cleared",
    )
    .await;
    assert!(cleared.is_ok());
    shutdown(&s).await;
}

/// ⑪ 双壳共存（真实 Chrome）：异步操作 + 同步操作同一实例交替。
#[tokio::test]
async fn async_sync_shell_coexistence_chromium() {
    let Some(_) = common::find_chrome() else {
        eprintln!("SKIP: async_sync_shell_coexistence_chromium (no chrome)");
        return;
    };
    let (_guard, ws) = setup();
    let (_server, url) = serve_http_multi(vec![(
        "/",
        r##"<!doctype html><html><head><title>Co</title></head><body>
          <input id="q"><button>b</button></body></html>"##,
    )]);

    let s = new_shell(&ws).await;
    let tab =
        fastbrowser::engine::TabId(s.open(url).await.unwrap()["tab"].as_u64().unwrap() as u32);
    s.wait_for_load_state(tab, "load", Duration::from_secs(10))
        .await
        .unwrap();

    // 异步壳操作
    let v = s
        .tool_call("get_page_title".into(), json!({}))
        .await
        .unwrap();
    assert_eq!(v["title"], "Co");
    // 同步壳操作同一实例：必须经 spawn_blocking（同步 block_on 不能直接在 tokio 线程上跑）
    let inner = s.inner().clone();
    let v2 = tokio::task::spawn_blocking(move || {
        inner.tool_call("get_current_url", serde_json::json!({}))
    })
    .await
    .unwrap()
    .unwrap();
    assert!(v2["url"].as_str().unwrap().contains("127.0.0.1"));
    // 再切回异步
    let v3 = s
        .tool_call(
            "execute_js".into(),
            json!({"script": "document.title", "tab": tab.as_u32()}),
        )
        .await
        .unwrap();
    assert_eq!(v3["result"], "Co");
    shutdown(&s).await;
}

/// ⑫ 并发压力：3 标签 × 15 次并发导航/读取，全部成功。
#[tokio::test]
async fn async_stress_concurrent_calls_chromium() {
    let Some(_) = common::find_chrome() else {
        eprintln!("SKIP: async_stress_concurrent_calls_chromium (no chrome)");
        return;
    };
    let (_guard, ws) = setup();
    let (_server, url) = serve_http_multi(vec![
        (
            "/",
            r##"<!doctype html><html><head><title>Z1</title></head><body>z1</body></html>"##,
        ),
        (
            "/two",
            r##"<!doctype html><html><head><title>Z2</title></head><body>z2</body></html>"##,
        ),
        (
            "/three",
            r##"<!doctype html><html><head><title>Z3</title></head><body>z3</body></html>"##,
        ),
    ]);

    let s = new_shell(&ws).await;
    let opened = s
        .open_many(vec![
            url.clone(),
            format!("{url}two"),
            format!("{url}three"),
        ])
        .await
        .unwrap();
    let tabs: Vec<u32> = opened
        .iter()
        .map(|v| v["tab"].as_u64().unwrap() as u32)
        .collect();

    let mut tasks = Vec::new();
    for i in 0..15 {
        let tab = tabs[i % 3];
        tasks.push(("get_page_title".into(), json!({"tab": tab})));
        tasks.push(("get_current_url".into(), json!({"tab": tab})));
    }
    let res = s.run_concurrently(tasks).await.unwrap();
    assert_eq!(res.len(), 30);
    for r in &res {
        assert!(!r.is_null(), "each concurrent call must return a value");
    }
    shutdown(&s).await;
}

/// ⑬ 异步会话跨壳持久化：A 保存，B 加载恢复 cookie/storage。
#[tokio::test]
async fn async_session_roundtrip_two_shells_chromium() {
    let Some(_) = common::find_chrome() else {
        eprintln!("SKIP: async_session_roundtrip_two_shells_chromium (no chrome)");
        return;
    };
    let (_guard, ws) = setup();
    let (_server, url) = serve_http_multi(vec![(
        "/",
        r##"<!doctype html><html><head><title>Se</title></head><body>se</body></html>"##,
    )]);
    let state_path = tempfile::tempdir()
        .unwrap()
        .path()
        .join("state.json")
        .to_str()
        .unwrap()
        .to_string();

    // Shell A：写入并保存
    let s = new_shell(&ws).await;
    let tab = fastbrowser::engine::TabId(
        s.open(url.clone()).await.unwrap()["tab"].as_u64().unwrap() as u32,
    );
    s.wait_for_load_state(tab, "load", Duration::from_secs(10))
        .await
        .unwrap();
    let host = url.trim_end_matches('/').replace("http://", "");
    let host = host.split(':').next().unwrap_or(&host).to_string();
    s.tool_call(
        "cookie_set".into(),
        json!({"name": "async_cookie", "value": "jar", "domain": host}),
    )
    .await
    .unwrap();
    s.tool_call(
        "storage_set".into(),
        json!({"key": "async_token", "value": "tok", "tab": tab.as_u32()}),
    )
    .await
    .unwrap();
    s.session_save(state_path.clone()).await.unwrap();
    s.shutdown().await;

    // Shell B：加载并恢复
    let s2 = new_shell(&ws).await;
    s2.session_load(state_path).await.unwrap();
    let tab2 =
        fastbrowser::engine::TabId(s2.open(url).await.unwrap()["tab"].as_u64().unwrap() as u32);
    let c = s2
        .tool_call("cookie_get".into(), json!({"domain": host}))
        .await
        .unwrap();
    assert!(c["cookies"]
        .as_array()
        .unwrap()
        .iter()
        .any(|x| x["name"] == "async_cookie" && x["value"] == "jar"));
    let st = s2
        .tool_call(
            "storage_get".into(),
            json!({"key": "async_token", "tab": tab2.as_u32()}),
        )
        .await
        .unwrap();
    assert_eq!(st["value"], "tok");
    shutdown(&s2).await;
}
