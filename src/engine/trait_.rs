// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! `BrowserEngine` trait —— 统一的浏览器引擎抽象（第 1 天就要定死的契约）。
//!
//! 所有具体引擎（mock / cef / webview）都实现本 trait；上层 Agent 工具
//! 只依赖本抽象，因此新增平台只需新增一个引擎实现，工具层零改动。

use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;

use crate::engine::host::{EncodedFrameSink, PageEventSink, ViewFrameSink};
use crate::engine::snapshot::{ImageInfo, LinkInfo};
use crate::engine::{
    ContextId, Cookie, DialogInfo, ElementRef, EngineError, FrameStreamOptions, HistoryEntry,
    Image, InputEvent, PageEvent, PageSnapshot, Result, TabId, TabInfo, TabOptions, ViewFrame,
    ViewHandle, Viewport,
};

/// 引擎能力位：工具层依据这些位向 LLM 收敛可用工具。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct EngineCapabilities {
    pub supports_cdp: bool,
    pub supports_coordinate_input: bool,
    pub supports_osr: bool,
    pub supports_windowed: bool,
    pub supports_touch: bool,
    pub supports_dom_injection: bool,
    pub supports_storage: bool,
    pub supports_cookies: bool,
    pub supports_network_control: bool,
}

/// 工具对引擎的能力要求（能力位驱动工具收敛：LLM 只会看到引擎支持的子集）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Capability {
    /// 需要真实 CDP 能力（如 `save_as_pdf` 的 `Page.printToPDF`）。
    Cdp,
    /// 需要网络请求控制（`block_request` / `intercept_request` / 拦截处理）。
    NetworkControl,
}

impl EngineCapabilities {
    /// 判断该引擎能力集是否满足某项工具要求。
    pub fn supports(&self, cap: Capability) -> bool {
        match cap {
            Capability::Cdp => self.supports_cdp,
            Capability::NetworkControl => self.supports_network_control,
        }
    }

    pub fn full() -> Self {
        EngineCapabilities {
            supports_cdp: true,
            supports_coordinate_input: true,
            supports_osr: true,
            supports_windowed: true,
            supports_touch: true,
            supports_dom_injection: true,
            supports_storage: true,
            supports_cookies: true,
            supports_network_control: true,
        }
    }

    /// 移动端 JS 注入引擎的典型能力集（降级）。
    pub fn js_injection() -> Self {
        EngineCapabilities {
            supports_cdp: false,
            supports_coordinate_input: false,
            supports_osr: false,
            supports_windowed: true,
            supports_touch: true,
            supports_dom_injection: true,
            supports_storage: true,
            supports_cookies: true,
            supports_network_control: false,
        }
    }
}

// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.
/// 浏览器引擎统一接口。
pub trait BrowserEngine: Send + Sync {
    /// 引擎名（"mock" / "chromium" / "cef" / "webview"）。
    fn name(&self) -> &'static str;

    /// 能力位。
    fn capabilities(&self) -> EngineCapabilities;

    /// 若该标签页支持 CDP，返回其 CDP websocket 端点（供宿主自建工具/调试）。
    fn cdp_endpoint(&self, tab: TabId) -> Option<String>;

    /// **浏览器级** CDP 端点（可访问 `Target` 域：发现/切换/新建标签页）。
    ///
    /// 跨进程连接应优先使用它而不是 `cdp_endpoint`（page 级）：page 级端点绑定
    /// 到单个页面，页面一关闭连接即失效，且部分浏览器不允许 page 级连接访问
    /// `Target` 域。默认 `None`（非 CDP 引擎）。
    fn browser_cdp_endpoint(&self) -> Option<String> {
        None
    }

    // ── 标签页 ────────────────────────────────────────────────
    fn create_tab(&self, url: &str, opts: &TabOptions) -> Result<TabId>;
    fn close_tab(&self, tab: TabId) -> Result<()>;
    fn list_tabs(&self) -> Vec<TabInfo>;
    fn switch_tab(&self, tab: TabId) -> Result<()>;
    fn active_tab(&self) -> Option<TabId>;
    /// 打开一个独立浏览器窗口（新 target），返回其标签页 ID。
    /// 无窗口概念的引擎（webview/mock）默认退化为普通 `create_tab`。
    fn new_window(&self, url: &str, opts: &TabOptions) -> Result<TabId> {
        self.create_tab(url, opts)
    }

