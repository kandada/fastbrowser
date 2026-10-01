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
/// System/default browser discovery (Chrome/Edge/Chromium) for the `system`
/// engine and the `auto` chain (desktop / pip).
#[cfg(feature = "engine-cdp")]
pub mod system;

#[cfg(feature = "engine-cef")]
pub mod cef;

#[cfg(feature = "engine-webview")]
pub mod webview;

/// 按配置创建引擎。
///
/// `webview_ops` 在引擎为 "webview" / "webkit" 时需要（由 sdk 层注入宿主实现）。
/// Normalize an engine name to its canonical form, accepting common aliases
/// from other ecosystems (Playwright/Puppeteer call it `chromium`/`chrome`;
/// `chrome-for-testing`/`cft` is our bundled build).
pub fn normalize_engine(name: &str) -> String {
    match name.trim().to_ascii_lowercase().as_str() {
        "chrome"
        | "headless"
        | "chromium-headless"
        | "chromium-headless-shell"
        | "chrome-headless"
        | "chrome-headless-shell" => "chromium".to_string(),
        "chrome-for-testing" | "cft" | "playwright-chromium" | "chromium-bundled" => {
            "bundled".to_string()
        }
        "webkit" => "webview".to_string(),
        // browser-brand spellings
        "google-chrome" | "googlechrome" | "chromium-browser" | "chrome-beta" | "chrome-canary"
        | "msedge" | "edge" | "edge-chromium" => "chromium".to_string(),
        "safari" | "wkwebview" | "safari-ios" | "webkitgtk" | "wpe" => "webview".to_string(),
        "default" | "default-browser" | "system-browser" | "system-default" => "system".to_string(),
        other => other.to_string(),
    }
}

/// How the engine was selected: what the caller asked for vs. what was actually
/// created. Surfaced through `status()` / `get_info()` so an `auto` fallback is
/// never silent.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct EngineReport {
    /// Canonical engine name requested (`normalize_engine(config.engine)`).
    pub requested: String,
    /// Tier actually used (`mock` | `chromium` | `bundled` | `system` | `webview`
    /// | `cef`).
    pub used: String,
    /// `true` when `used != requested` (e.g. `auto` degraded).
    pub degraded: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tried: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub missing: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

const HINT_CHROMIUM_CDP_URL: &str = "start Chrome/Chromium with --remote-debugging-port=9222 and pass its ws devtools endpoint as cdp_url, or use engine 'system'/'auto' (they launch an installed browser for you)";
const HINT_BUNDLED: &str =
    "set CHROME_PATH to a Chrome/Chromium binary, run scripts/fetch-chromium.sh (bundled Chrome for Testing), or use engine 'system'/'auto'";
const HINT_SYSTEM: &str = "install Chrome/Edge/Chromium (or set CHROME_PATH). The default browser is only usable when it is CDP-capable (not Safari/Firefox)";
const HINT_AUTO_MOCK: &str = "no drivable real browser was found, so `auto` fell back to `mock` (no real pages are loaded). To use a real browser: install Chrome/Edge/Chromium and/or `pip install playwright && playwright install chromium`, then set CHROME_PATH; or start a browser with --remote-debugging-port and pass cdp_url. Set allow_fallback_mock=false to fail instead of degrading.";

