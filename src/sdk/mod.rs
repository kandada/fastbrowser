// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! 对外 SDK：`Fastbrowser` 结构 + FFI + 插件 + 设备回调。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use serde_json::{json, Value};

use crate::bridge::Runtime;
use crate::config::Config;
use crate::engine::host::{EncodedFrameSink, PageEventSink, ViewFrameSink};
use crate::engine::{
    EngineError, FrameStreamOptions, Image, PageSnapshot, Result, TabId, ViewHandle, Viewport,
};
use crate::engines::create_engine;
use crate::sdk::types::SdkInfo;
use crate::session::SessionState;

pub mod device_callback;
pub mod ffi;
pub mod plugin;
pub mod types;

// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.
/// fastbrowser 内核宿主句柄（对齐 fastshell 的 `Fastshell`）。
///
/// 并发模型：所有方法 `&self`；内部 `RwLock<Runtime>` + 引擎 per-tab 锁，
/// 因此**多线程/多 Agent 可共享同一实例并发操作不同标签页**。
/// `init`/`shutdown` 取写锁（独占），其余操作取读锁（可共享）。
pub struct Fastbrowser {
    runtime: RwLock<Option<Runtime>>,
    config: RwLock<Config>,
    initialized: AtomicBool,
    event_sink: Mutex<Option<Arc<dyn PageEventSink>>>,
    frame_sink: Mutex<Option<Arc<dyn ViewFrameSink>>>,
    encoded_frame_sink: Mutex<Option<Arc<dyn EncodedFrameSink>>>,
    /// 待恢复的会话（`session_load` 后，在下次 `open` 时应用）。
    pending_state: Mutex<Option<SessionState>>,
}

impl Default for Fastbrowser {
    fn default() -> Self {
        Self::new()
    }
}

impl Fastbrowser {
    pub fn new() -> Self {
        Fastbrowser {
            runtime: RwLock::new(None),
            config: RwLock::new(Config::default()),
            initialized: AtomicBool::new(false),
            event_sink: Mutex::new(None),
            frame_sink: Mutex::new(None),
            encoded_frame_sink: Mutex::new(None),
            pending_state: Mutex::new(None),
        }
    }

    // Copyright (c) 2025 xiefujin <490021684@qq.com>
    // Licensed under Apache-2.0, see LICENSE file for full license terms.
    /// 初始化内核：创建引擎、装配 Runtime、建立会话。
    pub fn init(&self, config: Config) -> Result<()> {
        let ops = plugin::take_webview_ops();
        crate::fb_log!(
            "init: engine={}, mode={:?}",
            config.engine,
            config.rendering_mode
        );
        let engine = create_engine(&config, ops)?;
        let rt = Runtime::new(engine, config.clone());
        rt.session().ensure_default();
        *self.runtime.write().unwrap_or_else(|e| e.into_inner()) = Some(rt);
        *self.config.write().unwrap_or_else(|e| e.into_inner()) = config;
        *self.event_sink.lock().unwrap_or_else(|e| e.into_inner()) = plugin::get_event_sink();
        *self.frame_sink.lock().unwrap_or_else(|e| e.into_inner()) = plugin::get_frame_sink();
        *self
            .encoded_frame_sink
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = plugin::get_encoded_frame_sink();
        self.initialized.store(true, Ordering::SeqCst);
        Ok(())
    }

    pub fn is_initialized(&self) -> bool {
        self.initialized.load(Ordering::SeqCst)
    }

    /// 对 runtime 执行一次操作（读锁；多线程可共享）。闭包返回普通值。
    fn with_runtime<R>(&self, f: impl FnOnce(&Runtime) -> R) -> Result<R> {
        let rt = self.runtime.read().unwrap_or_else(|e| e.into_inner());
        let rt = rt.as_ref().ok_or_else(EngineError::not_initialized)?;
        Ok(f(rt))
    }

    /// 对 runtime 执行一次可能失败的操作（闭包返回 `Result`，扁平化传播）。
    fn with_runtime_ok<T>(&self, f: impl FnOnce(&Runtime) -> Result<T>) -> Result<T> {
        let rt = self.runtime.read().unwrap_or_else(|e| e.into_inner());
        let rt = rt.as_ref().ok_or_else(EngineError::not_initialized)?;
        f(rt)
    }