    // ── 导航 ──────────────────────────────────────────────────
    fn navigate(&self, tab: TabId, url: &str) -> Result<()>;
    fn back(&self, tab: TabId) -> Result<()>;
    fn forward(&self, tab: TabId) -> Result<()>;
    fn reload(&self, tab: TabId) -> Result<()>;
    fn stop(&self, tab: TabId) -> Result<()>;

    // ── DOM / 内容 ────────────────────────────────────────────
    /// 生成交互元素快照（LLM 感知入口）。
    fn snapshot(&self, tab: TabId) -> Result<PageSnapshot>;
    fn get_page_text(&self, tab: TabId) -> Result<String>;
    fn get_page_html(&self, tab: TabId) -> Result<String>;
    fn get_links(&self, tab: TabId) -> Result<Vec<LinkInfo>>;
    fn get_images(&self, tab: TabId) -> Result<Vec<ImageInfo>>;
    fn get_table(&self, tab: TabId) -> Result<Vec<Vec<String>>>;
    fn execute_xpath(&self, tab: TabId, expr: &str) -> Result<Value>;

    // ── 元素级操作（移动端 JS 注入也基于此）────────────────────
    fn click_element(&self, tab: TabId, element: &ElementRef) -> Result<()>;
    fn set_element_value(&self, tab: TabId, element: &ElementRef, value: &str) -> Result<()>;
    fn select_option(&self, tab: TabId, element: &ElementRef, value: &str) -> Result<()>;
    fn check_element(&self, tab: TabId, element: &ElementRef, checked: bool) -> Result<()>;
    fn set_file_input(&self, tab: TabId, element: &ElementRef, paths: &[String]) -> Result<()>;

    // ── 样式注入 ──────────────────────────────────────────────
    fn inject_css(&self, tab: TabId, css: &str) -> Result<()>;

    // ── 原始事件注入 ──────────────────────────────────────────
    fn inject_event(&self, tab: TabId, ev: InputEvent) -> Result<()>;

    // ── JS ────────────────────────────────────────────────────
    fn evaluate(&self, tab: TabId, script: &str) -> Result<Value>;

    // ── 视图 / 渲染 ───────────────────────────────────────────
    fn set_viewport(&self, tab: TabId, vp: Viewport) -> Result<()>;
    fn screenshot(&self, tab: TabId) -> Result<Image>;
    fn view_handle(&self, tab: TabId) -> Option<ViewHandle>;
    fn view_frame(&self, tab: TabId) -> Result<ViewFrame>;

    // ── 设备模拟（CDP Emulation 域；默认 Unsupported）──────────
    /// 设置触摸模拟（移动端页面语义）。
    fn set_touch_emulation(&self, _tab: TabId, _enabled: bool) -> Result<()> {
        Err(crate::engine::EngineError::unsupported(format!(
            "engine '{}' does not support touch emulation",
            self.name()
        )))
    }
    /// 覆盖地理位置（纬度/经度，精确度米）。
    fn set_geolocation(&self, _tab: TabId, _lat: f64, _lng: f64, _accuracy: f64) -> Result<()> {
        Err(crate::engine::EngineError::unsupported(format!(
            "engine '{}' does not support geolocation override",
            self.name()
        )))
    }
    /// 覆盖时区（IANA 时区 ID，如 "America/New_York"）。
    fn set_timezone(&self, _tab: TabId, _timezone_id: &str) -> Result<()> {
        Err(crate::engine::EngineError::unsupported(format!(
            "engine '{}' does not support timezone override",
            self.name()
        )))
    }

    /// 直接捕获**编码后**的截图字节（不经历 RGBA 解码再编码），
    /// 用于 Agent 感知 / 传输体积优化。`format` ∈ {"png", "jpeg"}。
    /// 返回 (宽, 高, 原始字节)。默认引擎不支持则返回 Unsupported，
    /// 工具层会回落为 `screenshot()` 的 RGBA 路径。
    fn capture_encoded(&self, _tab: TabId, _format: &str) -> Result<(u32, u32, Vec<u8>)> {
        Err(crate::engine::EngineError::unsupported(format!(
            "engine '{}' does not support encoded capture",
            self.name()
        )))
    }

