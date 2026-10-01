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
    /// 引擎类型："mock" | "bundled" | "chromium" | "system" | "cef" | "webview"
    /// | "auto"。默认 "mock"。
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
    /// Desktop/pip: explicit system/default browser executable path. Highest
    /// priority for the `system` engine and the `auto` chain; equivalent to the
    /// `CHROME_PATH` environment variable. `None` = auto-discover.
    pub browser_path: Option<String>,
    /// Launch a real browser headless (a window is shown only when
    /// `rendering_mode:"hosted"`). Default `true`.
    pub prefer_headless: bool,
    /// Whether `auto` / `system` may use the platform default browser. Default
    /// `true`.
    pub use_default_browser: bool,
    /// Reuse the default browser's real profile. Default `false` — an isolated
    /// temporary profile is used so the user's session is never touched.
    pub use_user_profile: bool,
    /// Remote debugging port for a launched real browser (`0` = pick a free
    /// port automatically).
    pub remote_debug_port: u16,
    /// When the `auto` chain finds no real browser, fall back to `mock`. Default
    /// `true`, but the fallback is **always reported** (`degraded` + `hint`),
    /// never silent. Set `false` to fail instead.
    pub allow_fallback_mock: bool,
    /// 电脑操作表面层配置（feature `surface` 生效）。
    #[serde(default)]
    pub surface: SurfaceConfig,
}

/// 表面层配置（`Config.surface`）。
///
/// - `enabled=false`：不装配表面层（surface 工具隐藏）。
/// - `provider`：`auto`（浏览器 + 平台原生）/ `browser`（仅浏览器）/ `macos`
///   （仅原生）/ `mock`（脚本化）/ `none`。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SurfaceConfig {
    pub enabled: bool,
    pub provider: String,
    /// 默认快照最大节点数（0 = 用内核默认）。
    pub max_nodes: usize,
    /// 默认快照最大深度（0 = 用内核默认）。
    pub max_depth: usize,
    /// 默认是否只保留有意义节点。
    pub interesting_only: bool,
    /// web 表面是否优先使用引擎原生 AX 树（CDP `Accessibility.getFullAXTree` +
    /// 几何关联）；关闭则回退到注入 JS 的交互元素快照。
    pub prefer_ax_tree: bool,
    /// 单次表面操作超时（毫秒，0 = 内核默认）。
    pub timeout_ms: u64,
}

impl Default for SurfaceConfig {
    fn default() -> Self {
        SurfaceConfig {
            enabled: true,
            provider: "auto".to_string(),
            max_nodes: 0,
            max_depth: 0,
            interesting_only: true,
            prefer_ax_tree: true,
            timeout_ms: 0,
        }
    }
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
            browser_path: None,
            prefer_headless: true,
            use_default_browser: true,
            use_user_profile: false,
            remote_debug_port: 0,
            allow_fallback_mock: true,
            surface: SurfaceConfig::default(),
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

    /// 实际生效的 CDP 命令超时（毫秒）。
    ///
    /// `command_timeout_ms == 0` 表示“不限”，但完全不设上限会让卡死的命令
    /// 永久挂起；因此取一个宽松的默认值（30s，与 Playwright 一致），既避免
    /// 高负载下（如外部磁盘/多测试并行）真实命令被 5s 误杀，又保持有界。
    pub fn effective_command_timeout_ms(&self) -> u64 {
        if self.command_timeout_ms == 0 {
            30_000
        } else {
            self.command_timeout_ms
        }
    }
}