    /// 访问 runtime（读锁）。返回 `RwLockReadGuard<Option<Runtime>>`，
    /// 外部可用 `.as_ref().unwrap().engine() / .session()` 深入。
    pub fn runtime(&self) -> Result<std::sync::RwLockReadGuard<'_, Option<Runtime>>> {
        if !self.is_initialized() {
            return Err(EngineError::not_initialized());
        }
        Ok(self.runtime.read().unwrap_or_else(|e| e.into_inner()))
    }

    fn attach_sinks(&self, tab: TabId) {
        let sink = self
            .event_sink
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let frame = self
            .frame_sink
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let encoded = self
            .encoded_frame_sink
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if let Ok(rt) = self.runtime.read() {
            if let Some(rt) = rt.as_ref() {
                if let Some(s) = sink {
                    let _ = rt.engine().set_event_sink(tab, Some(s));
                }
                if let Some(f) = frame {
                    let _ = rt.engine().set_frame_sink(tab, Some(f));
                }
                if let Some(e) = encoded {
                    let _ = rt.engine().set_encoded_frame_sink(tab, Some(e));
                }
            }
        }
    }

    /// 打开 URL 并激活（自动建标签页）。
    pub fn open(&self, url: &str) -> Result<Value> {
        let tab = self.with_runtime_ok(|rt| rt.open(url))?;
        self.attach_sinks(tab);
        if let Some(state) = self
            .pending_state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            let _ = self.with_runtime_ok(|rt| state.apply(rt.engine(), tab));
        }
        // 轻量读取：避免 open 后触发整页快照的全量 DOM 扫描
        let url = self.with_runtime_ok(|rt| rt.engine().page_url(tab))?;
        let title = self.with_runtime_ok(|rt| rt.engine().page_title(tab))?;
        Ok(json!({"tab": tab.as_u32(), "url": url, "title": title}))
    }

    /// 导航（需已有标签页；无则自动创建）。
    pub fn navigate(&self, url: &str) -> Result<Value> {
        let tab = self.ensure_tab()?;
        self.with_runtime_ok(|rt| rt.engine().navigate(tab, url))?;
        let page_url = self.with_runtime_ok(|rt| rt.engine().page_url(tab))?;
        let title = self.with_runtime_ok(|rt| rt.engine().page_title(tab))?;
        Ok(json!({"ok": true, "url": page_url, "title": title}))
    }

    /// 执行工具调用。
    pub fn tool_call(&self, name: &str, params: Value) -> Result<Value> {
        self.with_runtime_ok(|rt| rt.call_tool(name, params))
    }

    /// 向当前聚焦元素插入文本（真实键盘通道；OSR 预览 UI 打字用）。
    pub fn type_text_focused(&self, text: &str) -> Result<Value> {
        let tab = self.ensure_tab()?;
        self.with_runtime_ok(|rt| rt.engine().type_text_focused(tab, text))?;
        Ok(json!({"ok": true, "inserted": text}))
    }

    /// 把当前活动标签页的窗口提到前台（有头模式下聚焦真实浏览器窗口）。
    pub fn focus_window(&self) -> Result<Value> {
        let tab = self
            .with_runtime(|rt| rt.engine().active_tab())?
            .ok_or(EngineError::tab_not_found(TabId(0)))?;
        self.with_runtime_ok(|rt| rt.engine().switch_tab(tab))?;
        Ok(json!({"ok": true, "tab": tab.as_u32()}))
    }

    /// 工具清单（给 LLM）。
    pub fn tool_list(&self) -> Value {
        self.with_runtime(|rt| rt.tool_list())
            .unwrap_or(Value::Null)
    }

    /// 工具数量。
    pub fn tool_count(&self) -> usize {
        self.with_runtime(|rt| rt.tool_count()).unwrap_or(0)
    }

    fn ensure_tab(&self) -> Result<TabId> {
        let tab = self.with_runtime_ok(|rt| rt.ensure_tab())?;
        self.attach_sinks(tab);
        Ok(tab)
    }

    /// 当前页快照。
    pub fn snapshot(&self) -> Result<PageSnapshot> {
        let tab = self.ensure_tab()?;
        self.with_runtime_ok(|rt| rt.engine().snapshot(tab))
    }

    /// 当前页截图。
    pub fn screenshot(&self) -> Result<Image> {
        let tab = self.ensure_tab()?;
        self.with_runtime_ok(|rt| rt.engine().screenshot(tab))
    }

    /// 当前标签页的宿主视图句柄。
    pub fn get_view(&self) -> Option<ViewHandle> {
        let rt = self.runtime().ok()?;
        let rt = rt.as_ref()?;
        let tab = rt.engine().active_tab()?;
        rt.engine().view_handle(tab)
    }

    /// 设置视口。
    pub fn set_viewport(&self, width: u32, height: u32) -> Result<()> {
        let tab = self.ensure_tab()?;
        let vp = Viewport::new(width, height);
        self.with_runtime_ok(|rt| rt.engine().set_viewport(tab, vp))
    }

    /// 启动向 `frame_sink` 的连续帧推送（OSR 预览窗用；引擎不支持则报错，
    /// 宿主可回落为轮询 `view_frame`）。
    pub fn start_frame_stream(&self, tab: TabId, opts: FrameStreamOptions) -> Result<Value> {
        let fps = opts.fps;
        self.with_runtime_ok(|rt| rt.engine().start_frame_stream(tab, opts))?;
        Ok(json!({"started": true, "tab": tab.as_u32(), "fps": fps}))
    }

    /// 停止帧推送。
    pub fn stop_frame_stream(&self, tab: TabId) -> Result<Value> {
        self.with_runtime_ok(|rt| rt.engine().stop_frame_stream(tab))?;
        Ok(json!({"stopped": true, "tab": tab.as_u32()}))
    }

    /// 动态切换渲染模式（托管/无头，thought.md §3.3）。
    pub fn set_rendering_mode(&self, mode: crate::engine::RenderingMode) {
        {
            let mut cfg = self.config.write().unwrap_or_else(|e| e.into_inner());
            cfg.rendering_mode = mode;
        }
        let _ = self.with_runtime(|rt| {
            rt.set_rendering_mode(mode);
        });
    }

    /// 保存当前标签页的会话（cookie + localStorage）到 JSON 文件。
    /// 对齐 browser-use 的 `storage_state`。
    pub fn session_save(&self, path: &str) -> Result<Value> {
        let tab = self.ensure_tab()?;
        let title = self.with_runtime(|rt| {
            rt.engine()
                .list_tabs()
                .iter()
                .find(|t| t.id == tab)
                .map(|t| t.title.clone())
                .unwrap_or_default()
        })?;
        let ts = self.with_runtime_ok(|rt| SessionState::capture(rt.engine(), tab, &title))?;
        let mut state = SessionState::new();
        state.tabs.push(ts);
        SessionState::save(&state, path)?;
        Ok(json!({"saved": path, "tabs": state.tabs.len()}))
    }

    /// 从 JSON 文件加载会话；下次 `open` 时自动恢复 cookie + localStorage。
    pub fn session_load(&self, path: &str) -> Result<Value> {
        let state = SessionState::load(path)?;
        let n = state.tabs.len();
        // 若已有活动标签页，立即恢复。
        if let Ok(tab) = self.ensure_tab() {
            let _ = self.with_runtime_ok(|rt| state.apply(rt.engine(), tab));
        }
        *self.pending_state.lock().unwrap_or_else(|e| e.into_inner()) = Some(state);
        Ok(json!({"loaded": path, "tabs": n}))
    }

    /// 清空当前状态：关闭除活动外的标签页、清 cookie 与 storage。
    /// Agent 循环在每个任务结束后调用，保证任务间隔离（对齐 browser-use clear_state）。
    pub fn clear_state(&self) -> Result<Value> {
        let active = self.with_runtime(|rt| rt.engine().active_tab())?;
        let tabs = self.with_runtime(|rt| rt.engine().list_tabs())?;
        let mut closed = 0;
        for t in tabs {
            if Some(t.id) != active {
                self.with_runtime_ok(|rt| rt.engine().close_tab(t.id))?;
                closed += 1;
            }
        }
        if let Ok(tab) = self.ensure_tab() {
            let _ = self.with_runtime_ok(|rt| {
                rt.engine().cookie_clear(tab, None, None)?;
                rt.engine().storage_clear(tab)?;
                Ok(())
            });
        }
        *self.pending_state.lock().unwrap_or_else(|e| e.into_inner()) = None;
        Ok(
            json!({"cleared": true, "closed_tabs": closed, "cookies_cleared": true, "storage_cleared": true}),
        )
    }

    /// 注册页面事件回调（init 前或 init 后均可，init 后自动附加到新标签页）。
    pub fn register_event_sink(&self, sink: Arc<dyn PageEventSink>) {
        *self.event_sink.lock().unwrap_or_else(|e| e.into_inner()) = Some(sink.clone());
        plugin::register_event_sink(sink);
    }

    /// 注册离屏帧回调。
    pub fn register_frame_sink(&self, sink: Arc<dyn ViewFrameSink>) {
        *self.frame_sink.lock().unwrap_or_else(|e| e.into_inner()) = Some(sink.clone());
        plugin::register_frame_sink(sink);
        // Attach to the active tab immediately so frames start flowing even if
        // the tab was created before the sink was registered (e.g. the desktop
        // shell registering a sink after the browser already opened a tab).
        if let Some(tab) = self
            .with_runtime(|rt| rt.engine().active_tab())
            .ok()
            .flatten()
        {
            self.attach_sinks(tab);
        }
    }

    /// 注册编码帧（JPEG/PNG）回调。`FrameSink` 常同时实现 RGBA 与编码两种 sink，
    /// 宿主两个都注册即可：RGBA 用于 Agent 感知，编码帧用于 UI 预览。
    pub fn register_encoded_frame_sink(&self, sink: Arc<dyn EncodedFrameSink>) {
        *self
            .encoded_frame_sink
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(sink.clone());
        plugin::register_encoded_frame_sink(sink);
        if let Some(tab) = self
            .with_runtime(|rt| rt.engine().active_tab())
            .ok()
            .flatten()
        {
            self.attach_sinks(tab);
        }
    }

    /// 内核信息。
    pub fn get_info(&self) -> SdkInfo {
        let st = self.status();
        SdkInfo {
            version: crate::VERSION.to_string(),
            platform: std::env::consts::OS.to_string(),
            engine: st["engine"].as_str().unwrap_or("").to_string(),
            initialized: self.is_initialized(),
            tabs: st["tabs"].as_u64().unwrap_or(0) as usize,
            active_tab: st["active_tab"].as_u64().map(|v| v as u32),
            profiles: st["profiles"].as_array().map(|a| a.len()).unwrap_or(0),
            tools: self.tool_count(),
        }
    }

    /// 状态摘要（调试）。
    pub fn status(&self) -> Value {
        self.with_runtime(|rt| rt.status())
            .unwrap_or_else(|_| json!({"initialized": false}))
    }

    /// 当前活跃标签页的 CDP 端点（websocket URL）。
    ///
    /// 供宿主写入连接清单（如 `browser.json`），以便外部进程
    /// （如独立 `fastbrowser` CLI 子进程）以 `engine="chromium"` 连到
    /// 本进程持有的常驻浏览器实例，实现"命令跨调用保活"。引擎不支持
    /// CDP（如 mock / webview 桥）时返回 `None`。
    pub fn cdp_endpoint(&self) -> Option<String> {
        self.with_runtime(|rt| {
            let tab = rt.active_tab().ok()?;
            rt.engine().cdp_endpoint(tab)
        })
        .ok()
        .flatten()
    }

    /// **浏览器级** CDP 端点（若引擎暴露）。
    ///
    /// 跨进程连接应优先使用它（而非 `cdp_endpoint` 的 page 级地址）：浏览器级
    /// 连接可访问 `Target` 域，能发现/切换/新建标签页，且不会因某个页面关闭而
    /// 失效。非 CDP 引擎返回 `None`。
    pub fn browser_cdp_endpoint(&self) -> Option<String> {
        self.with_runtime(|rt| rt.engine().browser_cdp_endpoint())
            .ok()
            .flatten()
    }

    /// 活动 Profile 的隔离上下文**原生标识**（如 CDP `browserContextId`）。
    ///
    /// 宿主可将其持久化，并在下一次（无状态）调用时通过
    /// `Config::browser_context_id` 传入，从而跨进程复用同一隔离上下文。
    /// 引擎不支持上下文时返回 `None`。
    pub fn active_context_id(&self) -> Option<String> {
        self.with_runtime(|rt| {
            let p = rt.session().active_profile()?;
            let ctx = rt.session().profile_context(p)?;
            rt.engine().context_native_id(ctx)
        })
        .ok()
        .flatten()
    }

    /// 动作审计日志（工具调用历史，新→旧）。
    pub fn audit(&self) -> Value {
        self.with_runtime(|rt| rt.audit())
            .unwrap_or_else(|_| json!([]))
    }

    /// 审计轨迹 → 可回放的 Python 脚本。
    pub fn audit_script(&self) -> String {
        self.with_runtime(|rt| rt.audit_script())
            .unwrap_or_default()
    }

    /// 清空审计日志。
    pub fn clear_audit(&self) {
        let _ = self.with_runtime(|rt| {
            rt.clear_audit();
        });
    }

    /// 关闭内核，释放引擎。
    pub fn shutdown(&self) {
        *self.runtime.write().unwrap_or_else(|e| e.into_inner()) = None;
        self.initialized.store(false, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sdk() -> Fastbrowser {
        let s = Fastbrowser::new();
        s.init(Config::default()).unwrap();
        s
    }

    #[test]
    fn open_and_tool_flow() {
        let s = sdk();
        let out = s.open("https://example.com").unwrap();
        assert_eq!(out["title"], "Example Page");
        let v = s.tool_call("extract_links", json!({})).unwrap();
        assert!(!v["links"].as_array().unwrap().is_empty());
        assert!(s.tool_count() >= 30);
    }

    #[test]
    fn snapshot_screenshot_view() {
        let s = sdk();
        s.open("https://example.com").unwrap();
        let snap = s.snapshot().unwrap();
        assert_eq!(snap.title, "Example Page");
        let img = s.screenshot().unwrap();
        assert!(img.is_valid());
        assert!(s.get_view().is_some());
        s.set_viewport(320, 480).unwrap();
        let snap = s.snapshot().unwrap();
        assert_eq!(snap.viewport.width, 320);
    }

    #[test]
    fn info_and_status() {
        let s = sdk();
        s.open("https://example.com").unwrap();
        let info = s.get_info();
        assert_eq!(info.engine, "mock");
        assert!(info.initialized);
        assert!(info.tools >= 30);
        let st = s.status();
        assert_eq!(st["engine"], "mock");
    }

    #[test]
    fn not_initialized_errors() {
        let s = Fastbrowser::new();
        assert!(s.tool_call("get_page_title", json!({})).is_err());
        assert!(s.open("https://example.com").is_err());
        let info = s.get_info();
        assert!(!info.initialized);
    }

    #[test]
    fn rendering_mode_switch() {
        let s = sdk();
        s.open("https://example.com").unwrap();
        assert_eq!(s.status()["rendering_mode"], "headless");
        s.set_rendering_mode(crate::engine::RenderingMode::Hosted);
        assert_eq!(s.status()["rendering_mode"], "hosted");
    }

    #[test]
    fn shutdown_resets() {
        let s = sdk();
        s.shutdown();
        assert!(!s.is_initialized());
        assert!(s.open("https://example.com").is_err());
    }

    #[test]
    fn session_save_load_restores_state() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let path = path.to_str().unwrap().to_string();

        // 会话 1：写入 cookie + storage，保存
        let s = sdk();
        s.open("https://example.com").unwrap();
        s.tool_call(
            "cookie_set",
            json!({"name": "sid", "value": "abc", "domain": "example.com"}),
        )
        .unwrap();
        s.tool_call("storage_set", json!({"key": "token", "value": "t1"}))
            .unwrap();
        s.session_save(&path).unwrap();
        s.shutdown();

        // 会话 2：加载并恢复
        let s2 = sdk();
        let loaded = s2.session_load(&path).unwrap();
        assert_eq!(loaded["tabs"], 1);
        s2.open("https://example.com").unwrap();
        let c = s2
            .tool_call("cookie_get", json!({"domain": "example.com"}))
            .unwrap();
        assert_eq!(c["cookies"][0]["value"], "abc");
        let st = s2
            .tool_call("storage_get", json!({"key": "token"}))
            .unwrap();
        assert_eq!(st["value"], "t1");
        s2.shutdown();
    }

    #[test]
    fn clear_state_isolates_tasks() {
        let s = sdk();
        s.open("https://example.com").unwrap();
        s.tool_call("new_tab", json!({"url": "https://example.com/search"}))
            .unwrap();
        s.tool_call(
            "cookie_set",
            json!({"name": "sid", "value": "x", "domain": "example.com"}),
        )
        .unwrap();
        s.tool_call("storage_set", json!({"key": "k", "value": "v"}))
            .unwrap();

        let out = s.clear_state().unwrap();
        assert_eq!(out["cleared"], true);
        assert!(out["closed_tabs"].as_u64().unwrap() >= 1);
        assert_eq!(
            s.tool_call("list_tabs", json!({})).unwrap()["tabs"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        let c = s.tool_call("cookie_get", json!({})).unwrap();
        assert_eq!(c["cookies"].as_array().unwrap().len(), 0);
    }
}
