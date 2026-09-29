// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Runtime 装配层：引擎 + 会话 + 工具 的统一调用入口。
//!
//! 对齐 fastshell 的 `bridge::Runtime` 语义：内核内部各子系统在此装配，
//! SDK 层只面向 `Runtime` 暴露简洁 API。
//!
//! 并发模型：`Runtime` 全部 `&self` 方法 + 内部锁（audit/session/config）。
//! 引擎采用 per-tab 锁，因此**不同标签页的工具调用可并行**（单实例并发
//! 多标签页）；同一标签页内串行。`Fastbrowser` 以 `RwLock<Runtime>` 暴露，
//! 多线程各自持有读锁即可并发调用工具。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, RwLock};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::config::Config;
use crate::engine::{BrowserEngine, EngineCapabilities, Result, TabId, TabOptions};
use crate::session::SessionManager;
use crate::tools::{ToolContext, ToolRegistry};

/// 内核运行时。
pub struct Runtime {
    engine: Arc<dyn BrowserEngine>,
    session: Mutex<SessionManager>,
    config: RwLock<Config>,
    active_profile_tab: Mutex<Option<TabId>>,
    /// First-tab creation is serialized WITHOUT holding a lock across the host
    /// `create_webview` callback: the creator sets `creating` and creates;
    /// others wait (bounded) on `tab_ready` until an active tab appears.
    tab_init: Mutex<()>,
    tab_ready: Condvar,
    creating: AtomicBool,
    /// 动作审计日志（工具调用 + SDK 操作）。
    audit: Mutex<crate::audit::AuditLog>,
    /// 电脑操作表面层（feature `surface`）。
    #[cfg(feature = "surface")]
    surface: Option<crate::engine::SurfaceRuntime>,
}

impl Runtime {
    pub fn new(engine: Box<dyn BrowserEngine>, config: Config) -> Self {
        let engine: Arc<dyn BrowserEngine> = Arc::from(engine);
        #[cfg(feature = "surface")]
        let surface = build_surface(&engine, &config);
        Runtime {
            engine,
            session: Mutex::new(SessionManager::new()),
            config: RwLock::new(config),
            active_profile_tab: Mutex::new(None),
            tab_init: Mutex::new(()),
            tab_ready: Condvar::new(),
            creating: AtomicBool::new(false),
            audit: Mutex::new(crate::audit::AuditLog::new()),
            #[cfg(feature = "surface")]
            surface,
        }
    }

    pub fn engine(&self) -> &dyn BrowserEngine {
        self.engine.as_ref()
    }

    /// 引擎原始能力位。
    pub fn engine_capabilities(&self) -> EngineCapabilities {
        self.engine.capabilities()
    }

    /// 生效能力位：在引擎能力上叠加运行时装配的表面层能力。
    pub fn effective_caps(&self) -> EngineCapabilities {
        #[allow(unused_mut)]
        let mut caps = self.engine.capabilities();
        #[cfg(feature = "surface")]
        {
            caps.supports_surface = self
                .surface
                .as_ref()
                .map(|s| s.is_available())
                .unwrap_or(false);
        }
        caps
    }

    /// 表面层（未装配则为 None）。
    #[cfg(feature = "surface")]
    pub fn surface(&self) -> Option<&crate::engine::SurfaceRuntime> {
        self.surface.as_ref()
    }