    // ── 轻量页面内省（避免整页快照的全量 DOM 扫描）──────────────
    /// 读取页面标题。默认走整页快照（保守）；cdp/mock/webview 覆盖为廉价读取。
    fn page_title(&self, tab: TabId) -> Result<String> {
        self.snapshot(tab).map(|s| s.title)
    }
    /// 读取当前页面 URL。默认走整页快照；引擎覆盖为廉价读取。
    fn page_url(&self, tab: TabId) -> Result<String> {
        self.snapshot(tab).map(|s| s.url)
    }

    // ── 会话状态（cookie / storage）───────────────────────────
    fn cookie_get(&self, tab: TabId, domain: Option<&str>) -> Result<Vec<Cookie>>;
    fn cookie_set(&self, tab: TabId, cookie: &Cookie) -> Result<()>;
    fn cookie_clear(&self, tab: TabId, domain: Option<&str>, name: Option<&str>) -> Result<()>;
    fn storage_get(&self, tab: TabId, key: &str) -> Result<Option<String>>;
    fn storage_set(&self, tab: TabId, key: &str, value: &str) -> Result<()>;
    /// 导出全部 localStorage（持久化/清空用）。
    fn storage_all(&self, tab: TabId) -> Result<std::collections::HashMap<String, String>>;
    /// 清空 localStorage。
    fn storage_clear(&self, tab: TabId) -> Result<()>;

    // ── 网络控制 ──────────────────────────────────────────────
    fn block_requests(&self, tab: TabId, patterns: &[String], enabled: bool) -> Result<()>;
    fn intercept_requests(&self, tab: TabId, patterns: &[String], enabled: bool) -> Result<()>;
    /// 设置 Basic Auth 凭据；之后该标签页的认证挑战（401）自动应答。
    /// 默认 Unsupported（仅 CDP 引擎实现）。
    fn set_basic_auth(&self, _tab: TabId, _username: &str, _password: &str) -> Result<()> {
        Err(crate::engine::EngineError::unsupported(format!(
            "engine '{}' does not support basic auth",
            self.name()
        )))
    }

    // ── 事件 ──────────────────────────────────────────────────
    /// 拉取该标签页待处理事件（LLM 循环在每次动作后调用）。
    fn drain_events(&self, tab: TabId) -> Vec<PageEvent>;

    // ── 事件驱动等待 ─────────────────────────────────────────
    /// 当前事件代数（事件驱动等待的基准）。未实现通知机制的引擎返回 0，
    /// `wait_until` 退化为有界轮询（兼容 webview 等宿主桥引擎）。
    fn event_generation(&self, tab: TabId) -> u64 {
        let _ = tab;
        0
    }

    /// 阻塞等待标签页产生新事件（事件驱动，非轮询）。`since` 为上次观察到
    /// 的事件代数；期间产生新事件则立即返回 `(新代数, true)`，超时返回
    /// `(当前代数, false)`。默认实现退化为 sleep 兜底。
    fn wait_event(&self, tab: TabId, since: u64, timeout: Duration) -> (u64, bool) {
        let _ = tab;
        std::thread::sleep(timeout);
        (since, false)
    }

    // ── 全局事件等待（异步事件泵唤醒用）───────────────────────
    /// 任意标签页事件的全局代数（引擎级通知基准）。默认 0（无通知时退化为轮询）。
    fn any_event_generation(&self) -> u64 {
        0
    }

    /// 阻塞等待**任意**标签页产生事件。返回 `(新代数, 是否被事件唤醒)`。
    /// 供异步事件泵用：泵在有订阅者时阻塞于此，事件到来立即醒来抽事件并广播，
    /// 空闲时零唤醒（不再按固定间隔轮询）。
    fn wait_any_event(&self, since: u64, timeout: Duration) -> (u64, bool) {
        std::thread::sleep(timeout);
        (since, false)
    }

