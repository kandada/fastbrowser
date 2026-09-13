// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Chromium/CDP 引擎（feature `engine-cdp`）。
//!
//! 连接任何暴露 Chrome DevTools Protocol 的 Chromium 家族端点：
//! - **CEF 嵌入**（桌面，feature `engine-cef`）：由 `engines/cef.rs` 的宿主提供端点；
//! - **独立 Chromium / Chrome**：`--remote-debugging-port` 启动后直连
//!   （引擎名 `"chromium"`，端点从 `config.cdp_url` 读取）。
//!
//! 自动化全部走 CDP（Page/Runtime/DOM/Input/Network），DOM 快照复用注入 JS，
//! 与 webview 引擎语义一致。

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::cdp::event::{extract_evaluate_result, extract_screenshot_base64, CdpEvent};
use crate::cdp::CdpClient;
use crate::config::Config;
use crate::engine::host::{EncodedFrameSink, PageEventSink, ViewFrameSink};
use crate::engine::snapshot::{FrameSnapshot, ImageInfo, InteractiveElement, LinkInfo};
use crate::engine::{
    BrowserEngine, ContextId, Cookie, DialogInfo, ElementRef, EngineCapabilities, EngineError,
    ErrorKind, EventNotifier, FrameStreamOptions, HistoryEntry, Image, InputEvent, PageEvent,
    PageSnapshot, RefKind, Result, SnapshotMeta, TabId, TabInfo, TabOptions, ViewFrame, ViewHandle,
    Viewport,
};

/// 注入 JS（共享实现见 `engine::inject`）。
fn snapshot_js() -> &'static str {
    crate::engine::inject::runtime_extract_js()
}

fn parse_elements(
    raw: &Value,
) -> Result<(Vec<InteractiveElement>, SnapshotMeta, Vec<FrameSnapshot>)> {
    crate::engine::inject::parse_snapshot_value(raw)
}

fn action_js(id: char, body: &str) -> String {
    crate::engine::inject::action_js(id, body)
}

/// True for browser-internal pages that must never be driven as a "web page":
/// any extension page (side panel / new tab / options of the host extension,
/// and other extensions), chrome:// pages and devtools. The agent must never
/// navigate or click these — doing so replaces the host application's UI.
pub(crate) fn is_internal_page_url(url: &str) -> bool {
    if url.is_empty() {
        return true;
    }
    // Any extension scheme (chrome-extension, my-extension, moz-extension, …)
    // plus the common browser-internal schemes. The kernel stays host-agnostic:
    // it must not hardcode any particular application's scheme.
    let scheme = url.split(':').next().unwrap_or("");
    scheme.ends_with("-extension")
        || matches!(scheme, "chrome" | "chrome-untrusted" | "devtools" | "edge")
}

/// A real content page the fallback may auto-attach to (excludes about:blank /
/// other about: pages so a stale empty tab never shadows the actual page).
fn is_bindable_web_page(url: &str) -> bool {
    !is_internal_page_url(url) && !url.starts_with("about:")
}

/// 推送帧流的泵线程句柄（CDP 引擎用 `Page.startScreencast` + 后台线程驱动）。
struct FrameStream {
    running: Arc<AtomicBool>,
    join: Option<std::thread::JoinHandle<()>>,
    /// 启动时的配置（视口变更时用于重载帧流，让帧尺寸跟随新视口）。
    opts: FrameStreamOptions,
}

impl Drop for FrameStream {
    fn drop(&mut self) {
        // 引擎/标签页销毁时兜底停泵：泵线程的 drain 是非阻塞的，
        // 置 running=false 后至多 sleep_ms 内退出，join 不会悬挂。
        self.running.store(false, Ordering::Relaxed);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

/// 标签页内部状态（对应一个 CDP Target session）。
struct CdpTab {
    session_id: String,
    target_id: String,
    context: Option<String>,
    url: String,
    title: String,
    elements: Vec<InteractiveElement>,
    meta: SnapshotMeta,
    frames: Vec<FrameSnapshot>,
    viewport: Viewport,
    events: VecDeque<PageEvent>,
    cookies: Vec<Cookie>,
    storage: HashMap<String, String>,
    frame_seq: u64,
    view_handle: Option<ViewHandle>,
    pending_dialog: Option<DialogInfo>,
    paused_requests: Vec<Value>,
    /// Basic Auth 凭据（`set_basic_auth` 设置后，auth 挑战自动应答）。
    basic_auth: Option<(String, String)>,
    event_sink: Option<Arc<dyn PageEventSink>>,
    frame_sink: Option<Arc<dyn ViewFrameSink>>,
    encoded_frame_sink: Option<Arc<dyn EncodedFrameSink>>,
    /// 上次完整快照（DOM 未变时直接复用，省一次全量重扫）。
    last_snapshot: Option<PageSnapshot>,
    /// 帧推送流（若有）。
    frame_stream: Option<FrameStream>,
    /// 事件通知器（事件驱动等待用）：push 时唤醒等待者。
    notifier: Arc<EventNotifier>,
}

/// CEF 引擎（CDP 驱动）。
pub struct ChromiumCdpEngine {
    /// 共享 CDP 客户端：可被多个引擎实例（多 Agent）并发驱动同一浏览器。
    cdp: Arc<CdpClient>,
    /// per-tab 状态：`Arc<RwLock<CdpTab>>` 使不同标签页的操作互不阻塞，
    /// 同一标签页内操作串行（单实例并发多标签页的核心）。
    tabs: RwLock<HashMap<TabId, Arc<RwLock<CdpTab>>>>,
    active: RwLock<Option<TabId>>,
    next_tab: AtomicU32,
    /// 默认视口覆盖：`Some` 时对所有标签页施加 `Emulation.setDeviceMetricsOverride`
    /// （无头/OSR 或宿主显式指定）；`None` 时不覆盖，让页面使用宿主窗口自身视口
    /// （hosted 真实窗口下页面随窗口/侧栏重排，不会被钉死留白）。
    viewport: RwLock<Option<Viewport>>,
    /// sessionId（Target.attachToTarget）→ TabId，用于事件按会话路由。
    session_to_tab: RwLock<HashMap<String, TabId>>,
    /// 浏览器上下文句柄 → CDP browserContextId。
    contexts: RwLock<HashMap<ContextId, String>>,
    next_context: AtomicU32,
    /// 连接时使用的 CDP 端点。
    endpoint: String,
    /// 端点基础（`ws://host:port`），用于构造 page 级地址。
    endpoint_base: String,
    /// 优先附加的 targetId（宿主指定；缺省取第一个 page target）。
    preferred_target_id: Option<String>,
    /// 优先附加的 target（按 URL 子串匹配；`targetId` 未知时的兜底）。
    preferred_target_url_contains: Option<String>,
    /// 元素操作 Actionability 自动等待超时（毫秒）。
    action_timeout_ms: u64,
    /// 下载落盘目录（`accept_downloads` 时有效）。
    download_path: Option<String>,
    /// 是否允许下载。
    accept_downloads: bool,
    /// 持有宿主（如 CEF 引导）使其存活；引擎自身逻辑不感知。
    _keep: Option<Box<dyn std::any::Any + Send + Sync>>,
    /// 引擎级全局事件通知器（异步事件泵唤醒用）。
    any: Arc<EventNotifier>,
}

/// 从端点 URL 提取基础地址（`ws://host:port`）。
pub fn endpoint_base_of(url: &str) -> &str {
    match url.find("/devtools/") {
        Some(idx) => &url[..idx],
        None => url,
    }
}

impl ChromiumCdpEngine {
    /// 连接一个已就绪的 Chromium CDP 端点。
    pub fn connect(ws_url: &str, config: &Config, timeout_ms: u64) -> Result<Self> {
        Self::with_keep(ws_url, config, timeout_ms, None)
    }

    /// 连接端点，并携带一个需与引擎同生命周期的宿主对象。
    pub fn with_keep(
        ws_url: &str,
        config: &Config,
        timeout_ms: u64,
        keep: Option<Box<dyn std::any::Any + Send + Sync>>,
    ) -> Result<Self> {
        let endpoint = ws_url.to_string();
        let endpoint_base = endpoint_base_of(ws_url).to_string();
        let cdp = Arc::new(CdpClient::connect(ws_url, timeout_ms)?);
        let engine = ChromiumCdpEngine {
            cdp,
            tabs: RwLock::new(HashMap::new()),
            active: RwLock::new(None),
            next_tab: AtomicU32::new(1),
            viewport: RwLock::new(config.viewport),
            session_to_tab: RwLock::new(HashMap::new()),
            contexts: RwLock::new(HashMap::new()),
            next_context: AtomicU32::new(1),
            _keep: keep,
            endpoint,
            endpoint_base,
            preferred_target_id: config.cdp_target_id.clone(),
            preferred_target_url_contains: config.cdp_target_url_contains.clone(),
            action_timeout_ms: config.actionability_timeout_ms.max(1000),
            download_path: config.default_download_path.clone(),
            accept_downloads: config.accept_downloads,
            any: Arc::new(EventNotifier::new()),
        };
        engine.initialize()?;
        Ok(engine)
    }

    /// 用**共享的** `Arc<CdpClient>` 构造引擎（多 Agent 共享一个浏览器连接）。
    /// 注意：不执行 `initialize()`（避免重复附加默认 target）；直接 `create_tab`
    /// 开新标签页即可。多个引擎各操自己的标签页时，命令在同一连接上并发流水线化。
    pub fn from_shared(
        ws_url: &str,
        cdp: Arc<CdpClient>,
        config: &Config,
        keep: Option<Box<dyn std::any::Any + Send + Sync>>,
    ) -> Result<Self> {
        let endpoint = ws_url.to_string();
        let endpoint_base = endpoint_base_of(ws_url).to_string();
        Ok(ChromiumCdpEngine {
            cdp,
            tabs: RwLock::new(HashMap::new()),
            active: RwLock::new(None),
            next_tab: AtomicU32::new(1),
            viewport: RwLock::new(config.viewport),
            session_to_tab: RwLock::new(HashMap::new()),
            contexts: RwLock::new(HashMap::new()),
            next_context: AtomicU32::new(1),
            _keep: keep,
            endpoint,
            endpoint_base,
            preferred_target_id: config.cdp_target_id.clone(),
            preferred_target_url_contains: config.cdp_target_url_contains.clone(),
            action_timeout_ms: config.actionability_timeout_ms.max(1000),
            download_path: config.default_download_path.clone(),
            accept_downloads: config.accept_downloads,
            any: Arc::new(EventNotifier::new()),
        })
    }

    fn read_map(&self) -> std::sync::RwLockReadGuard<'_, HashMap<TabId, Arc<RwLock<CdpTab>>>> {
        self.tabs.read().unwrap_or_else(|e| e.into_inner())
    }
    fn write_map(&self) -> std::sync::RwLockWriteGuard<'_, HashMap<TabId, Arc<RwLock<CdpTab>>>> {
        self.tabs.write().unwrap_or_else(|e| e.into_inner())
    }

    /// 取出标签页 Arc（短读锁；随后按需 read/write 该标签页自身锁 → 不同标签页并行）。
    fn tab_arc(&self, tab: TabId) -> Result<Arc<RwLock<CdpTab>>> {
        self.read_map()
            .get(&tab)
            .cloned()
            .ok_or_else(|| EngineError::tab_not_found(tab))
    }

    /// 对单个标签页做可变操作（per-tab 写锁）。
    fn with_tab<R>(&self, tab: TabId, f: impl FnOnce(&mut CdpTab) -> Result<R>) -> Result<R> {
        let p = self.tab_arc(tab)?;
        let mut guard = p.write().unwrap_or_else(|e| e.into_inner());
        f(&mut guard)
    }

    /// 只读单个标签页。
    fn read_tab<R>(&self, tab: TabId, f: impl FnOnce(&CdpTab) -> R) -> Result<R> {
        let p = self.tab_arc(tab)?;
        let guard = p.read().unwrap_or_else(|e| e.into_inner());
        Ok(f(&guard))
    }

    /// 把标签页存储的视口应用到真实页面（`Emulation.setDeviceMetricsOverride`）。
    /// 必须在标签页已建好（session 已 attach）后调用；幂等、失败静默。
    fn apply_viewport(&self, tab: TabId) {
        // 仅在配置了视口覆盖时施加；`None`（hosted 默认）时不动页面视口。
        let configured = *self.viewport.read().unwrap_or_else(|e| e.into_inner());
        if let Some(vp) = configured {
            let _ = self.set_viewport(tab, vp);
        }
    }

    /// 确保新标签页真正导航到 `url`：读取**实时**地址，若仍是空白（或渲染器
    /// 冻结读不到）则显式 `Page.navigate`。`Target.createTarget` 在
    /// `newWindow:true`（隔离上下文）时会忽略 `url` 参数，必须补一次导航。
    fn ensure_navigation(&self, tab: TabId, url: &str) {
        if url.is_empty() || url == "about:blank" || url == "about:blank#blocked" {
            return;
        }
        let live = self
            .eval_quick(tab, "location.href||''")
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_default();
        if !live.is_empty() && live != "about:blank" && live != "about:blank#blocked" {
            return;
        }
        if let Ok(sid) = self.session(tab) {
            let _ = self.cdp.send_with_session_timeout(
                Some(&sid),
                "Page.navigate",
                json!({ "url": url }),
                5000,
            );
        }
    }

    /// 查找已附加的标签页（按稳定 `targetId`）。
    fn tab_for_target(&self, target_id: &str) -> Option<TabId> {
        self.read_map()
            .iter()
            .find(|(_, arc)| arc.read().unwrap_or_else(|e| e.into_inner()).target_id == target_id)
            .map(|(id, _)| *id)
    }

