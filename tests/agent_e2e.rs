// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Agent 任务端到端（真实浏览器，feature `engine-cdp`）：
//!
//! 在一个真实 HTTP 页面上跑「感知 → 行动 → 验证」闭环：
//! open → snapshot → fill_form → 真实坐标点击提交按钮 → 表单提交导航到
//! /dashboard → 提取+断言成功 → done。无浏览器时自动 SKIP。

#![cfg(feature = "engine-cdp")]

mod common;

use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::{Duration, Instant};

use fastbrowser::sdk::Fastbrowser;
use fastbrowser::Config;
use serde_json::json;

/// 两个真实路由：/ 登录页（表单提交到 /dashboard），/dashboard 成功页。
fn serve_app() -> (TcpListener, String) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = listener.try_clone().unwrap();
    std::thread::spawn(move || {
        for stream in server.incoming() {
            let Ok(mut stream) = stream else { break };
            let mut buf = [0u8; 2048];
            let _ = stream.read(&mut buf);
            let req = String::from_utf8_lossy(&buf);
            let path = req.split_whitespace().nth(1).unwrap_or("/");
            let (body, status) = if path.starts_with("/dashboard") {
                (
                    r#"<html><head><title>Dashboard</title></head><body><h1>Welcome back, alice</h1><p>login succeeded</p></body></html>"#,
                    "200 OK",
                )
            } else {
                (
                    r#"<!doctype html><html><head><title>Login</title></head><body>
                       <h1>Sign in</h1>
                       <form method="get" action="/dashboard">
                         <input name="username" placeholder="Username">
                         <input name="password" type="password" placeholder="Password">
                         <button type="submit">Login</button>
                       </form>
                     </body></html>"#,
                    "200 OK",
                )
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
fn agent_task_on_real_browser() {
    let b = match common::shared_browser(true) {
        Ok(b) => b,
        Err(_) => {
            eprintln!("SKIP: agent_task_on_real_browser (no chrome)");
            return;
        }
    };
    let _guard = common::browser_guard(b);
    let ws = b.ws.clone();

    let (_server, base) = serve_app();

    let sdk = Fastbrowser::new();
    let cfg = Config {
        engine: "chromium".into(),
        cdp_url: Some(ws),
        ..Config::default()
    };
    sdk.init(cfg).unwrap();

    // 1. 打开登录页
    sdk.open(&base).unwrap();
    wait_until("login page", Duration::from_secs(10), || {
        sdk.snapshot().map(|s| s.title == "Login").unwrap_or(false)
    });

    // 2. 感知：快照拿到表单控件
    let snap = sdk.snapshot().unwrap();
    let username = snap
        .interactive
        .iter()
        .find(|e| {
            e.tag == "input"
                && e.attrs
                    .get("placeholder")
                    .map(|p| p == "Username")
                    .unwrap_or(false)
        })
        .unwrap()
        .id;
    let password = snap
        .interactive
        .iter()
        .find(|e| {
            e.tag == "input"
                && e.attrs
                    .get("placeholder")
                    .map(|p| p == "Password")
                    .unwrap_or(false)
        })
        .unwrap()
        .id;
    let submit = snap
        .interactive
        .iter()
        .find(|e| e.tag == "button" && e.text.as_deref() == Some("Login"))
        .unwrap()
        .id;

    // 3. 行动：填表 + 真实点击提交（坐标级 Input.dispatchMouseEvent → 表单提交）
    sdk.tool_call("type", json!({"id": username.to_string(), "text": "alice"}))
        .unwrap();
    sdk.tool_call(
        "type",
        json!({"id": password.to_string(), "text": "secret"}),
    )
    .unwrap();
    sdk.tool_call("click", json!({"id": submit.to_string()}))
        .unwrap();

    // 4. 验证：导航到 /dashboard 并断言成功
    wait_until("dashboard", Duration::from_secs(30), || {
        sdk.snapshot()
            .map(|s| s.title == "Dashboard")
            .unwrap_or(false)
    });
    let text = sdk.tool_call("get_page_text", json!({})).unwrap();
    assert!(text["text"]
        .as_str()
        .unwrap()
        .contains("Welcome back, alice"));
    let url = sdk.tool_call("get_current_url", json!({})).unwrap();
    assert!(url["url"].as_str().unwrap().contains("dashboard"));
    sdk.tool_call("assert_text_contains", json!({"text": "login succeeded"}))
        .unwrap();

    // 5. 收尾：清状态 + done
    sdk.clear_state().unwrap();
    let done = sdk
        .tool_call("done", json!({"answer": "login flow end-to-end succeeded"}))
        .unwrap();
    assert_eq!(done["done"], true);

    sdk.shutdown();
    eprintln!("PASS: agent_task_on_real_browser");
}