    pub fn session(&self) -> std::sync::MutexGuard<'_, SessionManager> {
        self.session.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn config(&self) -> std::sync::RwLockReadGuard<'_, Config> {
        self.config.read().unwrap_or_else(|e| e.into_inner())
    }

    /// 活动标签页（失败则提示）。
    pub fn active_tab(&self) -> Result<TabId> {
        self.engine.active_tab().ok_or_else(|| {
            crate::engine::EngineError::new(
                crate::engine::ErrorKind::TabNotFound,
                "no active tab; create one with new_tab or open",
            )
        })
    }

    /// 确保存在活动标签页。
    pub fn ensure_tab(&self) -> Result<TabId> {
        self.ensure_tab_url("about:blank").map(|(t, _)| t)
    }

    /// Serialized first-tab creation, optionally loading `url` immediately.
    ///
    /// Returns `(tab, created_with_url)`:
    /// - `created_with_url == true`: THIS call created the tab and it is already
    ///   loading `url` — the caller must not navigate again.
    /// - `false`: an existing / concurrently-created tab was returned; the
    ///   caller is responsible for navigating it to `url`.
    ///
    /// Every "no active tab yet" path MUST go through here (including
    /// `navigate`); otherwise a batched `navigate` + `get_title` each create
    /// their own tab and the active tab can point at the wrong (blank) page.
    pub fn ensure_tab_url(&self, url: &str) -> Result<(TabId, bool)> {
        // Fast path: an active tab already exists (the steady state) — no lock.
        if let Some(t) = self.engine.active_tab() {
            return Ok((t, false));
        }
        // Slow path: serialize only the FIRST tab creation. Without this,
        // concurrent first-use tools each create a blank tab, so the active tab
        // can point at the wrong page. Waiters block (bounded) instead of
        // creating their own tab.
        let mut guard = self.tab_init.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(t) = self.engine.active_tab() {
            return Ok((t, false));
        }
        if self.creating.swap(true, Ordering::SeqCst) {
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                if let Some(t) = self.engine.active_tab() {
                    return Ok((t, false));
                }
                let now = Instant::now();
                if now >= deadline {
                    return Err(crate::engine::EngineError::new(
                        crate::engine::ErrorKind::Timeout,
                        "ensure_tab: another tab is still being created",
                    ));
                }
                let (g, _) = self
                    .tab_ready
                    .wait_timeout(guard, deadline - now)
                    .unwrap_or_else(|e| e.into_inner());
                guard = g;
            }
        }
        // Creator: create WITHOUT holding the lock across the host callback
        // (avoids re-entrancy deadlock), then wake the waiters.
        drop(guard);
        let created = self.engine.create_tab(url, &TabOptions::default());
        self.creating.store(false, Ordering::SeqCst);
        self.tab_ready.notify_all();
        Ok((created?, true))
    }

    /// 创建并激活标签页（若活动 Profile 有隔离上下文，则在该上下文中创建）。
    pub fn open(&self, url: &str) -> Result<TabId> {
        let start = std::time::Instant::now();
        let result = self.create_tab_in_profile(url, true);
        let duration = start.elapsed().as_millis() as u64;
        let tab = match &result {
            Ok(t) => {
                self.audit
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .record_ok(
                        "open",
                        Some(t.as_u32()),
                        json!({"url": url}),
                        json!({"tab": t.as_u32()}),
                        duration,
                    );
                *t
            }
            Err(e) => {
                self.audit
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .record_err("open", None, json!({"url": url}), e, duration);
                return Err(e.clone());
            }
        };
        {
            let mut session = self.session.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(p) = session.active_profile() {
                session.bind_tab(tab, p);
                session.record_visit(p, url);
            }
        }
        *self
            .active_profile_tab
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(tab);
        Ok(tab)
    }

    /// 在当前活动 Profile 的（隔离）上下文中创建标签页。
    /// 若 Profile 已被显式分配隔离上下文（`set_profile_context`），无论
    /// `isolated_profiles` 开关如何都使用之；开关只控制"是否自动为尚无
    /// 上下文的 Profile 惰性创建"。自动创建在浏览器不支持时重试后退化。
    pub fn create_tab_in_profile(&self, url: &str, active: bool) -> Result<TabId> {
        let opts = TabOptions {
            active,
            ..TabOptions::default()
        };
        // 宿主指定了要复用的隔离上下文（跨进程）→ 首次使用时接管到活动 profile。
        // 这样无状态调用（如 CLI 子进程）不会每次新建一个浏览器上下文。
        let configured_ctx = self
            .config
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .browser_context_id
            .clone();
        if let Some(native) = configured_ctx {
            let needs_adopt = {
                let session = self.session.lock().unwrap_or_else(|e| e.into_inner());
                session
                    .active_profile()
                    .map(|p| session.profile_context(p).is_none())
                    .unwrap_or(false)
            };
            if needs_adopt {
                if let Ok(ctx) = self.engine.adopt_context(&native) {
                    let mut session = self.session.lock().unwrap_or_else(|e| e.into_inner());
                    if let Some(p) = session.active_profile() {
                        let _ = session.set_profile_context(p, Some(ctx));
                    }
                }
            }
        }
        let session = self.session.lock().unwrap_or_else(|e| e.into_inner());
        let profile = session.active_profile();
        let context = profile.and_then(|p| session.profile_context(p));
        drop(session);
        match context {
            Some(ctx) => self.engine.create_tab_in_context(url, &opts, ctx),
            None => {
                if !self
                    .config
                    .read()
                    .unwrap_or_else(|e| e.into_inner())
                    .isolated_profiles
                {
                    return self.engine.create_tab(url, &opts);
                }
                let created = create_context_retry(self.engine.as_ref())
                    .zip(profile)
                    .map(|(ctx, p)| (p, ctx));
                if let Some((p, ctx)) = created {
                    let _ = self
                        .session
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .set_profile_context(p, Some(ctx));
                    self.engine.create_tab_in_context(url, &opts, ctx)
                } else {
                    self.engine.create_tab(url, &opts)
                }
            }
        }
    }

    /// 执行一个工具。
    pub fn call_tool(&self, name: &str, params: Value) -> Result<Value> {
        // Accept Playwright MCP / browser-use tool + param names.
        let name = crate::tools::aliases::resolve_tool_alias(name);
        let registry = ToolRegistry::new();
        let tool = registry.find(&name).ok_or_else(|| {
            let mut msg = format!("unknown tool '{name}'");
            if let Some(s) = registry.suggest(&name) {
                msg.push_str(&format!("; did you mean '{s}'?"));
            }
            msg.push_str(" (call tool 'tools' to list available tools)");
            crate::engine::EngineError::new(crate::engine::ErrorKind::InvalidArgument, msg)
        })?;
        let (params, warnings) = crate::tools::aliases::prepare_params(params, &tool.spec.params);
        // 能力位检查：工具声明的能力不被当前引擎支持时，明确报错（而非底层 Unsupported）。
        let caps = self.effective_caps();
        if !tool.supported_by(&caps) {
            return Err(crate::engine::EngineError::unsupported(format!(
                "tool '{name}' requires a capability the '{}' engine does not support",
                self.engine.name()
            )));
        }
        let start = std::time::Instant::now();
        let tab = self.engine.active_tab().map(|t| t.as_u32());
        let mut ctx = ToolContext {
            runtime: self,
            params: params.clone(),
        };
        let result =
            (tool.run)(&mut ctx).map_err(|e| crate::tools::tool::enrich_tool_error(e, &tool.spec));
        let duration = start.elapsed().as_millis() as u64;
        let mut audit = self.audit.lock().unwrap_or_else(|e| e.into_inner());
        match &result {
            Ok(v) => audit.record_ok(&name, tab, params, v.clone(), duration),
            Err(e) => audit.record_err(&name, tab, params, e, duration),
        }
        drop(audit);
        result.map(|v| crate::tools::aliases::attach_warnings(v, warnings))
    }

    /// 审计日志（新→旧）。
    pub fn audit(&self) -> Value {
        self.audit
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .to_json()
    }

    /// 审计轨迹 → 可回放的 Python 脚本（仅成功动作）。
    pub fn audit_script(&self) -> String {
        self.audit
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .to_replay_python()
    }

    pub fn audit_len(&self) -> usize {
        self.audit.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    pub fn clear_audit(&self) {
        self.audit.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }

    /// 工具清单（按当前引擎能力过滤后的子集）。
    pub fn tool_list(&self) -> Value {
        let caps = self.effective_caps();
        ToolRegistry::new().list_for(&caps)
    }

    /// 工具数量（按当前引擎能力过滤后）。
    pub fn tool_count(&self) -> usize {
        let caps = self.effective_caps();
        ToolRegistry::new().count_for(&caps)
    }

    /// 当前状态摘要（给宿主/调试）。
    pub fn status(&self) -> Value {
        let tabs = self.engine.list_tabs();
        let session = self.session.lock().unwrap_or_else(|e| e.into_inner());
        #[allow(unused_mut)]
        let mut v = json!({
            "engine": self.engine.name(),
            "tabs": tabs.len(),
            "active_tab": self.engine.active_tab().map(|t| t.as_u32()),
            "profiles": session.list_profiles(),
            "tools": self.tool_count(),
            "rendering_mode": self.config.read().unwrap_or_else(|e| e.into_inner()).rendering_mode,
            "version": crate::VERSION,
        });
        #[cfg(feature = "surface")]
        {
            let surface = match self.surface.as_ref() {
                Some(s) => json!({
                    "available": s.is_available(),
                    "providers": s.registry().providers().iter().map(|p| p.name()).collect::<Vec<_>>(),
                    "capabilities": s.capabilities(),
                }),
                None => json!({"available": false}),
            };
            v["surface"] = surface;
        }
        v
    }

    /// 动态切换渲染模式（托管/无头）。引擎可据此调整窗口行为。
    pub fn set_rendering_mode(&self, mode: crate::engine::RenderingMode) {
        self.config
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .rendering_mode = mode;
    }

    /// 活动 profile 的标签页归属（调试）。
    pub fn active_profile_tab(&self) -> Option<TabId> {
        *self
            .active_profile_tab
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }
}