/// Resolve and create the engine, returning how it was chosen.
pub fn resolve_engine(
    config: &Config,
    webview_ops: Option<Arc<dyn WebViewOps>>,
) -> Result<(Box<dyn BrowserEngine>, EngineReport)> {
    #[cfg(not(feature = "engine-webview"))]
    let _ = &webview_ops;
    let engine = normalize_engine(&config.engine);
    let mut report = EngineReport {
        requested: engine.clone(),
        ..Default::default()
    };
    #[allow(unreachable_patterns)] // feature 关闭时的兜底分支
    let boxed: Box<dyn BrowserEngine> = match engine.as_str() {
        "mock" => {
            crate::fb_log!("engine 'mock'");
            report.used = "mock".into();
            Box::new(mock::MockEngine::new())
        }
        #[cfg(feature = "engine-cdp")]
        "chromium" => {
            let url = config.cdp_url.clone().ok_or_else(|| {
                EngineError::new(
                    crate::engine::ErrorKind::InvalidArgument,
                    "engine 'chromium' requires config.cdp_url (a devtools ws endpoint)",
                )
            })?;
            let timeout = config.effective_command_timeout_ms();
            let e = cdp::ChromiumCdpEngine::connect(&url, config, timeout).map_err(|e| {
                EngineError::new(crate::engine::ErrorKind::Navigation, format!("{} ({})", e, HINT_CHROMIUM_CDP_URL))
            })?;
            report.used = "chromium".into();
            Box::new(e)
        }
        #[cfg(feature = "engine-cef")]
        "cef" => {
            report.used = "cef".into();
            Box::new(cef::CefEngine::new(config)?)
        }
        // 打包集成 Chromium（vendor/chromium，Chrome for Testing）
        #[cfg(feature = "engine-cdp")]
        "bundled" => {
            let bin = bundled::find_bundled_binary_in(config.browser_path.as_deref()).ok_or_else(|| {
                EngineError::new(
                    crate::engine::ErrorKind::Io,
                    format!("bundled chromium not found; {}", HINT_BUNDLED),
                )
            })?;
            report.used = "bundled".into();
            Box::new(bundled::BundledChromium::connect_engine(bin, config)?)
        }
        #[cfg(feature = "engine-cdp")]
        "system" => {
            let bin = system::find_system_browser(config).ok_or_else(|| {
                EngineError::new(
                    crate::engine::ErrorKind::Io,
                    format!("no system/default browser found; {}", HINT_SYSTEM),
                )
            })?;
            report.used = "system".into();
            Box::new(bundled::BundledChromium::connect_engine(bin, config)?)
        }
        #[cfg(feature = "engine-webview")]
        "webview" | "webkit" => {
            let ops = webview_ops.ok_or_else(|| {
                EngineError::new(
                    crate::engine::ErrorKind::Plugin,
                    "engine 'webview'/'webkit' requires a WebViewOps plugin (sdk::plugin::register_webview_ops)",
                )
            })?;
            report.used = "webview".into();
            Box::new(webview::WebViewEngine::new(ops, config)?)
        }
        // 自动降级：真实 Chromium → bundled → 系统/默认浏览器 → WebView → mock。
        "auto" => return create_auto_engine(config, webview_ops, report),
        "chromium" => {
            return Err(EngineError::unsupported(
                "engine 'chromium' not compiled (enable feature engine-cdp)",
            ))
        }
        "bundled" => {
            return Err(EngineError::unsupported(
                "engine 'bundled' not compiled (enable feature engine-cdp)",
            ))
        }
        "system" => {
            return Err(EngineError::unsupported(
                "engine 'system' not compiled (enable feature engine-cdp)",
            ))
        }
        "cef" => {
            return Err(EngineError::unsupported(
                "engine 'cef' not compiled (enable feature engine-cef)",
            ))
        }
        "webview" | "webkit" => {
            return Err(EngineError::unsupported(
                "engine 'webview'/'webkit' not compiled (enable feature engine-webview)",
            ))
        }
        other => {
            return Err(EngineError::invalid(format!(
                "unknown engine '{other}' (expected mock | bundled | chromium | system | cef | webview | webkit | auto)"
            )))
        }
    };
    Ok((boxed, report))
}

/// Create an engine, discarding the selection report (kept for callers/tests
/// that don't need it). Prefer [`resolve_engine`] to observe `auto` fallbacks.
pub fn create_engine(
    config: &Config,
    webview_ops: Option<Arc<dyn WebViewOps>>,
) -> Result<Box<dyn BrowserEngine>> {
    resolve_engine(config, webview_ops).map(|(e, _)| e)
}

