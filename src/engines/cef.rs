// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! 桌面端 CEF 引擎（feature `engine-cef`）。
//!
//! 架构（对齐调研结论）：
//! - **CEF 作为浏览器宿主**：由 `cef` crate（tauri-apps/cef-rs）在独立线程启动，
//!   打开 `--remote-debugging-port` 暴露 CDP 端点；`on_context_initialized` 时
//!   创建一个离屏（windowless）浏览器作为首个 page target。
//! - **自动化走 CDP**：`CefEngine` 本质就是 `ChromiumCdpEngine`（见 `engines/cdp.rs`），
//!   宿主对象作为 keep-alive 随引擎存活。
//!
//! 构建前提：先运行 `scripts/fetch-cef.sh`（下载预编译 CEF，不编译 Chromium），
//! 需要 `cmake` + `ninja`，且仅在桌面三平台（Windows/macOS/Linux）上编译。

use crate::cdp::discovery::{discover_page_ws, free_port};
use crate::config::Config;
use crate::engine::Result;
use crate::engines::cdp::ChromiumCdpEngine;

/// CEF 引擎 = 连接 CEF 调试端点的 Chromium/CDP 引擎。
pub type CefEngine = ChromiumCdpEngine;

impl ChromiumCdpEngine {
    /// 启动 CEF 宿主并连接其 CDP 端点。
    pub fn new(config: &Config) -> Result<CefEngine> {
        let host = host::CefHost::start(free_port()?)?;
        let ws_url = discover_page_ws(host.port)?;
        let timeout = config.command_timeout_ms.max(5_000);
        Self::with_keep(&ws_url, config, timeout, Some(Box::new(host)))
    }
}

// ⚠️ 以下 CEF 宿主引导代码依赖 `cef` crate（tauri-apps/cef-rs）与预编译 CEF。
// 构建前请先运行 `scripts/fetch-cef.sh` 并安装 cmake + ninja（下载预编译 CEF，
// 不编译 Chromium）。CEF 二进制体积大（约 150MB），随应用打包分发（见
// `scripts/package-desktop.sh`）。本模块默认不参与 CI 编译。
pub mod host {
    //! CEF 引导：解析命令行 → execute_process → initialize → 创建浏览器 → 消息泵 → shutdown。
    //!
    //! 架构：CEF 作为宿主在独立线程运行，`on_context_initialized` 时创建一个
    //! 离屏（windowless）浏览器并打开 `remote_debugging_port`；自动化由
    //! `ChromiumCdpEngine` 通过 CDP 驱动（无需自绘，截图走 Page.captureScreenshot）。

    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    use cef::{self, args::Args, *};

    pub struct CefHost {
        pub port: u16,
        shutdown: Arc<AtomicBool>,
        handle: Option<std::thread::JoinHandle<()>>,
    }

    // ── 浏览器进程处理器：上下文初始化后创建离屏浏览器（供 CDP 连接）────────
    #[derive(Clone)]
    struct FbBph {}

    wrap_browser_process_handler! {
        pub struct FbBphBuilder {
            handler: FbBph,
        }

        impl BrowserProcessHandler {
            fn on_context_initialized(&self) {
                let window_info = WindowInfo {
                    windowless_rendering_enabled: 1,
                    ..Default::default()
                };
                let mut client = FbClientBuilder::build();
                let url: CefString = "about:blank".into();
                let settings = cef::BrowserSettings::default();
                let _ = cef::browser_host_create_browser_sync(
                    Some(&window_info),
                    Some(&mut client),
                    Some(&url),
                    Some(&settings),
                    None,
                    None,
                );
            }
        }
    }

    impl FbBphBuilder {
        fn build(handler: FbBph) -> BrowserProcessHandler {
            Self::new(handler)
        }
    }

    // ── 极简 Client（离屏渲染由 CDP 截图承担，无需自绘）───────────────────
    wrap_client! {
        pub struct FbClientBuilder {}

        impl Client {}
    }

    impl FbClientBuilder {
        fn build() -> Client {
            Self::new()
        }
    }

    // ── App：装配浏览器进程处理器 ─────────────────────────────────────────
    wrap_app! {
        pub struct FbAppBuilder {}

        impl App {
            fn browser_process_handler(&self) -> Option<cef::BrowserProcessHandler> {
                Some(FbBphBuilder::build(FbBph {}))
            }
        }
    }

    impl CefHost {
        /// 启动 CEF 主进程（阻塞式消息泵放在独立线程），返回调试端口。
        pub fn start(port: u16) -> crate::engine::Result<CefHost> {
            let shutdown = Arc::new(AtomicBool::new(false));
            let flag = shutdown.clone();
            let handle = std::thread::spawn(move || {
                let args = Args::new();
                let Some(cmd_line) = args.as_cmd_line() else {
                    return;
                };
                let switch = CefString::from("type");
                let is_browser_process = cmd_line.has_switch(Some(&switch)) != 1;

                let mut app = FbAppBuilder::new();
                let _ret = cef::execute_process(
                    Some(args.as_main_args()),
                    Some(&mut app),
                    std::ptr::null_mut(),
                );
                if !is_browser_process {
                    return; // 子进程自行退出
                }
                let settings = Settings {
                    windowless_rendering_enabled: 1,
                    external_message_pump: 1,
                    remote_debugging_port: port as i32,
                    ..Default::default()
                };
                let ok = cef::initialize(
                    Some(args.as_main_args()),
                    Some(&settings),
                    Some(&mut app),
                    std::ptr::null_mut(),
                );
                if ok != 1 {
                    return;
                }
                while !flag.load(Ordering::SeqCst) {
                    cef::do_message_loop_work();
                    std::thread::sleep(Duration::from_millis(10));
                }
                cef::shutdown();
            });
            Ok(CefHost {
                port,
                shutdown,
                handle: Some(handle),
            })
        }

        pub fn stop(mut self) {
            self.shutdown.store(true, Ordering::SeqCst);
            if let Some(h) = self.handle.take() {
                let _ = h.join();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 端点发现 / 端口工具已移至 crate::cdp::discovery（含单测）。
    #[test]
    fn cef_engine_type_alias_holds() {
        // CefEngine 即 ChromiumCdpEngine 的别名（连接端点逻辑在 cdp.rs）
        let _ = std::any::type_name::<CefEngine>();
    }
}
