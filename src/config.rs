// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! 内核配置（对齐 fastshell 的 Config 语义，但聚焦浏览器域）。

use serde::{Deserialize, Serialize};

use crate::engine::{RenderingMode, Viewport};

/// 内核配置。由宿主（App / CLI / 语言绑定）在 `init` 时传入。
/// 字段均可省略（`#[serde(default)]`），部分 JSON 即可覆盖默认配置。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// 引擎类型："mock" | "cef" | "webview"。默认按编译 feature 决定。
    pub engine: String,
    /// 渲染模式：托管（窗口内嵌 / 宿主嵌入视图）或 无头（离屏）。
    pub rendering_mode: RenderingMode,
    /// 配置文件（Profile）名称，用于多账号隔离。
    pub profile_name: String,
    /// 是否无痕（incognito）：关闭时不落盘 cookie/storage。
    pub incognito: bool,
    /// 自定义 User-Agent。
    pub user_agent: Option<String>,
    /// 初始视口（宽/高/缩放）。
    pub viewport: Option<Viewport>,
    /// HTTP 代理地址，例如 "http://127.0.0.1:7890"。
    pub proxy: Option<String>,
    /// 磁盘缓存目录（空则内存缓存）。
    pub cache_path: Option<String>,
    /// 会话状态（cookie/storage）落盘目录（incognito 时忽略）。
    pub storage_path: Option<String>,
    /// Chromium/CDP 引擎的连接端点（如 ws://127.0.0.1:9222/devtools/page/xxx）。
    /// 引擎为 "chromium" 时必填；"cef" 引擎可留空（自动发现）。
    pub cdp_url: Option<String>,
    /// 精确附加到指定 CDP target（`targetId`）。设置后 `initialize` 不再取
    /// "第一个 page target"，而是 attach 该 target——用于宿主把某个可见的
    /// 浏览器视图（如 Electron `WebContentsView`）交给内核驱动。
    pub cdp_target_id: Option<String>,
    /// 按 URL 子串匹配要附加的 page target（`targetId` 未知时的兜底）。
    pub cdp_target_url_contains: Option<String>,
    /// 宿主指定要复用的隔离浏览器上下文原生 id（CDP `browserContextId`）。
    /// 设置后新建标签页在该上下文中进行，实现跨进程/跨调用复用同一隔离上下文，
    /// 避免无状态调用每次新建上下文。默认 `None`（由 `isolated_profiles` 决定）。
    pub browser_context_id: Option<String>,
    /// 浏览器 locale（如 "zh-CN"），对齐 browser-use BrowserConfig。
    pub locale: Option<String>,
    /// 是否自动接受下载。
    pub accept_downloads: bool,
    /// 默认下载目录。
    pub default_download_path: Option<String>,
    /// 传递给底层浏览器的额外启动参数（Chromium/CEF）。
    pub extra_browser_args: Vec<String>,
    /// 工具调用超时（毫秒），0 表示不限。
    pub command_timeout_ms: u64,
    /// 元素操作（点击/输入）的 Actionability 自动等待上限（毫秒）：
    /// 在超时内等待元素「出现→可见→不被遮挡→位置稳定」后再动作，
    /// 对齐 Playwright 语义。默认 5000。
    pub actionability_timeout_ms: u64,
    /// 移动端网络访问是否先询问宿主权限。
    pub network_ask_permission: bool,
    /// 是否自动接受不必要的弹窗/下载（保持内核行为可预期）。
    pub auto_accept_dialogs: bool,
    /// 多账号隔离：为每个 Profile 创建独立浏览器上下文（CDP BrowserContext）。
    /// 默认关闭——某些无头浏览器（如 Chrome for Testing）不支持
    /// `Target.createBrowserContext`，开启后在不受支持的引擎上会自动降级。
    pub isolated_profiles: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            engine: "mock".to_string(),
            rendering_mode: RenderingMode::Headless,
            profile_name: "default".to_string(),
            incognito: false,
            user_agent: None,
            viewport: None,
            proxy: None,
            cache_path: None,
            storage_path: None,
            cdp_url: None,
            cdp_target_id: None,
            cdp_target_url_contains: None,
            browser_context_id: None,
            locale: None,
            accept_downloads: false,
            default_download_path: None,
            extra_browser_args: Vec::new(),
            command_timeout_ms: 0,
            actionability_timeout_ms: 5000,
            network_ask_permission: false,
            auto_accept_dialogs: true,
            isolated_profiles: false,
        }
    }
}

impl Config {
    /// 快速构造：仅指定引擎。
    pub fn for_engine(engine: impl Into<String>) -> Self {
        Config {
            engine: engine.into(),
            ..Config::default()
        }
    }

    /// 快速构造：托管（有头）模式。
    pub fn hosted(mut self) -> Self {
        self.rendering_mode = RenderingMode::Hosted;
        self
    }
}
