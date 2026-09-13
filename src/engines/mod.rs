// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! 具体引擎适配（feature 门控）。
//!
//! - `mock`：内存参考引擎（无外部依赖，默认启用）——既是测试桩，也是
//!   "无浏览器环境的脚本化内核"，保证工具集在任何平台都可完整跑通。
//! - `chromium`：连真实 Chrome / Chromium / Edge 的 CDP 端点（`config.cdp_url`）。
//! - `cef`：桌面端 CEF 嵌入（`engine-cef`，预编译 CEF，无需编译 Chromium）。
//! - `webview` / `webkit`：系统 WebView 桥（`engine-webview`，宿主提供 `WebViewOps`；
//!   Android=WebView(Chromium) / 鸿蒙=ArkWeb(Chromium) / iOS·macOS=WKWebView(WebKit)）。
//! - `auto`：自动降级——有 `cdp_url` 先连真实 Chromium，连不上则回落 WebView ops，
//!   再回落 mock。保证"用户本机没有 Chromium 也能用"。

use std::sync::Arc;

use crate::config::Config;
use crate::engine::{BrowserEngine, EngineError, Result, WebViewOps};

pub mod mock;

#[cfg(feature = "engine-cdp")]
pub mod bundled;
#[cfg(feature = "engine-cdp")]
pub mod cdp;

#[cfg(feature = "engine-cef")]
pub mod cef;

#[cfg(feature = "engine-webview")]
pub mod webview;

/// 按配置创建引擎。
///
/// `webview_ops` 在引擎为 "webview" / "webkit" 时需要（由 sdk 层注入宿主实现）。
pub fn create_engine(
    config: &Config,
    webview_ops: Option<Arc<dyn WebViewOps>>,
) -> Result<Box<dyn BrowserEngine>> {
    #[cfg(not(feature = "engine-webview"))]
    let _ = &webview_ops;
    #[allow(unreachable_patterns)] // feature 关闭时的兜底分支
    match config.engine.as_str() {
        "mock" => {
            crate::fb_log!("engine 'mock'");
            Ok(Box::new(mock::MockEngine::new()))
        }
        #[cfg(feature = "engine-cdp")]
        "chromium" => {
            let url = config.cdp_url.clone().ok_or_else(|| {
                EngineError::new(
                    crate::engine::ErrorKind::InvalidArgument,
                    "engine 'chromium' requires config.cdp_url (a devtools ws endpoint)",
                )
            })?;
            let timeout = config.command_timeout_ms.max(5_000);
            cdp::ChromiumCdpEngine::connect(&url, config, timeout)
                .map(|e| Box::new(e) as Box<dyn BrowserEngine>)
        }
        #[cfg(feature = "engine-cef")]
        "cef" => cef::CefEngine::new(config).map(|e| Box::new(e) as Box<dyn BrowserEngine>),
        // 打包集成 Chromium（vendor/chromium，Chrome for Testing）
        #[cfg(feature = "engine-cdp")]
        "bundled" => {
            let bin = bundled::find_bundled_binary().ok_or_else(|| {
                EngineError::new(
                    crate::engine::ErrorKind::Io,
                    "bundled chromium not found; run scripts/fetch-chromium.sh",
                )
            })?;
            bundled::BundledChromium::connect_engine(bin, config)
                .map(|e| Box::new(e) as Box<dyn BrowserEngine>)
        }
        #[cfg(feature = "engine-webview")]
        "webview" | "webkit" => {
            let ops = webview_ops.ok_or_else(|| {
                EngineError::new(
                    crate::engine::ErrorKind::Plugin,
                    "engine 'webview'/'webkit' requires a WebViewOps plugin (sdk::plugin::register_webview_ops)",
                )
            })?;
            webview::WebViewEngine::new(ops, config).map(|e| Box::new(e) as Box<dyn BrowserEngine>)
        }
        // 自动降级：真实 Chromium → WebView（宿主 ops）→ mock（脚本内核）。
        // 保证无 Chromium 环境下内核仍可用（如 iOS 强制 WebKit、桌面未装 Chrome）。
        "auto" => create_auto_engine(config, webview_ops),
        "chromium" => Err(EngineError::unsupported(
            "engine 'chromium' not compiled (enable feature engine-cdp)",
        )),
        "bundled" => Err(EngineError::unsupported(
            "engine 'bundled' not compiled (enable feature engine-cdp)",
        )),
        "cef" => Err(EngineError::unsupported(
            "engine 'cef' not compiled (enable feature engine-cef)",
        )),
        "webview" | "webkit" => Err(EngineError::unsupported(
            "engine 'webview'/'webkit' not compiled (enable feature engine-webview)",
        )),
        other => Err(EngineError::invalid(format!(
            "unknown engine '{other}' (expected mock | bundled | chromium | cef | webview | webkit | auto)"
        ))),
    }
}