    /// 附加一个 CDP target（幂等），并完成域启用 / 观察器 / 视口初始化。
    ///
    /// `url` / `title` 为创建或发现时已知的初值——先填充，使 `list_tabs` 立刻
    /// 返回真实地址（而不是空串），随后由 `refresh` / CDP 事件持续更新。
    /// `context` 为 CDP `browserContextId`（非默认上下文时非空）。
    fn attach_target(
        &self,
        target_id: &str,
        context: Option<String>,
        url: &str,
        title: &str,
    ) -> Result<TabId> {
        if let Some(existing) = self.tab_for_target(target_id) {
            return Ok(existing);
        }
        let attach = self.cdp.send(
            "Target.attachToTarget",
            json!({ "targetId": target_id, "flatten": true }),
        )?;
        let session_id = attach
            .get("result")
            .and_then(|r| r.get("sessionId"))
            .and_then(Value::as_str)
            .ok_or_else(|| EngineError::new(ErrorKind::Navigation, "attachToTarget failed"))?
            .to_string();
        let tab = TabId(self.next_tab.fetch_add(1, Ordering::SeqCst));
        let t = CdpTab {
            session_id: session_id.clone(),
            target_id: target_id.to_string(),
            context,
            url: url.to_string(),
            title: title.to_string(),
            elements: Vec::new(),
            meta: SnapshotMeta::default(),
            frames: Vec::new(),
            viewport: self
                .viewport
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .unwrap_or(Viewport::new(1280, 800)),
            events: VecDeque::new(),
            cookies: Vec::new(),
            storage: HashMap::new(),
            frame_seq: 0,
            view_handle: None,
            pending_dialog: None,
            paused_requests: Vec::new(),
            basic_auth: None,
            event_sink: None,
            frame_sink: None,
            encoded_frame_sink: None,
            last_snapshot: None,
            frame_stream: None,
            notifier: Arc::new(EventNotifier::new()),
        };
        let _ = self
            .cdp
            .send_with_session(Some(&session_id), "Page.enable", json!({}));
        let _ = self
            .cdp
            .send_with_session(Some(&session_id), "Runtime.enable", json!({}));
        let _ = self
            .cdp
            .send_with_session(Some(&session_id), "DOM.enable", json!({}));
        let _ = self
            .cdp
            .send_with_session(Some(&session_id), "Network.enable", json!({}));
        // 快照 DOM 变更观察器（缓存失效检测）
        self.install_snapshot_observer(&session_id);
        self.session_to_tab
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(session_id, tab);
        self.write_map().insert(tab, Arc::new(RwLock::new(t)));
        self.apply_viewport(tab);
        Ok(tab)
    }

    /// 依据宿主绑定（`targetId` > URL 子串）或最早附加的可绑定页，挑选活动标签。
    fn pick_active(&self, infos: &[Value]) -> Option<TabId> {
        if let Some(id) = &self.preferred_target_id {
            if let Some(tab) = self.tab_for_target(id) {
                return Some(tab);
            }
        }
        if let Some(needle) = &self.preferred_target_url_contains {
            for info in infos {
                if info.get("type").and_then(Value::as_str) != Some("page") {
                    continue;
                }
                let url = info.get("url").and_then(Value::as_str).unwrap_or("");
                if is_bindable_web_page(url) && url.contains(needle.as_str()) {
                    if let Some(tid) = info.get("targetId").and_then(Value::as_str) {
                        if let Some(tab) = self.tab_for_target(tid) {
                            return Some(tab);
                        }
                    }
                }
            }
        }
        // 兜底：最早附加的可绑定页（TabId 最小 = 最先附加）。
        self.read_map().keys().min().copied()
    }

    fn initialize(&self) -> Result<()> {
        // 浏览器级下载行为（允许下载到目录；无头亦生效）
        self.ensure_download_behavior();
        // 浏览器级端点：发现并附加**所有**可绑定页面 target，使 `list_tabs`
        // 与浏览器真实标签一致；这也是跨进程 CLI 每次 init 都能看到全部标签、
        // 并跟随「上次活跃标签」的基础。宿主的扩展页面（侧栏/新标签页/设置）
        // 与 chrome:// / devtools / about: 一律排除，避免把 UI 当成网页。
        match self.cdp.send("Target.getTargets", json!({})) {
            Ok(targets) => {
                let infos = targets
                    .get("result")
                    .and_then(|r| r.get("targetInfos"))
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                for info in &infos {
                    if info.get("type").and_then(Value::as_str) != Some("page") {
                        continue;
                    }
                    let url = info.get("url").and_then(Value::as_str).unwrap_or("");
                    if !is_bindable_web_page(url) {
                        continue;
                    }
                    let target_id = info.get("targetId").and_then(Value::as_str).unwrap_or("");
                    if target_id.is_empty() {
                        continue;
                    }
                    let context = info
                        .get("browserContextId")
                        .and_then(Value::as_str)
                        .map(String::from);
                    let title = info.get("title").and_then(Value::as_str).unwrap_or("");
                    let _ = self.attach_target(target_id, context, url, title);
                }
                if let Some(tab) = self.pick_active(&infos) {
                    *self.active.write().unwrap_or_else(|e| e.into_inner()) = Some(tab);
                }
                Ok(())
            }
            // 页面级端点：当前会话即默认标签页（无 sessionId）。
            Err(_) => {
                let tab = TabId(self.next_tab.fetch_add(1, Ordering::SeqCst));
                self.session_to_tab
                    .write()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(String::new(), tab);
                self.write_map().insert(
                    tab,
                    Arc::new(RwLock::new(CdpTab {
                        session_id: String::new(),
                        target_id: String::new(),
                        context: None,
                        url: String::new(),
                        title: String::new(),
                        elements: Vec::new(),
                        meta: SnapshotMeta::default(),
                        frames: Vec::new(),
                        viewport: self
                            .viewport
                            .read()
                            .unwrap_or_else(|e| e.into_inner())
                            .unwrap_or(Viewport::new(1280, 800)),
                        events: VecDeque::new(),
                        cookies: Vec::new(),
                        storage: HashMap::new(),
                        frame_seq: 0,
                        view_handle: None,
                        pending_dialog: None,
                        paused_requests: Vec::new(),
                        basic_auth: None,
                        event_sink: None,
                        frame_sink: None,
                        encoded_frame_sink: None,
                        last_snapshot: None,
                        frame_stream: None,
                        notifier: Arc::new(EventNotifier::new()),
                    })),
                );
                self.apply_viewport(tab);
                *self.active.write().unwrap_or_else(|e| e.into_inner()) = Some(tab);
                let _ = self.cdp.send("Page.enable", json!({}));
                let _ = self.cdp.send("Runtime.enable", json!({}));
                let _ = self.cdp.send("DOM.enable", json!({}));
                let _ = self.cdp.send("Network.enable", json!({}));
                Ok(())
            }
        }
    }

    fn session(&self, tab: TabId) -> Result<String> {
        self.read_tab(tab, |t| t.session_id.clone())
    }

    fn eval(&self, tab: TabId, expr: &str) -> Result<Value> {
        let sid = self.session(tab)?.to_string();
        let resp = self.cdp.send_with_session(
            Some(&sid),
            "Runtime.evaluate",
            json!({
                "expression": expr,
                "returnByValue": true,
                "awaitPromise": true,
                "userGesture": true,
            }),
        )?;
        extract_evaluate_result(&resp)
            .ok_or_else(|| EngineError::new(ErrorKind::Evaluate, "no result from Runtime.evaluate"))
    }

    fn refresh(&self, tab: TabId) -> Result<()> {
        // 页面导航进行中时 Runtime.evaluate 可能阻塞：用短超时 + 容错保留旧快照，
        // 但标记 meta.stale，让 Agent 知道这是过期数据、应等待后重扫。
        let raw = match self.eval_quick(tab, snapshot_js()) {
            Ok(v) => v,
            Err(e) => {
                crate::fb_log!("snapshot refresh skipped for tab {tab} (page busy): {e}");
                let _ = self.with_tab(tab, |t| {
                    t.meta.stale = true;
                    Ok(())
                });
                return Ok(());
            }
        };
        let (elements, meta, frames) = parse_elements(&raw)?;
        let title = self
            .eval_quick(tab, "document.title||''")
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_default();
        let url = self
            .eval_quick(tab, "location.href||''")
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_default();
        self.with_tab(tab, |t| {
            t.elements = elements;
            t.meta = meta;
            t.meta.stale = false;
            t.frames = frames;
            if !title.is_empty() {
                t.title = title;
            }
            if !url.is_empty() {
                t.url = url;
            }
            Ok(())
        })?;
        // 未施加视口覆盖（hosted 真实窗口）时，把上报视口同步为页面真实尺寸，
        // 避免快照里的 viewport 停留在默认值。
        if self
            .viewport
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .is_none()
        {
            if let Ok(v) = self.eval_quick(
                tab,
                "({w:window.innerWidth,h:window.innerHeight,d:window.devicePixelRatio||1})",
            ) {
                if let (Some(w), Some(h)) = (
                    v.get("w").and_then(Value::as_u64),
                    v.get("h").and_then(Value::as_u64),
                ) {
                    if w > 0 && h > 0 {
                        let d = v.get("d").and_then(Value::as_f64).unwrap_or(1.0);
                        let _ = self.with_tab(tab, |t| {
                            t.viewport = Viewport {
                                width: w as u32,
                                height: h as u32,
                                device_scale_factor: d,
                            };
                            Ok(())
                        });
                    }
                }
            }
        }
        Ok(())
    }

    fn resolve_id(&self, tab: TabId, r: &ElementRef) -> Result<char> {
        self.read_tab(tab, |t| match r.kind {
            RefKind::Snapshot => r
                .value
                .chars()
                .next()
                .ok_or_else(|| EngineError::invalid("bad snapshot id")),
            _ => t
                .elements
                .iter()
                .find(|e| e.matches_ref(r))
                .map(|e| e.id)
                .ok_or_else(|| {
                    EngineError::new(ErrorKind::Dom, format!("element {r:?} not found"))
                }),
        })?
    }

    // ── 下载落盘 ────────────────────────────────────────────────
    /// 浏览器级下载行为：`accept_downloads` 时允许下载到目标目录（无头亦生效）。
    /// 浏览器级命令，无需 session。
    fn ensure_download_behavior(&self) {
        if !self.accept_downloads {
            return;
        }
        let path = self
            .download_path
            .clone()
            .unwrap_or_else(|| "downloads".to_string());
        let _ = std::fs::create_dir_all(&path);
        let _ = self.cdp.send(
            "Browser.setDownloadBehavior",
            json!({
                "behavior": "allow",
                "downloadPath": path,
                "eventsEnabled": true,
            }),
        );
    }