/// Final `auto` step: mock. Never silent — the report always carries
/// `degraded` + `reason` + `hint`. Errors out when `allow_fallback_mock=false`.
fn finalize_mock(
    config: &Config,
    mut report: EngineReport,
) -> Result<(Box<dyn BrowserEngine>, EngineReport)> {
    if !config.allow_fallback_mock {
        return Err(EngineError::unsupported(format!(
            "no real browser available (requested '{}', tried: {}); {} (set allow_fallback_mock=true to use mock).",
            report.requested,
            report.tried.join(", "),
            HINT_AUTO_MOCK
        )));
    }
    report.used = "mock".into();
    report.degraded = report.used != report.requested;
    if report.reason.is_none() {
        report.reason = Some("no real browser available".into());
    }
    report.hint = Some(HINT_AUTO_MOCK.into());
    Ok((Box::new(mock::MockEngine::new()), report))
}

/// `auto` 降级链（收集 tried/missing/reason；降级绝不静默）。
#[cfg(feature = "engine-cdp")]
fn create_auto_engine(
    config: &Config,
    webview_ops: Option<Arc<dyn WebViewOps>>,
    mut report: EngineReport,
) -> Result<(Box<dyn BrowserEngine>, EngineReport)> {
    #[cfg(not(feature = "engine-webview"))]
    let _ = &webview_ops;

    // 1. 显式 cdp_url → 尝试外部 Chromium
    report.tried.push("chromium".into());
    if let Some(url) = &config.cdp_url {
        let timeout = config.effective_command_timeout_ms();
        match cdp::ChromiumCdpEngine::connect(url, config, timeout) {
            Ok(e) => {
                report.used = "chromium".into();
                return Ok((Box::new(e), report));
            }
            Err(e) => report.reason = Some(format!("external CDP failed: {e}")),
        }
    } else {
        report.missing.push("external CDP (cdp_url)".into());
    }

    // 2. 打包集成 Chromium（vendor/chromium 或 CHROME_PATH/browser_path）
    report.tried.push("bundled".into());
    if let Some(bin) = bundled::find_bundled_binary_in(config.browser_path.as_deref()) {
        match bundled::BundledChromium::connect_engine(bin, config) {
            Ok(e) => {
                report.used = "bundled".into();
                report.degraded = report.used != report.requested;
                return Ok((Box::new(e), report));
            }
            Err(e) => report.reason = Some(format!("bundled launch failed: {e}")),
        }
    } else {
        report
            .missing
            .push("bundled Chrome for Testing (vendor/chromium | CHROME_PATH)".into());
    }

    // 3. 系统/默认浏览器（仅 CDP-capable）
    if config.use_default_browser {
        report.tried.push("system".into());
        if let Some(bin) = system::find_system_browser(config) {
            match bundled::BundledChromium::connect_engine(bin, config) {
                Ok(e) => {
                    report.used = "system".into();
                    report.degraded = report.used != report.requested;
                    return Ok((Box::new(e), report));
                }
                Err(e) => report.reason = Some(format!("system browser launch failed: {e}")),
            }
        } else {
            report
                .missing
                .push("system/default browser (Chrome/Edge/Chromium)".into());
        }
    }

    // 4. 宿主 WebView ops（移动端 / macOS WKWebView）
    #[cfg(feature = "engine-webview")]
    {
        report.tried.push("webview".into());
        if let Some(ops) = webview_ops {
            if let Ok(e) = webview::WebViewEngine::new(ops, config) {
                report.used = "webview".into();
                report.degraded = report.used != report.requested;
                return Ok((Box::new(e), report));
            }
        }
    }
    report.missing.push("host WebView backend".into());

    // 5. mock 兜底（绝不静默）
    finalize_mock(config, report)
}

/// `auto` 降级链（未编译 engine-cdp 时）。
#[cfg(not(feature = "engine-cdp"))]
fn create_auto_engine(
    config: &Config,
    webview_ops: Option<Arc<dyn WebViewOps>>,
    mut report: EngineReport,
) -> Result<(Box<dyn BrowserEngine>, EngineReport)> {
    #[cfg(not(feature = "engine-webview"))]
    let _ = &webview_ops;
    report
        .missing
        .push("external CDP / bundled / system (feature engine-cdp off)".into());
    #[cfg(feature = "engine-webview")]
    {
        report.tried.push("webview".into());
        if let Some(ops) = webview_ops {
            if let Ok(e) = webview::WebViewEngine::new(ops, config) {
                report.used = "webview".into();
                report.degraded = report.used != report.requested;
                return Ok((Box::new(e), report));
            }
        }
    }
    report.missing.push("host WebView backend".into());
    finalize_mock(config, report)
}

