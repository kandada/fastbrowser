// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! 打包集成 Chromium（引擎 `"bundled"`，feature `engine-cdp`）。
//!
//! 思路对齐 Playwright：随应用分发一个 Chromium 二进制（`vendor/chromium/`，
//! 由 `scripts/fetch-chromium.sh` 下载 Chrome for Testing），运行时以无头 +
//! `--remote-debugging-port` 启动，再经 `ChromiumCdpEngine`（CDP）驱动。
//! 这是桌面端"打包集成 Chromium"的可直接验证路径；`engine-cef` 是同一目标的
//! 嵌入（windowless CEF）变体。

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::config::Config;
use crate::engine::{EngineError, ErrorKind, Result};
use crate::engines::cdp::ChromiumCdpEngine;

static PROFILE_SEQ: AtomicU64 = AtomicU64::new(0);

/// 定位 Chromium 可执行文件（`CHROME_PATH` 覆盖；其次源码树 vendor/chromium；
/// 再其次相对当前可执行文件向上/常见目录，支持打包进 .app 后运行）。
pub fn find_bundled_binary() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("CHROME_PATH") {
        let pb = PathBuf::from(p);
        if pb.exists() {
            return Some(pb);
        }
    }
    let names = [
        "Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing",
        "Google Chrome for Testing",
        "chrome",
        "chromium",
        "chromium-browser",
        "google-chrome",
    ];

    let candidates = [
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("vendor/chromium"),
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../vendor/chromium"),
    ]
    .into_iter()
    .chain(
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf())),
    );

    for root in candidates {
        if let Ok(entries) = std::fs::read_dir(&root) {
            for e in entries.flatten() {
                let p = e.path();
                if let Some(bin) = probe(&p, &names) {
                    return Some(bin);
                }
            }
        }
    }

    // 打包布局兜底：可执行文件 ../../Frameworks（macOS .app）
    if let Ok(exe) = std::env::current_exe() {
        for depth in 1..=3 {
            let mut d = exe.clone();
            for _ in 0..depth {
                d.pop();
            }
            for sub in ["Frameworks", "chrome", "chromium"] {
                if let Ok(entries) = std::fs::read_dir(d.join(sub)) {
                    for e in entries.flatten() {
                        if let Some(bin) = probe(&e.path(), &names) {
                            return Some(bin);
                        }
                    }
                }
            }
        }
    }
    None
}

fn probe(dir: &PathBuf, names: &[&str]) -> Option<PathBuf> {
    for n in names {
        let cand = dir.join(n);
        if cand.is_file() {
            return Some(cand);
        }
    }
    if let Ok(sub) = std::fs::read_dir(dir) {
        for se in sub.flatten() {
            let sp = se.path();
            if !sp.is_dir() {
                continue;
            }
            for n in names {
                let cand = sp.join(n);
                if cand.is_file() {
                    return Some(cand);
                }
            }
        }
    }
    None
}

/// 绑定 Chromium 子进程的守卫（随引擎存活；Drop 时清理）。
pub struct BundledChromium {
    child: Child,
    port: u16,
    profile: PathBuf,
}

impl BundledChromium {
    /// 启动打包的 Chromium。`headed = true` 时打开**真实窗口**（用户像用 Chrome
    /// 一样直接操作，Agent 经 CDP 驱动同一实例）；`headed = false` 时无头离屏。
    pub fn launch(
        binary: &PathBuf,
        port: u16,
        viewport: Option<(u32, u32)>,
        headed: bool,
    ) -> Result<Self> {
        let seq = PROFILE_SEQ.fetch_add(1, Ordering::SeqCst);
        let profile =
            std::env::temp_dir().join(format!("fastbrowser-cft-{}-{seq}", std::process::id()));
        let _ = std::fs::create_dir_all(&profile);
        let mut cmd = Command::new(binary);
        cmd.arg(format!("--remote-debugging-port={port}"))
            .arg(format!("--user-data-dir={}", profile.display()))
            .arg("--no-first-run")
            .arg("--no-default-browser-check")
            .arg("--disable-extensions")
            .arg("about:blank");
        if !headed {
            cmd.arg("--headless=new").arg("--disable-gpu");
        }
        if let Some((w, h)) = viewport {
            cmd.arg(format!("--window-size={w},{h}"));
        } else if headed {
            cmd.arg("--start-maximized");
        }
        let child = cmd
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| {
                EngineError::new(ErrorKind::Io, format!("launch bundled chromium: {e}"))
            })?;
        Ok(BundledChromium {
            child,
            port,
            profile,
        })
    }

    /// 浏览器级 CDP 端点。
    pub fn browser_ws_url(&self) -> String {
        crate::cdp::discovery::discover_browser_ws(self.port).unwrap_or_default()
    }

    /// 启动并连接引擎（BundledChromium 作为 keep-alive 随引擎存活）。
    pub fn connect_engine(binary: PathBuf, config: &Config) -> Result<ChromiumCdpEngine> {
        let port = crate::cdp::discovery::free_port()?;
        let viewport = config.viewport.map(|v| (v.width, v.height));
        let headed = config.rendering_mode == crate::engine::RenderingMode::Hosted;
        let host = Self::launch(&binary, port, viewport, headed)?;
        // 等待调试端点就绪
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        let mut url = String::new();
        while std::time::Instant::now() < deadline {
            if let Ok(u) = crate::cdp::discovery::discover_browser_ws(port) {
                url = u;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        if url.is_empty() {
            return Err(EngineError::new(
                ErrorKind::Navigation,
                "bundled chromium debug endpoint not ready",
            ));
        }
        let timeout = config.command_timeout_ms.max(5_000);
        ChromiumCdpEngine::with_keep(&url, config, timeout, Some(Box::new(host)))
    }
}

impl Drop for BundledChromium {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.profile);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_bundled_binary_returns_none_or_some_without_panicking() {
        // 有 vendor/chromium 则找到，否则 None；两种都不应 panic
        let _ = find_bundled_binary();
    }
}