    // ── 快照 DOM 变更观察器（缓存失效检测）───────────────────────
    fn snapshot_observer_js() -> &'static str {
        r#"(function(){
            if (window.__fbObserverInstalled) return;
            window.__fbObserverInstalled = true;
            window.__fbDirty = true;
            try {
                var mo = new MutationObserver(function(){ window.__fbDirty = true; });
                mo.observe(document.documentElement || document,
                    {subtree:true, childList:true, attributes:true, characterData:true});
            } catch(e) {}
        })()"#
    }

    /// 安装观察器到当前文档，并注册到后续新文档（导航后仍有效）。
    fn install_snapshot_observer(&self, sid: &str) {
        let _ = self.cdp.send_with_session(
            Some(sid),
            "Page.addScriptToEvaluateOnNewDocument",
            json!({
                "source": Self::snapshot_observer_js(),
            }),
        );
        let _ = self.cdp.send_with_session_timeout(
            Some(sid),
            "Runtime.evaluate",
            json!({
                "expression": Self::snapshot_observer_js(),
                "returnByValue": true,
            }),
            700,
        );
    }

    /// DOM 是否自上次完整快照以来发生过变化（观察器标记；失败保守返回 true）。
    fn dom_is_dirty(&self, tab: TabId) -> bool {
        self.eval_quick(tab, "window.__fbDirty === true")
            .ok()
            .and_then(|v| v.as_bool())
            .unwrap_or(true)
    }

    /// 完整扫描后复位脏标记：`setTimeout` 让 MutationObserver 的微任务先跑完，
    /// 避免我们自己的 `data-fb` 标注把脏标记又置回 true。
    fn reset_dirty(&self, tab: TabId) {
        let _ = self.eval_quick(
            tab,
            "setTimeout(function(){window.__fbDirty=false;},0);true",
        );
    }

    /// 失效快照缓存。**关键**：`Input.insertText`/`el.value=` 等改的是 input 的
    /// value 属性（非 DOM 树突变），MutationObserver 不会触发 → 下次快照会返回
    /// 缓存旧值，导致 agent "type 后快照看不到输入"。故引擎自身改元素状态时
    /// 必须主动清缓存。
    fn invalidate_snapshot_cache(&self, tab: TabId) {
        let _ = self.with_tab(tab, |t| {
            t.last_snapshot = None;
            Ok(())
        });
    }

    /// 由当前标签页缓存状态构建快照。
    fn build_snapshot(&self, tab: TabId) -> Result<PageSnapshot> {
        self.read_tab(tab, |t| PageSnapshot {
            title: t.title.clone(),
            url: t.url.clone(),
            viewport: t.viewport,
            interactive: t.elements.clone(),
            frames: t.frames.clone(),
            timestamp_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
            meta: t.meta,
        })
    }

    // ── Actionability 自动等待 ───────────────────────────────────
    /// 等待元素「出现且可见」（供 fill/check/select 等非点击动作）。
    /// 事件驱动：元素出现通常伴随 DOM 变更事件，事件到来立即复查；无事件时
    /// 按有界间隔兜底（兼容纯动画/定时器引起的出现）。
    fn wait_present(&self, tab: TabId, sid: &str, id: char) -> Result<()> {
        let timeout = self.action_timeout_ms;
        let deadline = Instant::now() + Duration::from_millis(timeout);
        let mut gen = self.event_generation(tab);
        loop {
            let js = format!(
                r#"(function(){{
                    {find}
                    var el=window.__fbFind("{id}");
                    if(!el) return false;
                    try{{var r=el.getBoundingClientRect(); return r.width>0&&r.height>0;}}catch(e){{return false;}}
                }})()"#,
                find = crate::engine::inject::fb_find_js(),
            );
            let ready = self
                .cdp
                .send_with_session_timeout(
                    Some(sid),
                    "Runtime.evaluate",
                    json!({
                        "expression": js, "returnByValue": true,
                    }),
                    1500,
                )
                .ok()
                .and_then(|r| extract_evaluate_result(&r))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if ready {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(EngineError::new(
                    ErrorKind::Timeout,
                    format!(
                        "element '{id}' not actionable after {timeout}ms (not found or hidden)"
                    ),
                ));
            }
            // 事件驱动等待：事件到来立即复查，无事件最多 50ms 兜底复查
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                continue;
            }
            let (g, _) = self.wait_event(tab, gen, remaining.min(Duration::from_millis(50)));
            gen = g;
        }
    }

    /// Playwright 风格 actionability：等待元素「可见、不被遮挡、位置稳定」
    /// 后返回点击坐标 `{x, y}`。稳定判定依赖两次采样拉开足够时间，事件驱动
    /// 等待让「出现/可见」阶段即时响应，无事件时保持 ~50ms 采样间隔。
    fn wait_clickable(&self, tab: TabId, sid: &str, id: char, timeout_ms: u64) -> Result<Value> {
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        let mut last: Option<(f64, f64)> = None;
        let mut last_err = String::from("not yet checked");
        let mut gen = self.event_generation(tab);
        loop {
            // 先查超时，再探测——保证首轮也走同一路径（也避免初始值被判为死代码）
            if Instant::now() >= deadline {
                return Err(EngineError::new(
                    ErrorKind::Timeout,
                    format!(
                        "element '{id}' not actionable after {timeout_ms}ms (last: {last_err}); \
                         the page may still be loading/moving, or the element is covered/absent"
                    ),
                ));
            }
            match self.geometry_for_click_timeout(sid, id, 2000) {
                Ok(g) if g.get("ok").and_then(Value::as_bool) == Some(true) => {
                    let (x, y) = (
                        g.get("x").and_then(Value::as_f64).unwrap_or(0.0),
                        g.get("y").and_then(Value::as_f64).unwrap_or(0.0),
                    );
                    // 稳定：连续两次采样中心偏移 < 1px（元素已停止移动/动画）
                    if let Some((px, py)) = last {
                        if (x - px).abs() < 1.0 && (y - py).abs() < 1.0 {
                            return Ok(g);
                        }
                    }
                    last = Some((x, y));
                    last_err = "unstable (element still moving)".into();
                }
                Ok(g) => {
                    // 保留完整原因（含遮挡者信息，便于 LLM 处理 overlay）
                    let err = g.get("err").and_then(Value::as_str).unwrap_or("unknown");
                    let mut detail = err.to_string();
                    if err == "covered" {
                        if let Some(b) = g.get("blocker") {
                            detail.push_str(&format!(" by another element: {b}"));
                        }
                        detail.push_str("; scroll or close the overlay first");
                    }
                    last_err = detail;
                    last = None;
                }
                Err(e) => {
                    last_err = e.to_string();
                    last = None;
                }
            }
            // 事件驱动等待：事件到来立即复查，无事件最多 50ms 兜底复查
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                continue;
            }
            let (g, _) = self.wait_event(tab, gen, remaining.min(Duration::from_millis(50)));
            gen = g;
        }
    }
}

impl ChromiumCdpEngine {
    /// 基于 `Page.getNavigationHistory` + `Page.navigateToHistoryEntry` 的历史导航。
    /// `Page.goBack/goForward` 在新版 Chrome 中不可靠；用 entryId 精确导航。
    fn navigate_history(&self, tab: TabId, delta: i32) -> Result<()> {
        let sid = self.session(tab)?.to_string();
        let resp =
            self.cdp
                .send_with_session(Some(&sid), "Page.getNavigationHistory", json!({}))?;
        let entries = resp
            .pointer("/result/entries")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let current = resp
            .pointer("/result/currentIndex")
            .and_then(Value::as_u64)
            .unwrap_or(0) as i32;
        let target = current + delta;
        if target < 0 || target >= entries.len() as i32 {
            return Ok(()); // 历史边界 no-op
        }
        let entry_id = entries[target as usize]
            .get("id")
            .and_then(Value::as_i64)
            .unwrap_or(-1);
        if entry_id >= 0 {
            self.cdp.send_with_session(
                Some(&sid),
                "Page.navigateToHistoryEntry",
                json!({ "entryId": entry_id }),
            )?;
        }
        Ok(())
    }

    /// 坐标级真实点击：Playwright 风格 actionability 等待（出现→可见→不被遮挡→
    /// 位置稳定）→ 取中心点 → `Input.dispatchMouseEvent`。
    /// `button` ∈ {left, right, middle}；`click_count` 为 1（单击）/2（双击）。
    fn click_at(&self, tab: TabId, id: char, button: &str, click_count: u8) -> Result<()> {
        let sid = self.session(tab)?.to_string();
        // 自动等待 + 稳定性判定（动态页元素刚出现/还在动时会等它就绪）
        let geometry = self.wait_clickable(tab, &sid, id, self.action_timeout_ms)?;
        let x = geometry.get("x").and_then(Value::as_f64).unwrap_or(0.0);
        let y = geometry.get("y").and_then(Value::as_f64).unwrap_or(0.0);
        let params = |kind: &str| {
            json!({
                "type": kind, "x": x, "y": y, "button": button,
                "clickCount": click_count, "pointerType": "mouse",
            })
        };
        self.cdp.send_with_session(
            Some(&sid),
            "Input.dispatchMouseEvent",
            params("mousePressed"),
        )?;
        self.cdp.send_with_session(
            Some(&sid),
            "Input.dispatchMouseEvent",
            params("mouseReleased"),
        )?;
        self.invalidate_snapshot_cache(tab);
        self.push_event(tab, PageEvent::DomChanged);
        Ok(())
    }