/// `auto` 降级链。
#[cfg(feature = "engine-cdp")]
fn create_auto_engine(
    config: &Config,
    webview_ops: Option<Arc<dyn WebViewOps>>,
) -> Result<Box<dyn BrowserEngine>> {
    #[cfg(not(feature = "engine-webview"))]
    let _ = &webview_ops;
    // 1. 显式 cdp_url → 尝试外部 Chromium
    if let Some(url) = &config.cdp_url {
        let timeout = config.command_timeout_ms.max(3_000);
        if let Ok(e) = cdp::ChromiumCdpEngine::connect(url, config, timeout) {
            return Ok(Box::new(e) as Box<dyn BrowserEngine>);
        }
    }
    // 2. 打包集成 Chromium（vendor/chromium）→ 启动并连接
    if let Some(bin) = bundled::find_bundled_binary() {
        if let Ok(e) = bundled::BundledChromium::connect_engine(bin, config) {
            return Ok(Box::new(e) as Box<dyn BrowserEngine>);
        }
    }
    // 3. 宿主 WebView ops（移动端 / macOS WKWebView）→ webview
    #[cfg(feature = "engine-webview")]
    if let Some(ops) = webview_ops {
        if let Ok(e) = webview::WebViewEngine::new(ops, config) {
            return Ok(Box::new(e) as Box<dyn BrowserEngine>);
        }
    }
    // 4. 兜底 mock（脚本内核）
    Ok(Box::new(mock::MockEngine::new()))
}

/// `auto` 降级链（未编译 engine-cdp 时）。
#[cfg(not(feature = "engine-cdp"))]
fn create_auto_engine(
    config: &Config,
    webview_ops: Option<Arc<dyn WebViewOps>>,
) -> Result<Box<dyn BrowserEngine>> {
    #[cfg(not(feature = "engine-webview"))]
    let _ = &webview_ops;
    #[cfg(feature = "engine-webview")]
    if let Some(ops) = webview_ops {
        if let Ok(e) = webview::WebViewEngine::new(ops, config) {
            return Ok(Box::new(e) as Box<dyn BrowserEngine>);
        }
    }
    let _ = &config;
    Ok(Box::new(mock::MockEngine::new()))
}