/// 创建隔离浏览器上下文；浏览器尚未就绪（如刚启动的 Chrome for Testing）
/// 时短暂重试，随后失败则交由调用方降级。
fn create_context_retry(engine: &dyn BrowserEngine) -> Option<crate::engine::ContextId> {
    for attempt in 0..3 {
        match engine.create_context() {
            Ok(c) => return Some(c),
            Err(_) if attempt < 2 => {
                std::thread::sleep(std::time::Duration::from_millis(150));
            }
            Err(_) => return None,
        }
    }
    None
}

/// 装配电脑操作表面层（feature `surface`）。返回 `None` 表示未启用/无 provider。
#[cfg(feature = "surface")]
fn build_surface(
    engine: &Arc<dyn BrowserEngine>,
    config: &Config,
) -> Option<crate::engine::SurfaceRuntime> {
    if !config.surface.enabled {
        return None;
    }
    let registry = crate::surfaces::create_registry(engine.clone(), config);
    if registry.is_empty() {
        return None;
    }
    let timeout = if config.surface.timeout_ms > 0 {
        std::time::Duration::from_millis(config.surface.timeout_ms)
    } else {
        std::time::Duration::from_secs(crate::engine::SurfaceRuntime::DEFAULT_TIMEOUT_SECS)
    };
    crate::engine::SurfaceRuntime::with_timeout(registry, timeout).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engines::mock::MockEngine;

    fn runtime() -> Runtime {
        Runtime::new(Box::new(MockEngine::new()), Config::default())
    }

    #[test]
    fn open_and_call_tools() {
        let r = runtime();
        let tab = r.open("https://example.com").unwrap();
        assert_eq!(r.active_tab().unwrap(), tab);
        let out = r.call_tool("get_page_title", json!({})).unwrap();
        assert_eq!(out["title"], "Example Page");
        r.call_tool("navigate", json!({"url": "https://example.com/login"}))
            .unwrap();
        let out = r.call_tool("get_current_url", json!({})).unwrap();
        assert_eq!(out["url"], "https://example.com/login");
    }

    #[test]
    fn unknown_tool_errors() {
        let r = runtime();
        let res = r.call_tool("nope", json!({}));
        assert!(res.is_err());
    }

    #[test]
    fn tool_list_and_status() {
        let r = runtime();
        let list = r.tool_list();
        assert!(list.as_array().unwrap().len() >= 30);
        assert!(r.tool_count() >= 30);
        let st = r.status();
        assert_eq!(st["engine"], "mock");
    }

    #[test]
    fn ensure_tab_creates_when_missing() {
        let r = runtime();
        let tab = r.ensure_tab().unwrap();
        assert_eq!(tab, r.active_tab().unwrap());
    }

    #[test]
    fn ensure_tab_reuses_existing() {
        let r = runtime();
        let t1 = r.ensure_tab().unwrap();
        let t2 = r.ensure_tab().unwrap();
        assert_eq!(t1, t2);
        assert_eq!(
            r.engine.list_tabs().len(),
            1,
            "no extra tab on the fast path"
        );
    }

    #[test]
    fn ensure_tab_recreates_after_close() {
        let r = runtime();
        let t1 = r.ensure_tab().unwrap();
        r.engine.close_tab(t1).unwrap();
        assert!(r.active_tab().is_err(), "no active tab after close");
        let t2 = r.ensure_tab().unwrap();
        assert_eq!(r.active_tab().unwrap(), t2);
        assert_eq!(
            r.engine.list_tabs().len(),
            1,
            "exactly one tab after recreate"
        );
    }

    /// Concurrent first-use must create exactly ONE tab (the regression from the
    /// Android session: a batched `navigate` + `get_title` produced two blank
    /// tabs and bound the active tab to the wrong page).
    #[test]
    fn ensure_tab_concurrent_single_tab() {
        let r = Arc::new(runtime());
        let mut handles = Vec::new();
        for _ in 0..16 {
            let r = r.clone();
            handles.push(std::thread::spawn(move || r.ensure_tab().unwrap()));
        }
        let tabs: Vec<TabId> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        let first = tabs[0];
        assert!(
            tabs.iter().all(|t| *t == first),
            "threads disagree: {tabs:?}"
        );
        assert_eq!(r.engine.list_tabs().len(), 1, "exactly one tab");
        assert_eq!(r.active_tab().unwrap(), first);
    }

    #[test]
    fn navigate_without_tab_creates_one_with_url() {
        let r = runtime();
        let out = r
            .call_tool("navigate", json!({"url": "https://example.com/x"}))
            .unwrap();
        assert_eq!(r.engine.list_tabs().len(), 1, "exactly one tab");
        assert_eq!(out["url"], "https://example.com/x");
        let cur = r.call_tool("get_current_url", json!({})).unwrap();
        assert_eq!(cur["url"], "https://example.com/x");
    }

    /// Batched first tools (which each call `active_tab()` → `ensure_tab()`)
    /// must not create multiple tabs.
    #[test]
    fn concurrent_first_tools_single_tab() {
        let r = Arc::new(runtime());
        let mut handles = Vec::new();
        for _ in 0..8 {
            let r = r.clone();
            handles.push(std::thread::spawn(move || {
                r.call_tool("get_page_title", json!({})).unwrap();
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(
            r.engine.list_tabs().len(),
            1,
            "batched first tools must not create multiple tabs"
        );
    }

    #[test]
    fn ensure_tab_url_reports_creation_vs_reuse() {
        let r = runtime();
        let (t, created) = r.ensure_tab_url("https://a.com").unwrap();
        assert!(created, "first call must create the tab");
        assert_eq!(r.engine.list_tabs()[0].url, "https://a.com");
        assert_eq!(r.engine.list_tabs().len(), 1);
        let (t2, created2) = r.ensure_tab_url("https://b.com").unwrap();
        assert_eq!(t2, t, "reuses the existing active tab");
        assert!(!created2, "reuse must not report creation");
        assert_eq!(r.engine.list_tabs().len(), 1);
    }

    /// The real Android regression: a batched `navigate` + `get_page_title` on a
    /// fresh runtime. `navigate` must go through the SAME first-tab lock as
    /// `ensure_tab`, so exactly one tab is created and the *active* tab is the
    /// navigated one (previously a blank tab won and reads came back empty).
    #[test]
    fn concurrent_navigate_and_read_single_tab_with_url() {
        for _ in 0..25 {
            let r = Arc::new(runtime());
            let r1 = r.clone();
            let r2 = r.clone();
            let t1 = std::thread::spawn(move || {
                r1.call_tool(
                    "navigate",
                    json!({"url": "https://example.com/x", "wait_until": "none"}),
                )
                .unwrap()
            });
            let t2 = std::thread::spawn(move || r2.call_tool("get_page_title", json!({})).unwrap());
            t1.join().unwrap();
            t2.join().unwrap();
            assert_eq!(r.engine.list_tabs().len(), 1, "exactly one tab");
            let cur = r.call_tool("get_current_url", json!({})).unwrap();
            assert_eq!(
                cur["url"], "https://example.com/x",
                "the active tab must be the navigated one"
            );
        }
    }
}