    // ── 宿主回调 ──────────────────────────────────────────────
    fn set_event_sink(&self, tab: TabId, sink: Option<Arc<dyn PageEventSink>>) -> Result<()>;
    fn set_frame_sink(&self, tab: TabId, sink: Option<Arc<dyn ViewFrameSink>>) -> Result<()>;

    /// 设置编码帧（JPEG/PNG）推送回调。默认无操作（引擎按需覆盖）。
    fn set_encoded_frame_sink(
        &self,
        _tab: TabId,
        _sink: Option<Arc<dyn EncodedFrameSink>>,
    ) -> Result<()> {
        Ok(())
    }

    // ── JS 对话框（alert/confirm/prompt）────────────────────────
    /// 当前待处理的对话框（无则 None）。
    fn pending_dialog(&self, tab: TabId) -> Option<DialogInfo> {
        let _ = tab;
        None
    }
    /// 接受对话框（prompt 可带输入文本）。
    fn dialog_accept(&self, tab: TabId, prompt_text: Option<&str>) -> Result<()> {
        let _ = (tab, prompt_text);
        Err(EngineError::unsupported(format!(
            "engine '{}' does not support js dialogs (dialog_accept)",
            self.name()
        )))
    }
    /// 取消对话框。
    fn dialog_dismiss(&self, _tab: TabId) -> Result<()> {
        Err(crate::engine::EngineError::unsupported(format!(
            "engine '{}' does not support js dialogs (dialog_dismiss)",
            self.name()
        )))
    }

    // ── 元素动作变体（默认退化为单击，真实引擎可精确实现）────────
    fn double_click_element(&self, tab: TabId, element: &ElementRef) -> Result<()> {
        self.click_element(tab, element)
    }
    fn right_click_element(&self, tab: TabId, element: &ElementRef) -> Result<()> {
        self.click_element(tab, element)
    }

    // ── 真实键盘输入 ─────────────────────────────────────────────
    /// 以「真实用户输入」的方式把文本插入元素：聚焦后用浏览器原生输入通道
    /// （CDP `Input.insertText`）插入，正确触发受控组件（React）与 IME 的
    /// input 事件链，避免 JS `el.value=` 被框架忽略。`clear` 是否先清空。
    /// 不支持真实输入通道的引擎（webview/mock）默认退化为 `set_element_value`。
    fn type_text(&self, tab: TabId, element: &ElementRef, text: &str, clear: bool) -> Result<()> {
        let _ = clear;
        self.set_element_value(tab, element, text)
    }

    /// 向**当前聚焦**元素插入文本（真实键盘通道 `Input.insertText`）。
    /// 供 UI 用户在 OSR 预览中点击聚焦后直接打字；与 `type_text`（按元素）互补。
    fn type_text_focused(&self, _tab: TabId, _text: &str) -> Result<()> {
        Err(crate::engine::EngineError::unsupported(format!(
            "engine '{}' does not support focused text insertion",
            self.name()
        )))
    }

    // ── 离屏帧推送流（OSR push；预览窗用）────────────────────────
    /// 启动向 `frame_sink` 的连续帧推送。CDP 引擎用 `Page.startScreencast` +
    /// 内部泵线程；webview/mock 用轮询。同一标签页重复调用先停止旧流。
    /// 引擎不支持时返回 Unsupported（工具层可回落为拉取 `view_frame`）。
    fn start_frame_stream(&self, _tab: TabId, _opts: FrameStreamOptions) -> Result<()> {
        Err(crate::engine::EngineError::unsupported(format!(
            "engine '{}' does not support frame streaming",
            self.name()
        )))
    }
    /// 停止帧推送。
    fn stop_frame_stream(&self, _tab: TabId) -> Result<()> {
        Err(crate::engine::EngineError::unsupported(format!(
            "engine '{}' does not support frame streaming",
            self.name()
        )))
    }

    // ── PDF 导出 ──────────────────────────────────────────────
    fn print_to_pdf(&self, _tab: TabId) -> Result<Vec<u8>> {
        Err(crate::engine::EngineError::unsupported(format!(
            "engine '{}' does not support print_to_pdf",
            self.name()
        )))
    }

