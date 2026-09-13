// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! 测试共享工具：进程级共享一个真实浏览器 + 进程内串行化。
//!
//! ## 为什么共享而非每测试 spawn
//! `cargo test` 并行运行各测试二进制、且每个二进制内测试也并行。若每个用例
//! 各自 `launch_chrome`，会同时存在十几个无头 Chrome，造成 CPU 与发热飙升，
//! 且多浏览器并发下事件/时序极易 flaky。
//!
//! 本模块让**每个测试二进制只启动一次 Chrome**（进程级单例），测试之间用
//! **进程内 Mutex 串行**，从根本上消除「并发 spawn」「文件锁 stale 误判」
//! 「reap 误杀并发测试」三类 flaky 根因。
//!
//! ## 进程清理
//! - 共享的 Chrome 子进程随测试进程退出而成为孤儿，由下一次测试进程启动时的
//!   `reap_orphan_test_chrome()` 收割（此刻无并发测试 Chrome，误杀风险为零）；
//! - 测试用 Chrome 一律使用带标记的 user-data-dir（`fastbrowser-test-chrome-*`）。

// 各测试二进制只使用本模块的一部分（按 feature 组合不同），故整体关闭死代码告警。
#![allow(dead_code)]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

/// 测试专用 Chrome 的 user-data-dir 标记（普通 Chrome 不会使用；与清理脚本一致）。
pub const TEST_CHROME_MARKER: &str = "fastbrowser-test-chrome";

static USER_DATA_SEQ: AtomicU64 = AtomicU64::new(0);

/// 在 `base` 下创建带标记的 user-data-dir（`fastbrowser-test-chrome-<pid>-<seq>`），
/// 供测试启动的 Chrome 使用——保证能被孤儿收割器精确识别。
pub fn marker_user_data_dir(base: &std::path::Path) -> PathBuf {
    let seq = USER_DATA_SEQ.fetch_add(1, Ordering::SeqCst);
    let p = base.join(format!("{TEST_CHROME_MARKER}-{}-{seq}", std::process::id()));
    let _ = std::fs::create_dir_all(&p);
    p
}

/// Build a `file://` URL from a local path, valid on both Unix and Windows:
/// forward slashes plus a leading slash (Windows `C:\x` → `file:///C:/x`,
/// Unix `/tmp/x` → `file:///tmp/x`). Chrome normalizes to this form, so tests
/// must compare against it (a raw `path.display()` gives `file://C:\x`).
pub fn file_url(p: &std::path::Path) -> String {
    let s = p.to_string_lossy().replace('\\', "/");
    let s = if s.starts_with('/') {
        s
    } else {
        format!("/{s}")
    };
    format!("file://{s}")
}

/// 收割测试遗留的孤儿 Chrome 进程（命令行含 `fastbrowser-test-chrome` 标记）。
/// 仅在**进程启动时**（`shared_browser` 首次初始化）调用一次——此时本进程尚无
/// 并发测试 Chrome，杀掉的一定是上次崩溃遗留的真孤儿，绝无误杀。
pub fn reap_orphan_test_chrome() {
    #[cfg(unix)]
    {
        let out = match Command::new("ps").args(["-axo", "pid=,command="]).output() {
            Ok(o) => o,
            Err(_) => return,
        };
        let text = String::from_utf8_lossy(&out.stdout);
        let mut pids = Vec::new();
        for line in text.lines() {
            if line.contains(TEST_CHROME_MARKER) {
                if let Some(pid) = line.split_whitespace().next() {
                    if let Ok(p) = pid.parse::<i32>() {
                        pids.push(p.to_string());
                    }
                }
            }
        }
        if pids.is_empty() {
            return;
        }
        eprintln!(
            "[common] reap {} orphan test Chrome pid(s): {}",
            pids.len(),
            pids.join(",")
        );
        let _ = Command::new("kill").arg("-9").args(&pids).status();
    }
    #[cfg(windows)]
    {
        // TODO: 用 `wmic process where "CommandLine like '%fastbrowser-test-chrome%'" get ProcessId`
        // 枚举后 taskkill /F；Windows 测试暂为空操作。
    }
}

/// 定位本机 Chrome/Chromium/Edge（可用 `CHROME_PATH` 环境变量覆盖）。
pub fn find_chrome() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("CHROME_PATH") {
        let pb = PathBuf::from(p);
        if pb.exists() {
            return Some(pb);
        }
    }
    for p in [
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/Applications/Chromium.app/Contents/MacOS/Chromium",
        "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
        "/usr/bin/google-chrome",
        "/usr/bin/google-chrome-stable",
        "/usr/bin/chromium",
        "/usr/bin/chromium-browser",
    ] {
        let pb = PathBuf::from(p);
        if pb.exists() {
            return Some(pb);
        }
    }
    #[cfg(target_os = "windows")]
    {
        for p in [
            r"C:\Program Files\Google\Chrome\Application\chrome.exe",
            r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
        ] {
            let pb = PathBuf::from(p);
            if pb.exists() {
                return Some(pb);
            }
        }
    }
    None
}