/// 引擎是否已编译。
pub fn is_engine_built(engine: &str) -> bool {
    let engine = normalize_engine(engine);
    let engine = engine.as_str();
    engine == "mock"
        || engine == "auto"
        || ((engine == "bundled" || engine == "chromium" || engine == "system")
            && cfg!(feature = "engine-cdp"))
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
    fn finalize_mock_reports_degraded_with_hint() {
        // The `auto` last resort must be reported, never silent.
        let report = EngineReport {
            requested: "auto".into(),
            tried: vec!["bundled".into(), "system".into()],
            missing: vec!["bundled".into()],
            ..Default::default()
        };
        let (e, r) = match finalize_mock(&Config::default(), report) {
            Ok(x) => x,
            Err(err) => panic!("expected mock, got {err}"),
        };
        assert_eq!(e.name(), "mock");
        assert_eq!(r.used, "mock");
        assert!(r.degraded, "auto->mock must be marked degraded");
        assert!(r.reason.is_some(), "must explain why it degraded");
        assert!(
            r.hint
                .as_deref()
                .unwrap_or("")
                .to_lowercase()
                .contains("mock"),
            "hint must mention the mock fallback: {:?}",
            r.hint
        );
    }

    #[test]
    fn finalize_mock_errors_when_fallback_disabled() {
        let c = Config {
            allow_fallback_mock: false,
            ..Config::default()
        };
        let report = EngineReport {
            requested: "auto".into(),
            ..Default::default()
        };
        let err = match finalize_mock(&c, report) {
            Ok(_) => panic!("expected error (allow_fallback_mock=false)"),
            Err(e) => e,
        };
        assert!(
            err.to_string().contains("no real browser available"),
            "{err}"
        );
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
        // Disable the system/default-browser tier so this test stays focused on
        // bundled → mock (a system Chrome would otherwise be launched).
        let mut c = cfg("auto");
        c.use_default_browser = false;
        let e = ok(create_engine(&c, None));
        assert_eq!(e.name(), expect);
    }

    #[test]
    fn auto_with_bad_cdp_url_falls_back() {
        let _g = BUNDLE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _lease = acquire_chrome_lease();
        let mut c = cfg("auto");
        c.cdp_url = Some("ws://127.0.0.1:1/devtools/browser/x".into()); // 端口 1 通常关闭
        c.use_default_browser = false; // keep this test on bundled → mock
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
        // Disable the system/default-browser tier (a system Chrome/Edge would
        // otherwise be launched and the engine would be `chromium`).
        let mut c = cfg("auto");
        c.use_default_browser = false;
        #[cfg(feature = "engine-webview")]
        {
            // bundled 优先于 webview；无 bundled 时用 webview
            let expect = if bundled_present() {
                "chromium"
            } else {
                "webview"
            };
            let e = ok(create_engine(&c, ops()));
            assert_eq!(e.name(), expect);
        }
        #[cfg(not(feature = "engine-webview"))]
        {
            let expect = if bundled_present() {
                "chromium"
            } else {
                "mock"
            };
            let e = ok(create_engine(&c, None));
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

    #[test]
    fn normalize_engine_aliases() {
        assert_eq!(normalize_engine("chrome"), "chromium");
        assert_eq!(normalize_engine("headless"), "chromium");
        assert_eq!(normalize_engine("Chrome-Headless-Shell"), "chromium");
        assert_eq!(normalize_engine("cft"), "bundled");
        assert_eq!(normalize_engine("chrome-for-testing"), "bundled");
        assert_eq!(normalize_engine("webkit"), "webview");
        assert_eq!(normalize_engine("mock"), "mock");
        assert_eq!(normalize_engine("auto"), "auto");
        assert_eq!(normalize_engine("  Chromium  "), "chromium");
        // is_engine_built accepts aliases.
        assert_eq!(is_engine_built("chrome"), is_engine_built("chromium"));
    }
}