    // ── 导航历史 ──────────────────────────────────────────────
    fn get_history(&self, _tab: TabId) -> Result<Vec<HistoryEntry>> {
        Err(crate::engine::EngineError::unsupported(format!(
            "engine '{}' does not support get_history",
            self.name()
        )))
    }

    // ── 无障碍树（AX）─────────────────────────────────────────
    fn accessibility_tree(&self, _tab: TabId) -> Result<Value> {
        Err(crate::engine::EngineError::unsupported(format!(
            "engine '{}' does not support accessibility_tree",
            self.name()
        )))
    }

    // ── 隔离浏览器上下文（对应 CDP BrowserContext）──────────────
    fn create_context(&self) -> Result<ContextId> {
        Err(crate::engine::EngineError::unsupported(format!(
            "engine '{}' does not support isolated browser contexts",
            self.name()
        )))
    }
    fn dispose_context(&self, context: ContextId) -> Result<()> {
        let _ = context;
        Err(crate::engine::EngineError::unsupported(format!(
            "engine '{}' does not support disposing browser contexts",
            self.name()
        )))
    }
    /// 在指定隔离上下文中创建标签页（默认退化为普通 create_tab）。
    fn create_tab_in_context(
        &self,
        url: &str,
        opts: &TabOptions,
        context: ContextId,
    ) -> Result<TabId> {
        let _ = context;
        self.create_tab(url, opts)
    }
    /// 上下文的**原生稳定标识**（CDP `browserContextId`）。宿主可持久化该值，
    /// 在下一进程用 `adopt_context` 复用同一隔离上下文。默认 `None`（无上下文概念）。
    fn context_native_id(&self, _context: ContextId) -> Option<String> {
        None
    }
    /// 接管一个已存在的原生上下文（跨进程/跨连接复用）。默认不支持。
    fn adopt_context(&self, _native_id: &str) -> Result<ContextId> {
        Err(crate::engine::EngineError::unsupported(format!(
            "engine '{}' does not support adopting browser contexts",
            self.name()
        )))
    }

    // ── 网络请求拦截处理（配合 intercept_requests）──────────────
    /// 当前被 Fetch 拦截、等待处理的请求列表（JSON 摘要）。
    fn pending_requests(&self, tab: TabId) -> Vec<Value> {
        let _ = tab;
        Vec::new()
    }
    /// 用自定义响应放行被拦截的请求。
    fn fulfill_request(
        &self,
        tab: TabId,
        request_id: &str,
        status: u16,
        body: Option<Vec<u8>>,
        headers: Option<Value>,
    ) -> Result<()> {
        let _ = (request_id, status, body, headers);
        Err(crate::engine::EngineError::unsupported(format!(
            "engine '{}' does not support request interception fulfillment; tab {tab}",
            self.name()
        )))
    }
    /// 原样放行被拦截的请求。
    fn continue_request(&self, tab: TabId, request_id: &str) -> Result<()> {
        let _ = request_id;
        Err(crate::engine::EngineError::unsupported(format!(
            "engine '{}' does not support request interception; tab {tab}",
            self.name()
        )))
    }
    /// 放行被拦截的请求但**修改响应**（替换状态码/响应体/响应头），
    /// 对应 CDP `Fetch.continueRequest` 的 response 覆盖字段。
    fn modify_response(
        &self,
        tab: TabId,
        request_id: &str,
        status: u16,
        body: Option<Vec<u8>>,
        headers: Option<Value>,
    ) -> Result<()> {
        let _ = (request_id, status, body, headers);
        Err(crate::engine::EngineError::unsupported(format!(
            "engine '{}' does not support request interception; tab {tab}",
            self.name()
        )))
    }
    /// 中止被拦截的请求。
    fn abort_request(&self, tab: TabId, request_id: &str) -> Result<()> {
        let _ = request_id;
        Err(crate::engine::EngineError::unsupported(format!(
            "engine '{}' does not support request interception; tab {tab}",
            self.name()
        )))
    }
}

/// 工具函数：将引擎的 `Unsupported` 错误转成人类可读提示。
pub fn unsupported_msg(engine: &str, what: &str) -> EngineError {
    EngineError::unsupported(format!("engine '{engine}' does not support {what}"))
}