/// 取一个空闲的本地端口。
pub fn free_port() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    drop(l);
    port
}

fn http_get(url: &str) -> std::result::Result<String, String> {
    let rest = url.strip_prefix("http://").unwrap_or(url);
    let (host, path) = rest
        .split_once('/')
        .map(|(h, p)| (h, format!("/{p}")))
        .unwrap_or((rest, "/".to_string()));
    let addr = host
        .parse::<std::net::SocketAddr>()
        .map_err(|e| e.to_string())?;
    let mut conn = TcpStream::connect(addr).map_err(|e| e.to_string())?;
    conn.set_read_timeout(Some(Duration::from_secs(2))).ok();
    conn.write_all(
        format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n").as_bytes(),
    )
    .map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    while buf.len() < 8 * 1024 {
        match conn.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(_) => break,
        }
    }
    Ok(String::from_utf8_lossy(&buf).to_string())
}

fn try_get(url: &str) -> Option<String> {
    http_get(url).ok().map(|body| {
        body.split_once("\r\n\r\n")
            .map(|(_, b)| b.to_string())
            .unwrap_or(body)
    })
}

/// 轮询等待 Chrome 的 debug 端点，返回 browser ws 地址；超时返回 Err。
fn browser_ws_url(port: u16) -> Result<String, String> {
    let deadline = Instant::now() + Duration::from_secs(45);
    while Instant::now() < deadline {
        if let Some(body) = try_get(&format!("http://127.0.0.1:{port}/json/version")) {
            if body.contains("webSocketDebuggerUrl") {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&body) {
                    if let Some(ws) = v["webSocketDebuggerUrl"].as_str() {
                        return Ok(ws.to_string());
                    }
                }
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Err("chrome debug endpoint timeout".to_string())
}

/// 进程级共享浏览器（每个测试二进制仅一份）。
pub struct SharedBrowser {
    /// browser 级 CDP ws 地址。
    pub ws: String,
    /// 共享的 Chrome 子进程（进程退出后成为孤儿，由下次启动 reap）。
    _child: Child,
    _dir: tempfile::TempDir,
    /// 进程内串行锁：同一时刻至多一个测试操作本浏览器。
    lock: Mutex<()>,
}

/// 串行守卫：持有即独占共享浏览器；析构释放锁。
pub struct BrowserGuard {
    _lock: MutexGuard<'static, ()>,
}

static SHARED: OnceLock<Result<SharedBrowser, String>> = OnceLock::new();

/// 获取（并惰性启动）进程级共享浏览器。
///
/// `headless` 决定是否加 `--headless=new`（无头 vs 有头窗口）。
/// 首次调用时启动 Chrome；启动失败（如无 GUI 会话的有头测试）返回 Err，
/// 且该结果被缓存——整个测试二进制会一致地 SKIP。
pub fn shared_browser(headless: bool) -> Result<&'static SharedBrowser, String> {
    SHARED
        .get_or_init(|| {
            reap_orphan_test_chrome();
            try_launch(headless)
        })
        .as_ref()
        .map_err(|e| e.clone())
}

/// 获取共享浏览器的串行守卫。
pub fn browser_guard(b: &'static SharedBrowser) -> BrowserGuard {
    BrowserGuard {
        _lock: b.lock.lock().unwrap_or_else(|e| e.into_inner()),
    }
}

fn try_launch(headless: bool) -> Result<SharedBrowser, String> {
    let chrome = find_chrome().ok_or_else(|| "no chrome; set CHROME_PATH".to_string())?;
    let port = free_port();
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let mut cmd = Command::new(chrome);
    cmd.arg(format!("--remote-debugging-port={port}"))
        .arg(format!(
            "--user-data-dir={}",
            marker_user_data_dir(dir.path()).display()
        ))
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg("--disable-gpu")
        .arg("--disable-extensions");
    if headless {
        cmd.arg("--headless=new");
    }
    cmd.arg("about:blank")
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    match browser_ws_url(port) {
        Ok(ws) => Ok(SharedBrowser {
            ws,
            _child: child,
            _dir: dir,
            lock: Mutex::new(()),
        }),
        Err(e) => {
            // 启动失败（如无 GUI 会话）→ 清理子进程后返回错误。
            let _ = child.kill();
            let _ = child.wait();
            Err(e)
        }
    }
}