/// 引擎是否已编译。
pub fn is_engine_built(engine: &str) -> bool {
    engine == "mock"
        || engine == "auto"
        || ((engine == "bundled" || engine == "chromium") && cfg!(feature = "engine-cdp"))
        || (engine == "cef" && cfg!(feature = "engine-cef"))
        || ((engine == "webview" || engine == "webkit") && cfg!(feature = "engine-webview"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "engine-webview")]
    use crate::engine::host::NoopWebViewOps;
    use std::path::PathBuf;
    use std::sync::Mutex;
    use std::time::Duration;

    /// 并发启动多个 Chrome for Testing 会竞态，串行化 auto 类测试。
    static BUNDLE_LOCK: Mutex<()> = Mutex::new(());

    /// 跨进程租约（与 tests/common 一致）：同一时刻全局只跑一个真实浏览器。
    /// lib 单测无法引用 tests/common，这里内联一份最简实现。
    struct ChromeLease(PathBuf);
    impl Drop for ChromeLease {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    fn acquire_chrome_lease() -> ChromeLease {
        let path = std::env::temp_dir().join("fastbrowser-test-chrome.lease");
        loop {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(_) => return ChromeLease(path),
                Err(_) => {
                    if let Ok(meta) = std::fs::metadata(&path) {
                        if let Ok(m) = meta.modified() {
                            if m.elapsed()
                                .map(|e| e > Duration::from_secs(180))
                                .unwrap_or(false)
                            {
                                let _ = std::fs::remove_file(&path);
                                continue;
                            }
                        }
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        }
    }

    #[cfg(feature = "engine-webview")]
    fn ops() -> Option<Arc<dyn WebViewOps>> {
        Some(Arc::new(NoopWebViewOps))
    }

    fn cfg(engine: &str) -> Config {
        Config {
            engine: engine.to_string(),
            ..Config::default()
        }
    }

    /// 取 Ok（Box<dyn BrowserEngine> 无 Debug，避免 unwrap 编译失败）。
    fn ok(r: Result<Box<dyn BrowserEngine>>) -> Box<dyn BrowserEngine> {
        match r {
            Ok(e) => e,
            Err(e) => panic!("expected ok, got {e}"),
        }
    }

    #[test]
    fn unknown_engine_errors() {
        let c = Config {
            engine: "banana".into(),
            ..Config::default()
        };
        match create_engine(&c, None) {
            Ok(_) => panic!("expected error"),
            Err(e) => assert!(e.to_string().contains("banana")),
        }
    }

    #[test]
    fn mock_always_available() {
        let e = ok(create_engine(&cfg("mock"), None));
        assert_eq!(e.name(), "mock");
    }

    /// 环境感知：若仓库含 vendor/chromium，auto 优先 bundled。
    fn bundled_present() -> bool {
        #[cfg(feature = "engine-cdp")]
        {
            crate::engines::bundled::find_bundled_binary().is_some()
        }
        #[cfg(not(feature = "engine-cdp"))]
        {
            false
        }
    }

    #[test]
    fn auto_without_cdp_and_ops_falls_back() {
        let _g = BUNDLE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _lease = acquire_chrome_lease();
        let expect = if bundled_present() {
            "chromium"
        } else {
            "mock"
        };
        let e = ok(create_engine(&cfg("auto"), None));
        assert_eq!(e.name(), expect);
    }

    #[test]
    fn auto_with_bad_cdp_url_falls_back() {
        let _g = BUNDLE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _lease = acquire_chrome_lease();
        let mut c = cfg("auto");
        c.cdp_url = Some("ws://127.0.0.1:1/devtools/browser/x".into()); // 端口 1 通常关闭
        let expect = if bundled_present() {
            "chromium"
        } else {
            "mock"
        };
        let e = ok(create_engine(&c, None));
        assert_eq!(e.name(), expect);
    }

    #[test]
    fn auto_with_webview_ops() {
        let _g = BUNDLE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _lease = acquire_chrome_lease();
        #[cfg(feature = "engine-webview")]
        {
            // bundled 优先于 webview；无 bundled 时用 webview
            let expect = if bundled_present() {
                "chromium"
            } else {
                "webview"
            };
            let e = ok(create_engine(&cfg("auto"), ops()));
            assert_eq!(e.name(), expect);
        }
        #[cfg(not(feature = "engine-webview"))]
        {
            let expect = if bundled_present() {
                "chromium"
            } else {
                "mock"
            };
            let e = ok(create_engine(&cfg("auto"), None));
            assert_eq!(e.name(), expect);
        }
    }

    #[test]
    fn webkit_alias_requires_ops() {
        #[cfg(feature = "engine-webview")]
        {
            // 无 ops → 报错
            let r = create_engine(&cfg("webkit"), None);
            assert!(r.is_err());
            // 有 ops → 成功（宿主提供 WKWebView/WebView 实现）
            let e = ok(create_engine(&cfg("webkit"), ops()));
            assert_eq!(e.name(), "webview");
        }
    }

    #[test]
    fn webview_without_ops_errors() {
        #[cfg(feature = "engine-webview")]
        {
            let r = create_engine(&cfg("webview"), None);
            assert!(r.is_err());
        }
    }
}