    /// 查询元素点击几何：滚动进视口、取顶层视口中心点、检查被遮挡元素。
    /// 支持 iframe / shadow DOM 内元素（`window.__fbFind` 跨文档定位，
    /// 坐标沿 iframe 链累加换算到顶层视口）。
    fn geometry_for_click_timeout(&self, sid: &str, id: char, timeout_ms: u64) -> Result<Value> {
        let js = format!(
            r#"(function(){{
                {find}
                var el=window.__fbFind("{id}");
                if(!el) return {{ok:false,err:'notfound'}};
                try {{ el.scrollIntoView({{block:'center',inline:'center'}}); }} catch(e) {{}}
                var r=el.getBoundingClientRect();
                if(r.width<=0||r.height<=0) return {{ok:false,err:'hidden'}};
                var cx=r.left+r.width/2, cy=r.top+r.height/2;
                var w=el.ownerDocument.defaultView;
                var iframeChain=[];
                while(w && w!==window){{
                    var fe=w.frameElement;
                    if(fe){{ var fr=fe.getBoundingClientRect(); cx+=fr.left; cy+=fr.top; iframeChain.push(fe); }}
                    w=w.parent;
                }}
                // 可接受命中的元素集：el 自身 + 祖先 + shadow host 链
                // （elementFromPoint 不穿透 open shadow root，会返回宿主元素）
                var accept=[];
                var n=el;
                while(n){{ accept.push(n); n=n.parentNode; }}
                var root=el.getRootNode?el.getRootNode():document;
                while(root && root.host){{ accept.push(root.host); root=root.host.getRootNode?root.host.getRootNode():document; }}
                var top=document.elementFromPoint(cx,cy);
                var hit=top;
                if(top && hit && accept.indexOf(hit)===-1 && iframeChain.indexOf(hit)===-1){{
                    return {{ok:false,err:'covered',blocker:{{tag:top.tagName?top.tagName.toLowerCase():'',
                        role:top.getAttribute?top.getAttribute('role'):null,
                        text:(top.innerText||'').trim().slice(0,80)}}}};
                }}
                return {{ok:true,x:cx,y:cy}};
            }})()"#,
            find = crate::engine::inject::fb_find_js(),
        );
        let resp = self.cdp.send_with_session_timeout(
            Some(sid),
            "Runtime.evaluate",
            json!({
                "expression": js,
                "returnByValue": true,
            }),
            timeout_ms,
        )?;
        extract_evaluate_result(&resp)
            .ok_or_else(|| EngineError::new(ErrorKind::Evaluate, "no result from click geometry"))
    }

    /// 直接从 CDP 求值（&self 可用；storage 工具需要）。
    fn eval_sync(&self, tab: TabId, expr: &str) -> Result<Value> {
        let sid = self.session(tab)?.to_string();
        let resp = self.cdp.send_with_session(
            Some(&sid),
            "Runtime.evaluate",
            json!({
                "expression": expr,
                "returnByValue": true,
                "awaitPromise": true,
                "userGesture": true,
            }),
        )?;
        extract_evaluate_result(&resp)
            .ok_or_else(|| EngineError::new(ErrorKind::Evaluate, "no result from Runtime.evaluate"))
    }

    /// 短超时求值（快速内省用；导航进行中不会长时间阻塞）。
    fn eval_quick(&self, tab: TabId, expr: &str) -> Result<Value> {
        let sid = self.session(tab)?.to_string();
        let resp = self.cdp.send_with_session_timeout(
            Some(&sid),
            "Runtime.evaluate",
            json!({
                "expression": expr,
                "returnByValue": true,
                "awaitPromise": true,
                "userGesture": true,
            }),
            700,
        )?;
        extract_evaluate_result(&resp)
            .ok_or_else(|| EngineError::new(ErrorKind::Evaluate, "no result from Runtime.evaluate"))
    }

    /// 读取该标签页上下文中的真实 cookie（URL 作用域，遵守浏览器上下文隔离）。
    /// 通过 `Network.getCookies{urls:[当前URL]}` 获取，仅返回当前上下文内、
    /// 与该 URL 匹配的 cookie。
    fn real_cookies(&self, tab: TabId) -> Result<Vec<Cookie>> {
        let sid = self.session(tab)?.to_string();
        let url = self.read_tab(tab, |t| t.url.clone()).unwrap_or_default();
        if url.is_empty() {
            return Ok(Vec::new());
        }
        let resp = self.cdp.send_with_session(
            Some(&sid),
            "Network.getCookies",
            json!({ "urls": [url] }),
        )?;
        let arr = resp
            .get("result")
            .and_then(|r| r.get("cookies"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut out = Vec::new();
        for c in arr {
            out.push(Cookie {
                name: c
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                value: c
                    .get("value")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                domain: c
                    .get("domain")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                path: c
                    .get("path")
                    .and_then(Value::as_str)
                    .unwrap_or("/")
                    .to_string(),
                expires: c.get("expires").and_then(Value::as_f64).map(|f| f as i64),
                secure: c.get("secure").and_then(Value::as_bool).unwrap_or(false),
                http_only: c.get("httpOnly").and_then(Value::as_bool).unwrap_or(false),
                same_site: c.get("sameSite").and_then(Value::as_str).map(String::from),
            });
        }
        Ok(out)
    }

    /// 把 CDP 事件路由到对应标签页，并转成 PageEvent / 更新内部状态。
    fn route_cdp_events(&self) {
        for ev in self.cdp.drain_events() {
            let tab = match &ev.session_id {
                Some(sid) => self
                    .session_to_tab
                    .read()
                    .unwrap_or_else(|e| e.into_inner())
                    .get(sid)
                    .copied(),
                None => *self.active.read().unwrap_or_else(|e| e.into_inner()),
            };
            let Some(tab) = tab else { continue };
            let method = ev.method.as_str();
            match method {
                "Page.frameNavigated" => {
                    let url = ev.url().unwrap_or_default();
                    if ev
                        .params
                        .get("frame")
                        .and_then(|f| f.get("type"))
                        .and_then(Value::as_str)
                        == Some("navigation")
                    {
                        self.push_event(tab, PageEvent::NavigationCompleted { url, status: 200 });
                    }
                }
                "Page.frameStartedLoading" => {
                    let url = ev.url().unwrap_or_default();
                    self.push_event(tab, PageEvent::NavigationStarted { url });
                }
                "Page.loadEventFired" => {
                    let url = self.read_tab(tab, |t| t.url.clone()).unwrap_or_default();
                    self.push_event(tab, PageEvent::Loaded { url });
                }
                "Page.frameStoppedLoading" => {
                    let url = ev.url().unwrap_or_default();
                    self.push_event(tab, PageEvent::NavigationCompleted { url, status: 200 });
                }
                "Runtime.consoleAPICalled" => {
                    let args = ev
                        .params
                        .get("args")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default();
                    let message = args
                        .first()
                        .and_then(|a| a.get("value").and_then(Value::as_str))
                        .map(String::from)
                        .unwrap_or_else(|| {
                            args.first()
                                .and_then(|a| a.get("description").and_then(Value::as_str))
                                .unwrap_or("")
                                .to_string()
                        });
                    let level = match ev.params.get("type").and_then(Value::as_str) {
                        Some("error") => crate::engine::ConsoleLevel::Error,
                        Some("warning") => crate::engine::ConsoleLevel::Warning,
                        Some("info") | Some("debug") => crate::engine::ConsoleLevel::Debug,
                        _ => crate::engine::ConsoleLevel::Log,
                    };
                    self.push_event(tab, PageEvent::Console { level, message });
                }
                "Network.requestWillBeSent" => {
                    let req = ev.params.get("request").cloned().unwrap_or(Value::Null);
                    let url = req
                        .get("url")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let method = req
                        .get("method")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    self.push_event(tab, PageEvent::Request { url, method });
                }
                "Network.responseReceived" => {
                    let url = ev.url().unwrap_or_default();
                    let status = ev.status().unwrap_or(0);
                    self.push_event(tab, PageEvent::Response { url, status });
                }
                "Page.javascriptDialogOpening" => {
                    let message = ev
                        .params
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let kind = ev
                        .params
                        .get("type")
                        .and_then(Value::as_str)
                        .unwrap_or("alert")
                        .to_string();
                    let default_prompt = ev
                        .params
                        .get("defaultPrompt")
                        .and_then(Value::as_str)
                        .map(String::from);
                    let dlg = DialogInfo {
                        message: message.clone(),
                        kind: kind.clone(),
                        default_prompt,
                    };
                    let _ = self.with_tab(tab, |t| {
                        t.pending_dialog = Some(dlg);
                        Ok(())
                    });
                    self.push_event(tab, PageEvent::Dialog { message, kind });
                }
                "Page.javascriptDialogClosed" => {
                    let _ = self.with_tab(tab, |t| {
                        t.pending_dialog = None;
                        Ok(())
                    });
                }
                "Page.downloadWillBegin" => {
                    let url = ev.url().unwrap_or_default();
                    self.push_event(tab, PageEvent::Download { url });
                }
                "Fetch.requestPaused" => {
                    let req = ev.params.get("request").cloned().unwrap_or(Value::Null);
                    // 捕获请求体（POST data）供 LLM 感知/修改；截断防超大请求撑爆内存。
                    let post_data = req
                        .get("postData")
                        .and_then(Value::as_str)
                        .map(|s| s.chars().take(2048).collect::<String>());
                    let mut entry = json!({
                        "request_id": ev.params.get("requestId").and_then(Value::as_str).unwrap_or(""),
                        "url": req.get("url").and_then(Value::as_str).unwrap_or(""),
                        "method": req.get("method").and_then(Value::as_str).unwrap_or(""),
                        "resource_type": ev.params.get("resourceType").and_then(Value::as_str).unwrap_or(""),
                    });
                    if let Some(pd) = post_data {
                        entry["post_data"] = json!(pd);
                    }
                    let _ = self.with_tab(tab, |t| {
                        t.paused_requests.push(entry);
                        // 有界缓存：防被拦截请求长期未处理时无限增长（保留最新）
                        while t.paused_requests.len() > crate::engine::MAX_PAUSED_REQUESTS {
                            t.paused_requests.remove(0);
                        }
                        Ok(())
                    });
                }
                "Fetch.authRequired" => {
                    let request_id = ev
                        .params
                        .get("requestId")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let creds = self.read_tab(tab, |t| t.basic_auth.clone()).unwrap_or(None);
                    let sid = ev.session_id.clone().unwrap_or_default();
                    let response = match creds {
                        Some((u, p)) => json!({
                            "response": "ProvideCredentials",
                            "username": u,
                            "password": p,
                        }),
                        None => json!({ "response": "CancelAuth" }),
                    };
                    let _ = self.cdp.send_with_session(
                        Some(&sid),
                        "Fetch.continueWithAuth",
                        json!({ "requestId": request_id, "authChallengeResponse": response }),
                    );
                }
                _ => {}
            }
        }
    }

    fn push_event(&self, tab: TabId, ev: PageEvent) {
        if let Ok(p) = self.tab_arc(tab) {
            let mut t = p.write().unwrap_or_else(|e| e.into_inner());
            if let Some(sink) = t.event_sink.clone() {
                sink.on_page_event(tab, &ev);
            }
            t.events.push_back(ev);
            // 有界缓冲：防 Agent 不 drain 时无限增长（保留最新）
            while t.events.len() > crate::engine::MAX_BUFFERED_EVENTS {
                t.events.pop_front();
            }
            t.notifier.notify();
            self.any.notify();
        }
    }
}

impl BrowserEngine for ChromiumCdpEngine {
    fn name(&self) -> &'static str {
        "chromium"
    }

    fn capabilities(&self) -> EngineCapabilities {
        EngineCapabilities::full()
    }

    fn cdp_endpoint(&self, tab: TabId) -> Option<String> {
        let target_id = self.read_tab(tab, |t| t.target_id.clone()).ok()?;
        if target_id.is_empty() {
            Some(self.endpoint.clone())
        } else {
            Some(format!(
                "{}/devtools/page/{}",
                self.endpoint_base, target_id
            ))
        }
    }

    fn browser_cdp_endpoint(&self) -> Option<String> {
        Some(self.endpoint.clone())
    }

    fn create_tab(&self, url: &str, opts: &TabOptions) -> Result<TabId> {
        self.create_tab_in_context(url, opts, ContextId(0))
    }

    fn close_tab(&self, tab: TabId) -> Result<()> {
        let target_id = self.read_tab(tab, |t| t.target_id.clone())?;
        let _ = self
            .cdp
            .send("Target.closeTarget", json!({ "targetId": target_id }));
        self.session_to_tab
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|_, v| *v != tab);
        self.write_map().remove(&tab);
        let mut active = self.active.write().unwrap_or_else(|e| e.into_inner());
        if *active == Some(tab) {
            let first = self.read_map().keys().next().copied();
            *active = first;
        }
        Ok(())
    }

    fn create_context(&self) -> Result<ContextId> {
        let resp = self.cdp.send(
            "Target.createBrowserContext",
            json!({ "disposeOnDetach": false }),
        )?;
        let cdp_id = resp
            .get("result")
            .and_then(|r| r.get("browserContextId"))
            .and_then(Value::as_str)
            .ok_or_else(|| EngineError::new(ErrorKind::Internal, "createBrowserContext failed"))?
            .to_string();
        let id = ContextId(self.next_context.fetch_add(1, Ordering::SeqCst));
        self.contexts
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id, cdp_id);
        Ok(id)
    }

    fn dispose_context(&self, context: ContextId) -> Result<()> {
        let cdp_id = self
            .contexts
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&context)
            .ok_or_else(|| {
                EngineError::new(
                    ErrorKind::InvalidArgument,
                    format!("context {context} not found"),
                )
            })?;
        let _ = self.cdp.send(
            "Target.disposeBrowserContext",
            json!({ "browserContextId": cdp_id }),
        );
        // 关闭该上下文内的标签页
        let doomed: Vec<TabId> = self
            .read_map()
            .iter()
            .filter(|(_, arc)| {
                arc.read()
                    .unwrap_or_else(|e| e.into_inner())
                    .context
                    .as_deref()
                    == Some(cdp_id.as_str())
            })
            .map(|(id, _)| *id)
            .collect();
        for id in doomed {
            self.session_to_tab
                .write()
                .unwrap_or_else(|e| e.into_inner())
                .retain(|_, v| *v != id);
            self.write_map().remove(&id);
        }
        let mut active = self.active.write().unwrap_or_else(|e| e.into_inner());
        if let Some(a) = *active {
            if !self.read_map().contains_key(&a) {
                *active = self.read_map().keys().next().copied();
            }
        }
        Ok(())
    }

    fn context_native_id(&self, context: ContextId) -> Option<String> {
        self.contexts
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(&context)
            .cloned()
    }

    /// 接管一个已存在的 CDP `browserContextId`（跨进程/跨连接复用同一隔离
    /// 上下文）。先向浏览器确认该上下文仍存在，避免用到已随浏览器重启失效的 id。
    fn adopt_context(&self, native_id: &str) -> Result<ContextId> {
        if native_id.is_empty() {
            return Err(EngineError::invalid("empty browserContextId"));
        }
        {
            let contexts = self.contexts.read().unwrap_or_else(|e| e.into_inner());
            if let Some((id, _)) = contexts.iter().find(|(_, v)| v.as_str() == native_id) {
                return Ok(*id);
            }
        }
        let resp = self.cdp.send("Target.getBrowserContexts", json!({}))?;
        let exists = resp
            .get("result")
            .and_then(|r| r.get("browserContextIds"))
            .and_then(Value::as_array)
            .map(|ids| ids.iter().any(|v| v.as_str() == Some(native_id)))
            .unwrap_or(false);
        if !exists {
            return Err(EngineError::new(
                ErrorKind::InvalidArgument,
                format!("browser context '{native_id}' no longer exists"),
            ));
        }
        let id = ContextId(self.next_context.fetch_add(1, Ordering::SeqCst));
        self.contexts
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id, native_id.to_string());
        Ok(id)
    }

    fn create_tab_in_context(
        &self,
        url: &str,
        opts: &TabOptions,
        context: ContextId,
    ) -> Result<TabId> {
        let context_id = if context.as_u32() == 0 {
            None
        } else {
            self.contexts
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .get(&context)
                .cloned()
        };
        let mut params = json!({ "url": url, "newWindow": false });
        if let Some(cid) = &context_id {
            params["browserContextId"] = json!(cid);
        }
        // 非默认浏览器上下文无窗口：Chrome 要求 `newWindow: true` 才会创建 target
        // （否则报 "Failed to open new tab - no browser is open"）。先试普通创建，
        // 失败时以开新窗口重试。
        let resp = match self.cdp.send("Target.createTarget", params.clone()) {
            Ok(r) => r,
            Err(_) if context_id.is_some() => {
                // 非默认上下文需开新窗口：newWindow=true + 创建后激活窗口（避开无头节流）。
                params["newWindow"] = json!(true);
                self.cdp.send("Target.createTarget", params)?
            }
            Err(e) => return Err(e),
        };
        let target_id = resp
            .get("result")
            .and_then(|r| r.get("targetId"))
            .and_then(Value::as_str)
            .ok_or_else(|| EngineError::new(ErrorKind::Navigation, "createTarget failed"))?
            .to_string();
        let attach = self.cdp.send(
            "Target.attachToTarget",
            json!({ "targetId": target_id, "flatten": true }),
        )?;
        let session_id = attach
            .get("result")
            .and_then(|r| r.get("sessionId"))
            .and_then(Value::as_str)
            .ok_or_else(|| EngineError::new(ErrorKind::Navigation, "attachToTarget failed"))?
            .to_string();
        let tab = TabId(self.next_tab.fetch_add(1, Ordering::SeqCst));
        let in_context = context_id.is_some();
        let mut t = CdpTab {
            session_id,
            target_id,
            context: context_id,
            url: url.to_string(),
            title: String::new(),
            elements: Vec::new(),
            meta: SnapshotMeta::default(),
            frames: Vec::new(),
            viewport: self
                .viewport
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .unwrap_or(Viewport::new(1280, 800)),
            events: VecDeque::new(),
            cookies: Vec::new(),
            storage: HashMap::new(),
            frame_seq: 0,
            view_handle: None,
            pending_dialog: None,
            paused_requests: Vec::new(),
            basic_auth: None,
            event_sink: None,
            frame_sink: None,
            encoded_frame_sink: None,
            last_snapshot: None,
            frame_stream: None,
            notifier: Arc::new(EventNotifier::new()),
        };
        t.events.push_back(PageEvent::NavigationStarted {
            url: url.to_string(),
        });
        t.events.push_back(PageEvent::NavigationCompleted {
            url: url.to_string(),
            status: 200,
        });
        t.events.push_back(PageEvent::Loaded {
            url: url.to_string(),
        });
        let _ = self
            .cdp
            .send_with_session(Some(&t.session_id), "Page.enable", json!({}));
        let _ = self
            .cdp
            .send_with_session(Some(&t.session_id), "Runtime.enable", json!({}));
        let _ = self
            .cdp
            .send_with_session(Some(&t.session_id), "DOM.enable", json!({}));
        // 网络事件（请求/响应）也启用：新标签页同样能推送 Request/Response 事件
        let _ = self
            .cdp
            .send_with_session(Some(&t.session_id), "Network.enable", json!({}));
        // 下载行为（浏览器级，幂等）+ 快照观察器
        self.ensure_download_behavior();
        self.install_snapshot_observer(&t.session_id);
        self.session_to_tab
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(t.session_id.clone(), tab);
        self.write_map().insert(tab, Arc::new(RwLock::new(t)));
        self.apply_viewport(tab);
        // 新窗口（newWindow:true）的 URL 参数不生效 → 激活窗口并解冻后显式导航。
        // 无头下背景窗口的渲染器会被冻结（Runtime.evaluate 长时间不响应），
        // 需显式置为 active 生命周期；有头浏览器无此问题。
        if in_context && !url.is_empty() && url != "about:blank" {
            let sid = self.session(tab)?.to_string();
            let _ = self
                .cdp
                .send_with_session(Some(&sid), "Page.bringToFront", json!({}));
            let _ = self.cdp.send_with_session(
                Some(&sid),
                "Page.setWebLifecycleState",
                json!({ "state": "active" }),
            );
            let _ = self.cdp.send_with_session(
                Some(&sid),
                "Emulation.setFocusEmulationEnabled",
                json!({ "enabled": true }),
            );
        }
        // 默认上下文与隔离上下文都确保真正导航（隔离上下文会忽略 createTarget 的 url）。
        self.ensure_navigation(tab, url);
        let _ = self.refresh(tab);
        if opts.active {
            *self.active.write().unwrap_or_else(|e| e.into_inner()) = Some(tab);
        }
        Ok(tab)
    }

    fn list_tabs(&self) -> Vec<TabInfo> {
        let map = self.read_map();
        map.iter()
            .map(|(id, arc)| {
                let t = arc.read().unwrap_or_else(|e| e.into_inner());
                TabInfo {
                    id: *id,
                    url: t.url.clone(),
                    title: t.title.clone(),
                    loading: false,
                    pinned: false,
                    created_ms: 0,
                    target_id: if t.target_id.is_empty() {
                        None
                    } else {
                        Some(t.target_id.clone())
                    },
                }
            })
            .collect()
    }

    fn new_window(&self, url: &str, opts: &TabOptions) -> Result<TabId> {
        let resp = self.cdp.send(
            "Target.createTarget",
            json!({ "url": url, "newWindow": true }),
        )?;
        let target_id = resp
            .get("result")
            .and_then(|r| r.get("targetId"))
            .and_then(Value::as_str)
            .ok_or_else(|| EngineError::new(ErrorKind::Navigation, "createTarget failed"))?
            .to_string();
        let attach = self.cdp.send(
            "Target.attachToTarget",
            json!({ "targetId": target_id, "flatten": true }),
        )?;
        let session_id = attach
            .get("result")
            .and_then(|r| r.get("sessionId"))
            .and_then(Value::as_str)
            .ok_or_else(|| EngineError::new(ErrorKind::Navigation, "attachToTarget failed"))?
            .to_string();
        let tab = TabId(self.next_tab.fetch_add(1, Ordering::SeqCst));
        let mut t = CdpTab {
            session_id,
            target_id,
            context: None,
            url: url.to_string(),
            title: String::new(),
            elements: Vec::new(),
            meta: SnapshotMeta::default(),
            frames: Vec::new(),
            viewport: self
                .viewport
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .unwrap_or(Viewport::new(1280, 800)),
            events: VecDeque::new(),
            cookies: Vec::new(),
            storage: HashMap::new(),
            frame_seq: 0,
            view_handle: None,
            pending_dialog: None,
            paused_requests: Vec::new(),
            basic_auth: None,
            event_sink: None,
            frame_sink: None,
            encoded_frame_sink: None,
            last_snapshot: None,
            frame_stream: None,
            notifier: Arc::new(EventNotifier::new()),
        };
        t.events.push_back(PageEvent::NavigationStarted {
            url: url.to_string(),
        });
        t.events.push_back(PageEvent::NavigationCompleted {
            url: url.to_string(),
            status: 200,
        });
        t.events.push_back(PageEvent::Loaded {
            url: url.to_string(),
        });
        let _ = self
            .cdp
            .send_with_session(Some(&t.session_id), "Page.enable", json!({}));
        let _ = self
            .cdp
            .send_with_session(Some(&t.session_id), "Runtime.enable", json!({}));
        let _ = self
            .cdp
            .send_with_session(Some(&t.session_id), "DOM.enable", json!({}));
        let _ = self
            .cdp
            .send_with_session(Some(&t.session_id), "Network.enable", json!({}));
        self.ensure_download_behavior();
        self.install_snapshot_observer(&t.session_id);
        self.session_to_tab
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(t.session_id.clone(), tab);
        self.write_map().insert(tab, Arc::new(RwLock::new(t)));
        self.apply_viewport(tab);
        // 新窗口的 URL 参数不生效 → 激活窗口并解冻后显式导航。
        if !url.is_empty() && url != "about:blank" {
            let sid = self.session(tab)?.to_string();
            let _ = self
                .cdp
                .send_with_session(Some(&sid), "Page.bringToFront", json!({}));
            let _ = self.cdp.send_with_session(
                Some(&sid),
                "Page.setWebLifecycleState",
                json!({ "state": "active" }),
            );
            let _ = self.cdp.send_with_session_timeout(
                Some(&sid),
                "Page.navigate",
                json!({ "url": url }),
                800,
            );
        }
        let _ = self.refresh(tab);
        if opts.active {
            *self.active.write().unwrap_or_else(|e| e.into_inner()) = Some(tab);
        }
        Ok(tab)
    }

    fn switch_tab(&self, tab: TabId) -> Result<()> {
        if !self.read_map().contains_key(&tab) {
            return Err(EngineError::tab_not_found(tab));
        }
        let sid = self.read_tab(tab, |t| t.session_id.clone())?;
        let _ = self
            .cdp
            .send_with_session(Some(&sid), "Page.bringToFront", json!({}));
        *self.active.write().unwrap_or_else(|e| e.into_inner()) = Some(tab);
        Ok(())
    }

    fn active_tab(&self) -> Option<TabId> {
        *self.active.read().unwrap_or_else(|e| e.into_inner())
    }

    fn navigate(&self, tab: TabId, url: &str) -> Result<()> {
        let sid = self.session(tab)?;
        let _ = self
            .cdp
            .send_with_session(Some(&sid), "Page.navigate", json!({ "url": url }))?;
        self.with_tab(tab, |t| {
            t.url = url.to_string();
            // 只推 NavigationStarted；真正的 Completed/Loaded 由真实 CDP 事件
            // （Page.frameStoppedLoading / Page.loadEventFired）驱动，
            // 避免 `wait_for_navigation` 等逻辑在页面尚未加载完时误判“已加载”。
            t.events.push_back(PageEvent::NavigationStarted {
                url: url.to_string(),
            });
            while t.events.len() > crate::engine::MAX_BUFFERED_EVENTS {
                t.events.pop_front();
            }
            t.notifier.notify();
            self.any.notify();
            Ok(())
        })?;
        Ok(())
    }

    fn back(&self, tab: TabId) -> Result<()> {
        self.navigate_history(tab, -1)
    }

    fn forward(&self, tab: TabId) -> Result<()> {
        self.navigate_history(tab, 1)
    }

    fn reload(&self, tab: TabId) -> Result<()> {
        let sid = self.session(tab)?.to_string();
        let _ =
            self.cdp
                .send_with_session(Some(&sid), "Page.reload", json!({ "ignoreCache": false }));
        Ok(())
    }

    fn stop(&self, tab: TabId) -> Result<()> {
        let sid = self.session(tab)?.to_string();
        let _ = self
            .cdp
            .send_with_session(Some(&sid), "Page.stopLoading", json!({}));
        Ok(())
    }

    fn snapshot(&self, tab: TabId) -> Result<PageSnapshot> {
        // 快速路径：DOM 未变化（MutationObserver 脏标记）且 URL 未变 → 复用缓存，
        // 避免大页面高频快照时反复全量重扫整棵 DOM。
        let cached = self
            .read_tab(tab, |t| t.last_snapshot.clone())
            .unwrap_or(None);
        if let Some(c) = &cached {
            let url_ok = self
                .eval_quick(tab, "location.href||''")
                .ok()
                .and_then(|v| v.as_str().map(String::from))
                .map(|u| u == c.url)
                .unwrap_or(false);
            // 滚动位置也纳入快路径判定（滚动不是 DOM 突变，观察器不会标记）
            let scroll_ok = self
                .eval_quick(tab, "window.scrollY||0")
                .ok()
                .and_then(|v| v.as_f64())
                .map(|s| s as u32 == c.meta.scroll_y)
                .unwrap_or(false);
            if url_ok && scroll_ok && !self.dom_is_dirty(tab) {
                return Ok(c.clone());
            }
        }
        self.refresh(tab)?;
        let snap = self.build_snapshot(tab)?;
        // 仅在全量扫描成功（页面未忙、URL 已知）时缓存并复位脏标记；
        // 页面忙时保持脏标记，下次 snapshot 会再次尝试全量扫描。
        let ok = self
            .read_tab(tab, |t| !t.meta.stale && !t.url.is_empty())
            .unwrap_or(false);
        if ok {
            self.with_tab(tab, |t| {
                t.last_snapshot = Some(snap.clone());
                Ok(())
            })?;
            self.reset_dirty(tab);
        }
        Ok(snap)
    }

    fn get_page_text(&self, tab: TabId) -> Result<String> {
        self.eval(tab, "document.body?document.body.innerText:''")
            .map(|v| v.as_str().map(String::from).unwrap_or_default())
    }

    fn get_page_html(&self, tab: TabId) -> Result<String> {
        self.eval(
            tab,
            "document.documentElement?document.documentElement.outerHTML:''",
        )
        .map(|v| v.as_str().map(String::from).unwrap_or_default())
    }

    fn get_links(&self, tab: TabId) -> Result<Vec<LinkInfo>> {
        let v = self.eval(tab, "Array.from(document.querySelectorAll('a')).map(a=>({url:a.href,text:a.innerText||''}))")?;
        Ok(v.as_array()
            .map(|a| {
                a.iter()
                    .map(|e| LinkInfo {
                        url: e
                            .get("url")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        text: e
                            .get("text")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    fn get_images(&self, tab: TabId) -> Result<Vec<ImageInfo>> {
        let v = self.eval(
            tab,
            "Array.from(document.images).map(i=>({src:i.src,alt:i.alt}))",
        )?;
        Ok(v.as_array()
            .map(|a| {
                a.iter()
                    .map(|e| ImageInfo {
                        src: e
                            .get("src")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        alt: e
                            .get("alt")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        width: None,
                        height: None,
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    fn get_table(&self, tab: TabId) -> Result<Vec<Vec<String>>> {
        let v = self.eval(tab, r#"(()=>{const t=document.querySelector('table');if(!t)return [];return Array.from(t.rows).map(r=>Array.from(r.cells).map(c=>c.innerText||''));})()"#)?;
        Ok(v.as_array()
            .map(|rows| {
                rows.iter()
                    .map(|r| {
                        r.as_array()
                            .map(|cells| {
                                cells
                                    .iter()
                                    .filter_map(|c| c.as_str().map(String::from))
                                    .collect()
                            })
                            .unwrap_or_default()
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    fn execute_xpath(&self, tab: TabId, expr: &str) -> Result<Value> {
        let js = format!(
            r#"(()=>{{const r=document.evaluate('{}',document,null,XPathResult.ORDERED_NODE_SNAPSHOT_TYPE,null);let out=[];for(let i=0;i<r.snapshotLength;i++){{out.push(r.snapshotItem(i).innerText||'');}}return out;}})()"#,
            expr.replace('\'', "\\'")
        );
        self.eval(tab, &js)
    }

    fn click_element(&self, tab: TabId, element: &ElementRef) -> Result<()> {
        let id = self.resolve_id(tab, element)?;
        self.click_at(tab, id, "left", 1)
    }

    fn double_click_element(&self, tab: TabId, element: &ElementRef) -> Result<()> {
        let id = self.resolve_id(tab, element)?;
        self.click_at(tab, id, "left", 2)
    }

    fn right_click_element(&self, tab: TabId, element: &ElementRef) -> Result<()> {
        let id = self.resolve_id(tab, element)?;
        self.click_at(tab, id, "right", 1)
    }

    fn set_element_value(&self, tab: TabId, element: &ElementRef, value: &str) -> Result<()> {
        let id = self.resolve_id(tab, element)?;
        let sid = self.session(tab)?.to_string();
        self.wait_present(tab, &sid, id)?;
        let val = serde_json::to_string(value).unwrap_or_else(|_| "\"\"".into());
        let js = action_js(
            id,
            &format!(
                r#"el.focus();el.value={val};el.dispatchEvent(new Event('input',{{bubbles:true}}));el.dispatchEvent(new Event('change',{{bubbles:true}}))"#
            ),
        );
        let v = self.eval(tab, &js)?;
        if v.as_str() == Some("notfound") {
            return Err(EngineError::new(
                ErrorKind::Dom,
                "element not found in page",
            ));
        }
        self.invalidate_snapshot_cache(tab);
        Ok(())
    }

    fn type_text(&self, tab: TabId, element: &ElementRef, text: &str, clear: bool) -> Result<()> {
        let id = self.resolve_id(tab, element)?;
        let sid = self.session(tab)?.to_string();
        self.wait_present(tab, &sid, id)?;
        // 聚焦 + 可选清空
        let clear_js = if clear {
            "el.focus();el.value='';el.dispatchEvent(new Event('input',{bubbles:true}));"
                .to_string()
        } else {
            "el.focus();".to_string()
        };
        let v = self.eval(tab, &action_js(id, &clear_js))?;
        if v.as_str() == Some("notfound") {
            return Err(EngineError::new(
                ErrorKind::Dom,
                "element not found in page",
            ));
        }
        // 等元素真正获得焦点（headless 下 JS focus 偶发异步）。
        // 事件驱动：focus 通常伴随 DOM/输入事件，事件到来立即复查，无事件兜底轮询。
        let _ = crate::engine::wait_until(
            self,
            tab,
            Duration::from_millis(1000),
            "element focus",
            || {
                let focused = self
                    .eval_quick(tab, &format!("document.activeElement&&document.activeElement.getAttribute('data-fb')==='{id}'"))
                    .ok()
                    .and_then(|x| x.as_bool())
                    .unwrap_or(false);
                Ok(focused)
            },
        );
        // 真实键盘插入：Input.insertText 触发原生 input 事件链
        // （React 受控组件 / IME 兼容，等价于用户逐键输入）
        if !text.is_empty() {
            self.cdp
                .send_with_session(Some(&sid), "Input.insertText", json!({ "text": text }))?;
        }
        self.invalidate_snapshot_cache(tab);
        self.push_event(tab, PageEvent::DomChanged);
        Ok(())
    }

    fn type_text_focused(&self, tab: TabId, text: &str) -> Result<()> {
        if text.is_empty() {
            return Ok(());
        }
        let sid = self.session(tab)?.to_string();
        // 插入到页面当前聚焦元素（用户先在预览中点击聚焦，再直接打字）。
        self.cdp
            .send_with_session(Some(&sid), "Input.insertText", json!({ "text": text }))?;
        self.invalidate_snapshot_cache(tab);
        self.push_event(tab, PageEvent::DomChanged);
        Ok(())
    }

    fn select_option(&self, tab: TabId, element: &ElementRef, value: &str) -> Result<()> {
        let id = self.resolve_id(tab, element)?;
        let sid = self.session(tab)?.to_string();
        self.wait_present(tab, &sid, id)?;
        let val = serde_json::to_string(value).unwrap_or_else(|_| "\"\"".into());
        let js = action_js(
            id,
            &format!(r#"el.value={val};el.dispatchEvent(new Event('change',{{bubbles:true}}))"#),
        );
        let v = self.eval(tab, &js)?;
        if v.as_str() == Some("notfound") {
            return Err(EngineError::new(
                ErrorKind::Dom,
                "element not found in page",
            ));
        }
        Ok(())
    }

    fn check_element(&self, tab: TabId, element: &ElementRef, checked: bool) -> Result<()> {
        let id = self.resolve_id(tab, element)?;
        let sid = self.session(tab)?.to_string();
        self.wait_present(tab, &sid, id)?;
        let js = action_js(
            id,
            &format!(
                r#"el.checked={checked};el.dispatchEvent(new Event('change',{{bubbles:true}}))"#
            ),
        );
        let v = self.eval(tab, &js)?;
        if v.as_str() == Some("notfound") {
            return Err(EngineError::new(
                ErrorKind::Dom,
                "element not found in page",
            ));
        }
        self.invalidate_snapshot_cache(tab);
        Ok(())
    }

    fn set_file_input(&self, tab: TabId, element: &ElementRef, paths: &[String]) -> Result<()> {
        let id = self.resolve_id(tab, element)?;
        let sid = self.session(tab)?.to_string();
        // 1. 拿到根节点 → 2. querySelector 定位 [data-fb] → 3. setFileInputFiles
        let doc = self.cdp.send_with_session(
            Some(&sid),
            "DOM.getDocument",
            json!({ "depth": -1, "pierce": true }),
        )?;
        let root = doc
            .get("result")
            .and_then(|r| r.get("root"))
            .and_then(|r| r.get("nodeId"))
            .and_then(Value::as_i64)
            .ok_or_else(|| EngineError::new(ErrorKind::Dom, "DOM.getDocument failed"))?;
        let qs = self.cdp.send_with_session(
            Some(&sid),
            "DOM.querySelector",
            json!({ "nodeId": root, "selector": format!("[data-fb=\"{id}\"]") }),
        )?;
        let node_id = qs
            .get("result")
            .and_then(|r| r.get("nodeId"))
            .and_then(Value::as_i64)
            .ok_or_else(|| EngineError::new(ErrorKind::Dom, "file input element not found"))?;
        self.cdp.send_with_session(
            Some(&sid),
            "DOM.setFileInputFiles",
            json!({ "nodeId": node_id, "files": paths }),
        )?;
        self.push_event(tab, PageEvent::DomChanged);
        Ok(())
    }

    fn inject_css(&self, tab: TabId, css: &str) -> Result<()> {
        let sid = self.session(tab)?.to_string();
        let css_js = serde_json::to_string(css).unwrap_or_else(|_| "\"\"".into());
        let _ = self
            .cdp
            .send_with_session(Some(&sid), "CSS.enable", json!({}));
        let js = format!("(()=>{{let s=document.createElement('style');s.textContent={css_js};document.head.appendChild(s);return 'ok';}})()");
        let _ = self.eval(tab, &js)?;
        Ok(())
    }

    fn inject_event(&self, tab: TabId, ev: InputEvent) -> Result<()> {
        let sid = self.session(tab)?.to_string();
        match ev {
            InputEvent::Mouse(m) => {
                let kind = match m.kind {
                    crate::engine::MouseKind::Move => "mouseMoved",
                    crate::engine::MouseKind::Down => "mousePressed",
                    crate::engine::MouseKind::Up => "mouseReleased",
                    crate::engine::MouseKind::Click => "mousePressed",
                    crate::engine::MouseKind::DoubleClick => "mousePressed",
                };
                let button = match m.button {
                    crate::engine::MouseButton::Left => "left",
                    crate::engine::MouseButton::Right => "right",
                    crate::engine::MouseButton::Middle => "middle",
                    crate::engine::MouseButton::None => "none",
                };
                self.cdp.send_with_session(Some(&sid), "Input.dispatchMouseEvent", json!({
                    "type": kind, "x": m.x, "y": m.y, "button": button, "clickCount": m.click_count,
                }))?;
                if m.kind == crate::engine::MouseKind::Click
                    || m.kind == crate::engine::MouseKind::DoubleClick
                {
                    self.cdp.send_with_session(Some(&sid), "Input.dispatchMouseEvent", json!({
                        "type": "mouseReleased", "x": m.x, "y": m.y, "button": button, "clickCount": m.click_count,
                    }))?;
                }
                Ok(())
            }
            InputEvent::Key(k) => {
                let modifiers = modifier_mask(&k.modifiers);
                match k.kind {
                    crate::engine::KeyKind::Down => {
                        self.cdp.send_with_session(Some(&sid), "Input.dispatchKeyEvent", json!({
                            "type": "keyDown", "key": k.key, "code": k.code, "modifiers": modifiers,
                        }))?;
                    }
                    crate::engine::KeyKind::Up => {
                        self.cdp.send_with_session(Some(&sid), "Input.dispatchKeyEvent", json!({
                            "type": "keyUp", "key": k.key, "code": k.code, "modifiers": modifiers,
                        }))?;
                    }
                    crate::engine::KeyKind::Press => {
                        // 复刻 Playwright 序列：keyDown(text) → char(text) → keyUp。
                        // Enter → "\r"、Tab → "\t"、可打印单字符 → 原文本；其余无文本。
                        let text = match k.key.as_str() {
                            "Enter" => Some("\r"),
                            "Tab" => Some("\t"),
                            _ if k.text.chars().count() == 1 && !k.text.is_empty() => {
                                Some(k.text.as_str())
                            }
                            _ => None,
                        };
                        let t = text.unwrap_or("");
                        self.cdp.send_with_session(
                            Some(&sid),
                            "Input.dispatchKeyEvent",
                            json!({
                                "type": "keyDown", "key": k.key, "code": k.code,
                                "text": t, "unmodifiedText": t, "modifiers": modifiers,
                            }),
                        )?;
                        if let Some(tt) = text {
                            self.cdp.send_with_session(
                                Some(&sid),
                                "Input.dispatchKeyEvent",
                                json!({
                                    "type": "char", "key": k.key, "code": k.code,
                                    "text": tt, "unmodifiedText": tt, "modifiers": modifiers,
                                }),
                            )?;
                        }
                        self.cdp.send_with_session(Some(&sid), "Input.dispatchKeyEvent", json!({
                            "type": "keyUp", "key": k.key, "code": k.code, "modifiers": modifiers,
                        }))?;
                    }
                }
                Ok(())
            }
            InputEvent::Wheel(w) => {
                self.cdp.send_with_session(Some(&sid), "Input.dispatchMouseEvent", json!({
                    "type": "mouseWheel", "x": w.x, "y": w.y, "deltaX": w.delta_x, "deltaY": w.delta_y,
                }))?;
                Ok(())
            }
            InputEvent::Touch(t) => {
                let kind = match t.kind {
                    crate::engine::TouchKind::Start => "touchStart",
                    crate::engine::TouchKind::Move => "touchMove",
                    crate::engine::TouchKind::End => "touchEnd",
                    crate::engine::TouchKind::Cancel => "touchCancel",
                    crate::engine::TouchKind::Swipe => "touchStart",
                };
                let points: Vec<Value> = t
                    .points
                    .iter()
                    .map(|p| json!({ "x": p.x, "y": p.y }))
                    .collect();
                self.cdp.send_with_session(
                    Some(&sid),
                    "Input.dispatchTouchEvent",
                    json!({
                        "type": kind, "touchPoints": points,
                    }),
                )?;
                Ok(())
            }
        }
    }

    fn evaluate(&self, tab: TabId, script: &str) -> Result<Value> {
        self.eval(tab, script)
    }

    fn page_title(&self, tab: TabId) -> Result<String> {
        // 容错语义（对齐旧 snapshot 路径）：页面忙/冻结时回落缓存，不抛错
        let fresh = self
            .eval_quick(tab, "document.title||''")
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .filter(|s| !s.is_empty());
        let cached = self.read_tab(tab, |t| t.title.clone()).unwrap_or_default();
        Ok(fresh.unwrap_or(cached))
    }

    fn page_url(&self, tab: TabId) -> Result<String> {
        let fresh = self
            .eval_quick(tab, "location.href||''")
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .filter(|s| !s.is_empty());
        let cached = self.read_tab(tab, |t| t.url.clone()).unwrap_or_default();
        Ok(fresh.unwrap_or(cached))
    }

    fn set_viewport(&self, tab: TabId, vp: Viewport) -> Result<()> {
        let sid = self.session(tab)?.to_string();
        let _ = self.cdp.send_with_session(Some(&sid), "Emulation.setDeviceMetricsOverride", json!({
            "width": vp.width, "height": vp.height, "deviceScaleFactor": vp.device_scale_factor, "mobile": false,
        }));
        // 同步 screencast/截图的可见表面尺寸，保证 OSR 帧与视口一致。
        let _ = self.cdp.send_with_session(
            Some(&sid),
            "Emulation.setVisibleSize",
            json!({ "width": vp.width, "height": vp.height }),
        );
        let active_stream = self
            .read_tab(tab, |t| t.frame_stream.is_some())
            .unwrap_or(false);
        self.with_tab(tab, |t| {
            t.viewport = vp;
            Ok(())
        })?;
        *self.viewport.write().unwrap_or_else(|e| e.into_inner()) = Some(vp);
        // Chrome 的 screencast 不会随 device metrics 变更而动态改尺寸；
        // 帧流正在运行时重载一次，让后续帧跟随新视口（面板像真窗口重排）。
        if active_stream {
            let opts = self
                .read_tab(tab, |t| t.frame_stream.as_ref().map(|f| f.opts.clone()))
                .ok()
                .flatten();
            if let Some(opts) = opts {
                let _ = self.stop_frame_stream(tab);
                let _ = self.start_frame_stream(tab, opts);
            }
        }
        Ok(())
    }

    fn screenshot(&self, tab: TabId) -> Result<Image> {
        let sid = self.session(tab)?.to_string();
        let resp = self.cdp.send_with_session(
            Some(&sid),
            "Page.captureScreenshot",
            json!({
                "format": "png", "fromSurface": true,
            }),
        )?;
        let b64 = extract_screenshot_base64(&resp)
            .ok_or_else(|| EngineError::new(ErrorKind::View, "screenshot returned no data"))?;
        let png = decode_base64(&b64)
            .map_err(|e| EngineError::new(ErrorKind::View, format!("screenshot base64: {e}")))?;
        Image::from_png(&png)
    }

    fn set_touch_emulation(&self, tab: TabId, enabled: bool) -> Result<()> {
        let sid = self.session(tab)?.to_string();
        self.cdp.send_with_session(
            Some(&sid),
            "Emulation.setTouchEmulationEnabled",
            json!({ "enabled": enabled, "maxTouchPoints": if enabled { 5 } else { 0 } }),
        )?;
        Ok(())
    }

    fn set_geolocation(&self, tab: TabId, lat: f64, lng: f64, accuracy: f64) -> Result<()> {
        let sid = self.session(tab)?.to_string();
        self.cdp.send_with_session(
            Some(&sid),
            "Emulation.setGeolocationOverride",
            json!({ "latitude": lat, "longitude": lng, "accuracy": accuracy }),
        )?;
        Ok(())
    }

    fn set_timezone(&self, tab: TabId, timezone_id: &str) -> Result<()> {
        let sid = self.session(tab)?.to_string();
        self.cdp.send_with_session(
            Some(&sid),
            "Emulation.setTimezoneOverride",
            json!({ "timezoneId": timezone_id }),
        )?;
        Ok(())
    }

    /// 直接返回编码字节（png / jpeg），跳过 RGBA 解码+再编码，显著减小
    /// Agent 截图链路的传输与解码开销。宽高从文件头解析（不解码整图）。
    fn capture_encoded(&self, tab: TabId, format: &str) -> Result<(u32, u32, Vec<u8>)> {
        let sid = self.session(tab)?.to_string();
        let (fmt, quality) = match format {
            "jpeg" => ("jpeg", json!(80)),
            "png" => ("png", Value::Null),
            other => {
                return Err(EngineError::new(
                    ErrorKind::InvalidArgument,
                    format!("unsupported capture format '{other}' (png|jpeg)"),
                ))
            }
        };
        let mut params = serde_json::Map::new();
        params.insert("format".into(), json!(fmt));
        params.insert("fromSurface".into(), json!(true));
        if !quality.is_null() {
            params.insert("quality".into(), quality);
        }
        let resp = self.cdp.send_with_session(
            Some(&sid),
            "Page.captureScreenshot",
            Value::Object(params),
        )?;
        let b64 = extract_screenshot_base64(&resp)
            .ok_or_else(|| EngineError::new(ErrorKind::View, "screenshot returned no data"))?;
        let bytes = decode_base64(&b64)
            .map_err(|e| EngineError::new(ErrorKind::View, format!("screenshot base64: {e}")))?;
        let (w, h) = if format == "jpeg" {
            crate::png::jpeg_dimensions(&bytes).unwrap_or((0, 0))
        } else {
            crate::png::png_dimensions(&bytes).unwrap_or((0, 0))
        };
        Ok((w, h, bytes))
    }

    fn view_handle(&self, tab: TabId) -> Option<ViewHandle> {
        self.read_tab(tab, |t| t.view_handle).unwrap_or(None)
    }

    fn view_frame(&self, tab: TabId) -> Result<ViewFrame> {
        let seq = self.with_tab(tab, |t| {
            t.frame_seq += 1;
            Ok(t.frame_seq)
        })?;
        let img = self.screenshot(tab)?;
        Ok(ViewFrame::from_image(&img, seq))
    }

    fn start_frame_stream(&self, tab: TabId, opts: FrameStreamOptions) -> Result<()> {
        let sid = self.session(tab)?.to_string();
        let tab_arc = self.tab_arc(tab)?;
        // 停掉旧的推送流
        self.stop_frame_stream(tab)?;
        // screencast 编码：默认 PNG（自研解码器可还原 RGBA）；"jpeg" 体积小、
        // 直接透传给 UI 预览（无需解码）。请求 jpeg 时务必提供 quality，否则
        // Chrome 默认压缩可能过重。
        let format = if opts.format == "jpeg" { "jpeg" } else { "png" };
        let mut params = json!({ "format": format });
        if format == "jpeg" {
            params["quality"] = json!(80);
        }
        if opts.max_width > 0 {
            params["maxWidth"] = json!(opts.max_width);
        }
        if opts.max_height > 0 {
            params["maxHeight"] = json!(opts.max_height);
        }
        // 无头下后台标签页的渲染器会被冻结，不产生 screencast 帧；
        // 先 bringToFront 让目标进入前台合成（等价于 switch_tab 的激活语义），
        // 否则 App 里“打开即开帧流”会收不到任何帧。
        let _ = self
            .cdp
            .send_with_session(Some(&sid), "Page.bringToFront", json!({}));
        let _ = self.cdp.send_with_session(
            Some(&sid),
            "Emulation.setFocusEmulationEnabled",
            json!({ "enabled": true }),
        );
        self.cdp
            .send_with_session(Some(&sid), "Page.startScreencast", params)?;
        // 后台泵线程：消费 screencastFrame → 按格式转发到对应 sink；其余事件回灌。
        let running = Arc::new(AtomicBool::new(true));
        let run = running.clone();
        let cdp = self.cdp.clone();
        let pump_arc = tab_arc.clone();
        let sleep_ms = if opts.fps >= 60 {
            0
        } else {
            1000u64 / u64::from(opts.fps.max(1))
        };
        let format = format.to_string();
        let fmt = format.clone();
        let join = std::thread::spawn(move || {
            pump_screencast(cdp, pump_arc, tab, run, sleep_ms, &fmt);
        });
        self.with_tab(tab, |t| {
            t.frame_stream = Some(FrameStream {
                running,
                join: Some(join),
                opts: opts.clone(),
            });
            Ok(())
        })?;
        Ok(())
    }

    fn stop_frame_stream(&self, tab: TabId) -> Result<()> {
        let sid = self.session(tab)?;
        let _ = self
            .cdp
            .send_with_session(Some(&sid), "Page.stopScreencast", json!({}));
        let join = {
            let p = self.tab_arc(tab)?;
            let mut t = p.write().unwrap_or_else(|e| e.into_inner());
            match t.frame_stream.take() {
                Some(mut fs) => {
                    fs.running.store(false, Ordering::Relaxed);
                    // FrameStream 实现 Drop：用 take 取出 join，避免 move-out。
                    fs.join.take()
                }
                None => None,
            }
        };
        if let Some(j) = join {
            let _ = j.join();
        }
        Ok(())
    }

    fn cookie_get(&self, tab: TabId, domain: Option<&str>) -> Result<Vec<Cookie>> {
        let cookies = self.real_cookies(tab).unwrap_or_default();
        let cookies = match domain {
            Some(d) => cookies.into_iter().filter(|c| c.domain == d).collect(),
            None => cookies,
        };
        Ok(cookies)
    }

    fn cookie_set(&self, tab: TabId, cookie: &Cookie) -> Result<()> {
        let sid = self.session(tab)?.to_string();
        let mut params = serde_json::Map::new();
        params.insert("name".into(), json!(cookie.name));
        params.insert("value".into(), json!(cookie.value));
        params.insert("domain".into(), json!(cookie.domain));
        params.insert("path".into(), json!(cookie.path));
        params.insert("secure".into(), json!(cookie.secure));
        params.insert("httpOnly".into(), json!(cookie.http_only));
        if let Some(e) = cookie.expires {
            params.insert("expires".into(), json!(e));
        }
        if let Some(ss) = &cookie.same_site {
            params.insert("sameSite".into(), json!(ss));
        }
        let resp =
            self.cdp
                .send_with_session(Some(&sid), "Network.setCookie", Value::Object(params))?;
        let ok = resp
            .get("result")
            .and_then(|r| r.get("success"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        // 失败时保留 overlay 缓存（webview 语义）
        self.with_tab(tab, |t| {
            t.cookies
                .retain(|c| !(c.name == cookie.name && c.domain == cookie.domain));
            t.cookies.push(cookie.clone());
            Ok(())
        })?;
        if ok {
            Ok(())
        } else {
            Err(EngineError::new(
                ErrorKind::Navigation,
                "Network.setCookie rejected",
            ))
        }
    }

    fn cookie_clear(&self, tab: TabId, domain: Option<&str>, name: Option<&str>) -> Result<()> {
        let sid = self.session(tab)?.to_string();
        let targets: Vec<Cookie> = self
            .real_cookies(tab)
            .unwrap_or_default()
            .into_iter()
            .filter(|c| domain.map(|d| c.domain == d).unwrap_or(true))
            .filter(|c| name.map(|n| c.name == n).unwrap_or(true))
            .collect();
        for c in targets {
            let _ = self.cdp.send_with_session(
                Some(&sid),
                "Network.deleteCookies",
                json!({
                    "name": c.name, "domain": c.domain, "path": c.path,
                }),
            );
        }
        self.with_tab(tab, |t| {
            t.cookies.retain(|c| {
                let d_match = domain.map(|d| c.domain == d).unwrap_or(true);
                let n_match = name.map(|n| c.name == n).unwrap_or(true);
                !(d_match && n_match)
            });
            Ok(())
        })?;
        Ok(())
    }

    fn storage_get(&self, tab: TabId, key: &str) -> Result<Option<String>> {
        let v = self.eval_sync(tab, &format!("localStorage.getItem({})", json!(key)))?;
        Ok(v.as_str().map(String::from))
    }

    fn storage_set(&self, tab: TabId, key: &str, value: &str) -> Result<()> {
        self.eval_sync(
            tab,
            &format!("localStorage.setItem({},{})", json!(key), json!(value)),
        )?;
        self.with_tab(tab, |t| {
            t.storage.insert(key.to_string(), value.to_string());
            Ok(())
        })?;
        Ok(())
    }

    fn storage_all(&self, tab: TabId) -> Result<HashMap<String, String>> {
        let js = r#"(()=>{let o={};for(let i=0;i<localStorage.length;i++){const k=localStorage.key(i);o[k]=localStorage.getItem(k);}return o;})()"#;
        let v = self.eval_sync(tab, js)?;
        let mut out = HashMap::new();
        if let Some(obj) = v.as_object() {
            for (k, val) in obj {
                if let Some(s) = val.as_str() {
                    out.insert(k.clone(), s.to_string());
                }
            }
        }
        Ok(out)
    }

    fn storage_clear(&self, tab: TabId) -> Result<()> {
        self.eval_sync(tab, "localStorage.clear()")?;
        self.with_tab(tab, |t| {
            t.storage.clear();
            Ok(())
        })?;
        Ok(())
    }

    fn block_requests(&self, tab: TabId, patterns: &[String], enabled: bool) -> Result<()> {
        let sid = self.session(tab)?.to_string();
        if enabled {
            let mut current = Vec::new();
            let resp = self
                .cdp
                .send_with_session(Some(&sid), "Network.getBlockedURLs", json!({}));
            if let Ok(r) = resp {
                if let Some(arr) = r.pointer("/result/blockedURLs").and_then(Value::as_array) {
                    current = arr
                        .iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect();
                }
            }
            for p in patterns {
                if !current.contains(p) {
                    current.push(p.clone());
                }
            }
            self.cdp.send_with_session(
                Some(&sid),
                "Network.setBlockedURLs",
                json!({ "urls": current }),
            )?;
        } else {
            let mut current = Vec::new();
            let resp = self
                .cdp
                .send_with_session(Some(&sid), "Network.getBlockedURLs", json!({}));
            if let Ok(r) = resp {
                if let Some(arr) = r.pointer("/result/blockedURLs").and_then(Value::as_array) {
                    current = arr
                        .iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect();
                }
            }
            current.retain(|p| !patterns.contains(p));
            self.cdp.send_with_session(
                Some(&sid),
                "Network.setBlockedURLs",
                json!({ "urls": current }),
            )?;
        }
        Ok(())
    }

    fn intercept_requests(&self, tab: TabId, patterns: &[String], enabled: bool) -> Result<()> {
        let sid = self.session(tab)?.to_string();
        if enabled {
            let pats: Vec<Value> = patterns
                .iter()
                .map(|p| json!({ "urlPattern": p }))
                .collect();
            let _ =
                self.cdp
                    .send_with_session(Some(&sid), "Fetch.enable", json!({ "patterns": pats }));
        } else {
            let _ = self
                .cdp
                .send_with_session(Some(&sid), "Fetch.disable", json!({}));
        }
        Ok(())
    }

    fn set_basic_auth(&self, tab: TabId, username: &str, password: &str) -> Result<()> {
        let sid = self.session(tab)?.to_string();
        self.with_tab(tab, |t| {
            t.basic_auth = Some((username.to_string(), password.to_string()));
            Ok(())
        })?;
        // 启用 auth 挑战拦截：handleAuthRequests=true → Fetch.authRequired 事件
        // （patterns 为空数组表示不拦截请求 body，仅处理认证挑战）。
        let _ = self.cdp.send_with_session(
            Some(&sid),
            "Fetch.enable",
            json!({ "handleAuthRequests": true, "patterns": [] }),
        );
        Ok(())
    }

    fn drain_events(&self, tab: TabId) -> Vec<PageEvent> {
        self.route_cdp_events();
        self.tab_arc(tab)
            .map(|p| {
                p.write()
                    .unwrap_or_else(|e| e.into_inner())
                    .events
                    .drain(..)
                    .collect()
            })
            .unwrap_or_default()
    }

    fn event_generation(&self, tab: TabId) -> u64 {
        self.read_tab(tab, |t| t.notifier.generation()).unwrap_or(0)
    }

    fn wait_event(&self, tab: TabId, since: u64, timeout: Duration) -> (u64, bool) {
        // 短读锁克隆 Arc 后立即释放，避免持有标签页锁阻塞等待 → 与 push 的写锁死锁。
        let notifier = self.read_tab(tab, |t| t.notifier.clone());
        match notifier {
            Ok(n) => {
                let woke = n.wait_changed(since, timeout);
                (n.generation(), woke)
            }
            Err(_) => (since, false),
        }
    }

    fn any_event_generation(&self) -> u64 {
        self.any.generation()
    }

    fn wait_any_event(&self, since: u64, timeout: Duration) -> (u64, bool) {
        let woke = self.any.wait_changed(since, timeout);
        (self.any.generation(), woke)
    }

    fn set_event_sink(&self, tab: TabId, sink: Option<Arc<dyn PageEventSink>>) -> Result<()> {
        let _ = self.with_tab(tab, |t| {
            t.event_sink = sink;
            Ok(())
        });
        Ok(())
    }

    fn set_frame_sink(&self, tab: TabId, sink: Option<Arc<dyn ViewFrameSink>>) -> Result<()> {
        let _ = self.with_tab(tab, |t| {
            t.frame_sink = sink;
            Ok(())
        });
        Ok(())
    }

    fn set_encoded_frame_sink(
        &self,
        tab: TabId,
        sink: Option<Arc<dyn EncodedFrameSink>>,
    ) -> Result<()> {
        let _ = self.with_tab(tab, |t| {
            t.encoded_frame_sink = sink;
            Ok(())
        });
        Ok(())
    }

    // ── JS 对话框 ─────────────────────────────────────────────
    fn pending_dialog(&self, tab: TabId) -> Option<DialogInfo> {
        self.read_tab(tab, |t| t.pending_dialog.clone())
            .unwrap_or(None)
    }
    fn dialog_accept(&self, tab: TabId, prompt_text: Option<&str>) -> Result<()> {
        let sid = self.session(tab)?.to_string();
        let mut params = json!({ "accept": true });
        if let Some(text) = prompt_text {
            params["promptText"] = json!(text);
        }
        self.cdp
            .send_with_session(Some(&sid), "Page.handleJavaScriptDialog", params)?;
        let _ = self.with_tab(tab, |t| {
            t.pending_dialog = None;
            Ok(())
        });
        Ok(())
    }

    fn dialog_dismiss(&self, tab: TabId) -> Result<()> {
        let sid = self.session(tab)?.to_string();
        self.cdp.send_with_session(
            Some(&sid),
            "Page.handleJavaScriptDialog",
            json!({ "accept": false }),
        )?;
        let _ = self.with_tab(tab, |t| {
            t.pending_dialog = None;
            Ok(())
        });
        Ok(())
    }

    // ── PDF 导出 ──────────────────────────────────────────────
    fn print_to_pdf(&self, tab: TabId) -> Result<Vec<u8>> {
        let sid = self.session(tab)?.to_string();
        let resp = self.cdp.send_with_session(
            Some(&sid),
            "Page.printToPDF",
            json!({
                "preferCSSPageSize": true,
            }),
        )?;
        let b64 = resp
            .get("result")
            .and_then(|r| r.get("data"))
            .and_then(Value::as_str)
            .ok_or_else(|| EngineError::new(ErrorKind::View, "printToPDF returned no data"))?;
        decode_base64(b64)
            .map_err(|e| EngineError::new(ErrorKind::View, format!("pdf base64: {e}")))
    }

    // ── 导航历史 ──────────────────────────────────────────────
    fn get_history(&self, tab: TabId) -> Result<Vec<HistoryEntry>> {
        let sid = self.session(tab)?.to_string();
        let resp =
            self.cdp
                .send_with_session(Some(&sid), "Page.getNavigationHistory", json!({}))?;
        let entries = resp
            .get("result")
            .and_then(|r| r.get("entries"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        Ok(entries
            .iter()
            .map(|e| HistoryEntry {
                url: e
                    .get("url")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                title: e
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                transition: e
                    .get("transitionType")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
            })
            .collect())
    }

    // ── 无障碍树（AX）─────────────────────────────────────────
    fn accessibility_tree(&self, tab: TabId) -> Result<Value> {
        let sid = self.session(tab)?.to_string();
        let _ = self
            .cdp
            .send_with_session(Some(&sid), "Accessibility.enable", json!({}));
        let resp =
            self.cdp
                .send_with_session(Some(&sid), "Accessibility.getFullAXTree", json!({}))?;
        let nodes = resp
            .get("result")
            .and_then(|r| r.get("nodes"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut out = Vec::new();
        for n in nodes {
            let ignored = n.get("ignored").and_then(Value::as_bool).unwrap_or(false);
            if ignored {
                continue;
            }
            let role = n
                .get("role")
                .and_then(|r| r.get("value"))
                .and_then(Value::as_str)
                .unwrap_or("");
            // 只保留有意义的交互/结构节点，控制 token 量
            if role == "generic" || role == "none" || role == "presentation" {
                continue;
            }
            let name = n
                .get("name")
                .and_then(|r| r.get("value"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let value = n
                .get("value")
                .and_then(|r| r.get("value"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let desc = n
                .get("description")
                .and_then(|r| r.get("value"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let mut props = serde_json::Map::new();
            for k in [
                "checked", "disabled", "selected", "expanded", "level", "pressed",
            ] {
                if let Some(v) = n.get(k).and_then(|r| r.get("value")) {
                    props.insert(k.to_string(), v.clone());
                }
            }
            out.push(json!({
                "role": role,
                "name": name,
                "value": value,
                "description": desc,
                "props": props,
            }));
        }
        Ok(json!({ "tree": out, "count": out.len() }))
    }

    // ── 网络请求拦截处理 ───────────────────────────────────────
    fn pending_requests(&self, tab: TabId) -> Vec<Value> {
        self.read_tab(tab, |t| t.paused_requests.clone())
            .unwrap_or_default()
    }

    fn fulfill_request(
        &self,
        tab: TabId,
        request_id: &str,
        status: u16,
        body: Option<Vec<u8>>,
        headers: Option<Value>,
    ) -> Result<()> {
        let sid = self.session(tab)?.to_string();
        let mut params = serde_json::Map::new();
        params.insert("requestId".into(), json!(request_id));
        params.insert("responseCode".into(), json!(status));
        if let Some(h) = headers {
            params.insert("responseHeaders".into(), h);
        }
        if let Some(b) = body {
            use base64::Engine;
            params.insert(
                "body".into(),
                json!(base64::engine::general_purpose::STANDARD.encode(b)),
            );
        }
        self.cdp
            .send_with_session(Some(&sid), "Fetch.fulfillRequest", Value::Object(params))?;
        let _ = self.with_tab(tab, |t| {
            t.paused_requests
                .retain(|r| r.get("request_id").and_then(Value::as_str) != Some(request_id));
            Ok(())
        });
        Ok(())
    }

    fn modify_response(
        &self,
        tab: TabId,
        request_id: &str,
        status: u16,
        body: Option<Vec<u8>>,
        headers: Option<Value>,
    ) -> Result<()> {
        // 在 Request 阶段拦截下，`Fetch.fulfillRequest` 是「用指定响应替换」
        // 的正确路径（`Fetch.continueRequest` 的 responseCode/body 仅在
        // Response 阶段拦截下才生效，会被忽略）。
        let sid = self.session(tab)?.to_string();
        let mut params = serde_json::Map::new();
        params.insert("requestId".into(), json!(request_id));
        params.insert("responseCode".into(), json!(status));
        if let Some(h) = headers {
            params.insert("responseHeaders".into(), h);
        }
        if let Some(b) = body {
            use base64::Engine;
            params.insert(
                "body".into(),
                json!(base64::engine::general_purpose::STANDARD.encode(b)),
            );
        }
        self.cdp
            .send_with_session(Some(&sid), "Fetch.fulfillRequest", Value::Object(params))?;
        let _ = self.with_tab(tab, |t| {
            t.paused_requests
                .retain(|r| r.get("request_id").and_then(Value::as_str) != Some(request_id));
            Ok(())
        });
        Ok(())
    }

    fn continue_request(&self, tab: TabId, request_id: &str) -> Result<()> {
        let sid = self.session(tab)?.to_string();
        self.cdp.send_with_session(
            Some(&sid),
            "Fetch.continueRequest",
            json!({ "requestId": request_id }),
        )?;
        let _ = self.with_tab(tab, |t| {
            t.paused_requests
                .retain(|r| r.get("request_id").and_then(Value::as_str) != Some(request_id));
            Ok(())
        });
        Ok(())
    }

    fn abort_request(&self, tab: TabId, request_id: &str) -> Result<()> {
        let sid = self.session(tab)?.to_string();
        self.cdp.send_with_session(
            Some(&sid),
            "Fetch.failRequest",
            json!({
                "requestId": request_id,
                "errorReason": "BlockedByClient",
            }),
        )?;
        let _ = self.with_tab(tab, |t| {
            t.paused_requests
                .retain(|r| r.get("request_id").and_then(Value::as_str) != Some(request_id));
            Ok(())
        });
        Ok(())
    }
}

// ── 辅助函数 ───────────────────────────────────────────────────

/// 帧推送泵线程：持续消费该标签页的 screencast 帧并推给对应 sink；
/// 其余 CDP 事件回灌到共享队列（`route_cdp_events` 仍会消费到）。
fn pump_screencast(
    cdp: Arc<CdpClient>,
    tab_arc: Arc<RwLock<CdpTab>>,
    tab: TabId,
    running: Arc<AtomicBool>,
    sleep_ms: u64,
    format: &str,
) {
    while running.load(Ordering::Relaxed) {
        let sid = tab_arc
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .session_id
            .clone();
        let mut relay: Vec<CdpEvent> = Vec::new();
        for ev in cdp.drain_events() {
            if ev.session_id.as_deref() == Some(sid.as_str()) && ev.method == "Page.screencastFrame"
            {
                handle_screencast_frame(&cdp, &tab_arc, tab, &ev, format);
            } else {
                relay.push(ev);
            }
        }
        if !relay.is_empty() {
            cdp.reingest(relay);
        }
        if sleep_ms > 0 {
            std::thread::sleep(Duration::from_millis(sleep_ms));
        }
    }
}

/// Handle one screencast frame:
/// - `jpeg`: pass bytes straight to `EncodedFrameSink` (UI preview, no decode);
/// - `png`: decode to RGBA and push to `ViewFrameSink` (agent perception / compatibility path).
///
/// Then ack to keep frames coming (without an ack, Chrome stops the stream).
fn handle_screencast_frame(
    cdp: &Arc<CdpClient>,
    tab_arc: &Arc<RwLock<CdpTab>>,
    tab: TabId,
    ev: &CdpEvent,
    format: &str,
) {
    if let Some(b64) = ev.params.get("data").and_then(Value::as_str) {
        if let Ok(bytes) = decode_base64(b64) {
            if format == "jpeg" {
                let (w, h) = crate::png::jpeg_dimensions(&bytes).unwrap_or((0, 0));
                if w > 0 && h > 0 {
                    let mut t = tab_arc.write().unwrap_or_else(|e| e.into_inner());
                    t.frame_seq += 1;
                    if let Some(sink) = t.encoded_frame_sink.clone() {
                        sink.on_encoded_frame(
                            tab,
                            &crate::engine::EncodedViewFrame {
                                width: w,
                                height: h,
                                mime: "image/jpeg".to_string(),
                                bytes,
                                seq: t.frame_seq,
                            },
                        );
                    }
                }
            } else if let Ok(img) = Image::from_png(&bytes) {
                let mut t = tab_arc.write().unwrap_or_else(|e| e.into_inner());
                t.frame_seq += 1;
                if let Some(sink) = t.frame_sink.clone() {
                    let frame = ViewFrame::from_image(&img, t.frame_seq);
                    sink.on_view_frame(tab, &frame);
                }
            }
        }
    }
    // 用 `params.sessionId`（screencast 帧的 ack id）确认，否则 Chrome 停止续帧。
    if let Some(fid) = ev.params.get("sessionId") {
        let _ = cdp.send_with_session(
            ev.session_id.as_deref(),
            "Page.screencastFrameAck",
            json!({
                "sessionId": fid,
            }),
        );
    }
}

/// CDP 修饰键位掩码（Alt=1, Ctrl=2, Meta/Command=4, Shift=8）。
fn modifier_mask(m: &crate::engine::Modifiers) -> u32 {
    let mut mask = 0u32;
    if m.alt {
        mask |= 1;
    }
    if m.ctrl {
        mask |= 2;
    }
    if m.meta {
        mask |= 4;
    }
    if m.shift {
        mask |= 8;
    }
    mask
}

/// base64 → 字节。
fn decode_base64(s: &str) -> std::result::Result<Vec<u8>, base64::DecodeError> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.decode(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_base_extraction() {
        assert_eq!(
            endpoint_base_of("ws://127.0.0.1:9222/devtools/browser/abc"),
            "ws://127.0.0.1:9222"
        );
        assert_eq!(
            endpoint_base_of("ws://127.0.0.1:9222/devtools/page/abc"),
            "ws://127.0.0.1:9222"
        );
        assert_eq!(endpoint_base_of("ws://x/devtools/page/a"), "ws://x");
    }

    #[test]
    fn internal_pages_are_never_bindable() {
        // Host/extension UI must never be auto-attached (navigate would replace
        // the side panel / new tab and the input box would disappear). The check
        // is scheme-generic, not tied to any specific application.
        assert!(is_internal_page_url("my-extension://abc/sidepanel.html"));
        assert!(is_internal_page_url("chrome-extension://abc/newtab.html"));
        assert!(is_internal_page_url("moz-extension://abc/page.html"));
        assert!(is_internal_page_url("chrome://newtab"));
        assert!(is_internal_page_url(
            "devtools://devtools/bundled/inspector.html"
        ));
        assert!(!is_internal_page_url("https://www.baidu.com/"));

        assert!(is_bindable_web_page("https://www.baidu.com/"));
        assert!(!is_bindable_web_page("about:blank"));
        assert!(!is_bindable_web_page("my-extension://abc/newtab.html"));
    }

    #[test]
    fn snapshot_js_is_balanced() {
        let js = snapshot_js();
        assert!(js.starts_with("/*FB_EXTRACT_V3*/"));
        assert!(js.contains("data-fb"));
        assert!(js.contains("frames"));
        assert!(js.contains("contentDocument"));
        assert!(js.contains("meta"));
    }

    #[test]
    fn action_js_targets_data_fb() {
        let js = action_js('a', "el.click()");
        assert!(js.contains("[data-fb=\"a\"]"));
        assert!(js.contains("el.click()"));
    }

    #[test]
    fn parse_elements_handles_canned() {
        let raw = json!(
            r#"{"elements":[{"id":"a","tag":"a","text":"Next","href":"https://x","rect":{"x":0,"y":0,"width":1,"height":1},"value":null,"input_type":null,"checked":null,"visible":true}],"meta":{"total":1,"truncated":false,"viewport_h":800,"scroll_h":1200,"scroll_y":0},"frames":[]}"#
        );
        let (els, meta, frames) = parse_elements(&raw).unwrap();
        assert_eq!(els.len(), 1);
        assert_eq!(els[0].id, 'a');
        assert_eq!(els[0].tag, "a");
        assert_eq!(meta.total, 1);
        assert!(frames.is_empty());
    }

    #[test]
    fn cef_capabilities_full() {
        // 直接构造能力判断（无需真实 CEF）
        let caps = EngineCapabilities::full();
        assert!(caps.supports_cdp && caps.supports_osr && caps.supports_coordinate_input);
    }

    #[test]
    fn modifier_mask_values() {
        assert_eq!(
            modifier_mask(&crate::engine::Modifiers::with_ctrl(
                crate::engine::Modifiers::none()
            )),
            2
        );
        assert_eq!(
            modifier_mask(&crate::engine::Modifiers {
                shift: true,
                ..Default::default()
            }),
            8
        );
    }
}
