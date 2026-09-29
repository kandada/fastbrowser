// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! 宿主回调（插件）trait —— 保持内核 App 无关的关键缝隙。
//!
//! - `WebViewOps`：移动端系统 WebView 的真实操作由宿主（Kotlin/Swift/ArkTS）实现，
//!   通过 `sdk::plugin::register_webview_ops` 注入；Rust 侧只定义协议 + 注入 JS。
//! - `PageEventSink` / `ViewFrameSink`：把页面事件与离屏帧推送给宿主应用。

use std::sync::Arc;

use serde_json::Value;

use crate::engine::{
    EncodedViewFrame, EngineError, Image, InputEvent, PageEvent, Result, TabId, ViewFrame,
    ViewHandle, Viewport,
};

/// 宿主 WebView 操作（移动端桥）。
pub trait WebViewOps: Send + Sync {
    /// 创建原生 WebView，返回宿主句柄。
    fn create_webview(&self, url: &str, viewport: Viewport) -> Result<u64>;
    /// 销毁原生 WebView。
    fn destroy_webview(&self, handle: u64) -> Result<()>;
    /// 同步执行 JS，返回 JSON 值。
    fn evaluate_js(&self, handle: u64, script: &str) -> Result<Value>;
    /// 导航。
    fn navigate_webview(&self, handle: u64, url: &str) -> Result<()>;
    /// 截图。
    fn screenshot_webview(&self, handle: u64) -> Result<Image>;
    /// 设置视口。
    fn set_viewport_webview(&self, handle: u64, viewport: Viewport) -> Result<()>;
    /// 返回可嵌入宿主界面的原生视图句柄。
    fn native_view(&self, handle: u64) -> Result<ViewHandle>;
    /// 向 WebView 注入触摸/键盘事件（移动端通常改由 JS 派发）。
    fn dispatch_event(&self, handle: u64, event: &InputEvent) -> Result<()>;
    /// 前进/后退。
    fn go_back(&self, handle: u64) -> Result<()>;
    fn go_forward(&self, handle: u64) -> Result<()>;
}

impl dyn WebViewOps {
    pub fn noop() -> Arc<dyn WebViewOps> {
        Arc::new(NoopWebViewOps)
    }
}

/// 占位实现：未注入宿主 ops 时的兜底。
pub struct NoopWebViewOps;

impl WebViewOps for NoopWebViewOps {
    fn create_webview(&self, _url: &str, _viewport: Viewport) -> Result<u64> {
        Err(EngineError::unsupported("no WebViewOps plugin registered"))
    }
    fn destroy_webview(&self, _handle: u64) -> Result<()> {
        Err(EngineError::unsupported("no WebViewOps plugin registered"))
    }
    fn evaluate_js(&self, _handle: u64, _script: &str) -> Result<Value> {
        Err(EngineError::unsupported("no WebViewOps plugin registered"))
    }
    fn navigate_webview(&self, _handle: u64, _url: &str) -> Result<()> {
        Err(EngineError::unsupported("no WebViewOps plugin registered"))
    }
    fn screenshot_webview(&self, _handle: u64) -> Result<Image> {
        Err(EngineError::unsupported("no WebViewOps plugin registered"))
    }
    fn set_viewport_webview(&self, _handle: u64, _viewport: Viewport) -> Result<()> {
        Err(EngineError::unsupported("no WebViewOps plugin registered"))
    }
    fn native_view(&self, _handle: u64) -> Result<ViewHandle> {
        Err(EngineError::unsupported("no WebViewOps plugin registered"))
    }
    fn dispatch_event(&self, _handle: u64, _event: &InputEvent) -> Result<()> {
        Err(EngineError::unsupported("no WebViewOps plugin registered"))
    }
    fn go_back(&self, _handle: u64) -> Result<()> {
        Err(EngineError::unsupported("no WebViewOps plugin registered"))
    }
    fn go_forward(&self, _handle: u64) -> Result<()> {
        Err(EngineError::unsupported("no WebViewOps plugin registered"))
    }
}

/// 宿主原生无障碍 / 桌面表面操作（移动端桥）。
///
/// 与 [`WebViewOps`] 同构：Android 由宿主（Kotlin `AccessibilityService`，经 JNI）
/// 或其它平台提供，通过 `sdk::plugin::register_surface_ops` 注入；Rust 侧只定义协议。
///
/// 跨边界一律用 JSON 字符串，因为树/动作等内核类型均 `Serialize`/`Deserialize`，
/// 这样 C ABI / JNI 只需最少的函数指针，减少平台耦合。
pub trait SurfaceOps: Send + Sync {
    /// 能力位（`SurfaceCapabilities` 的 JSON）。
    fn capabilities(&self) -> Result<String>;
    /// 枚举可用表面（`Vec<SurfaceInfo>` 的 JSON）。
    fn list(&self) -> Result<String>;
    /// 生成快照（`SurfaceSnapshot` 的 JSON）；`opts_json` 为 `SnapshotOptions` 的 JSON。
    fn snapshot(&self, target: &str, opts_json: &str) -> Result<String>;
    /// 对元素 ref 施动作（`SurfaceAction` 的 JSON）。返回可选信息 JSON（可为空串）。
    fn act(&self, ref_: &str, action_json: &str) -> Result<String>;
    /// 坐标级输入（`InputEvent` 的 JSON）。默认不支持。
    fn input(&self, target: &str, event_json: &str) -> Result<()> {
        let _ = (target, event_json);
        Err(EngineError::unsupported(
            "SurfaceOps host did not implement input",
        ))
    }
    /// 截图（返回 `{"width":..,"height":..,"base64":".."}` 的 JSON）。默认不支持。
    fn screenshot(&self, target: &str) -> Result<String> {
        let _ = target;
        Err(EngineError::unsupported(
            "SurfaceOps host did not implement screenshot",
        ))
    }
    /// 拉取自上次调用以来的事件（`Vec<SurfaceEvent>` 的 JSON）。默认空。
    fn poll_events(&self, target: &str) -> Result<String> {
        let _ = target;
        Ok("[]".to_string())
    }
}

/// 占位实现：未注入宿主 ops 时的兜底。
pub struct NoopSurfaceOps;

impl SurfaceOps for NoopSurfaceOps {
    fn capabilities(&self) -> Result<String> {
        Err(EngineError::unsupported("no SurfaceOps plugin registered"))
    }
    fn list(&self) -> Result<String> {
        Err(EngineError::unsupported("no SurfaceOps plugin registered"))
    }
    fn snapshot(&self, _target: &str, _opts_json: &str) -> Result<String> {
        Err(EngineError::unsupported("no SurfaceOps plugin registered"))
    }
    fn act(&self, _ref_: &str, _action_json: &str) -> Result<String> {
        Err(EngineError::unsupported("no SurfaceOps plugin registered"))
    }
}

/// 页面事件推送回调（供宿主展示思考过程 / 驱动 UI）。
pub trait PageEventSink: Send + Sync {
    fn on_page_event(&self, tab: TabId, event: &PageEvent);
}

/// 离屏帧推送回调（供宿主预览窗渲染）。
pub trait ViewFrameSink: Send + Sync {
    fn on_view_frame(&self, tab: TabId, frame: &ViewFrame);
}

/// 编码帧推送回调（JPEG/PNG 字节直传，供宿主预览窗渲染）。
/// 相比 `ViewFrameSink` 的 RGBA 位图，省去「解码→编码」与体积开销。
pub trait EncodedFrameSink: Send + Sync {
    fn on_encoded_frame(&self, tab: TabId, frame: &EncodedViewFrame);
}

/// 通用的宿主插件 trait（对齐 fastshell DevicePlugin 语义），
/// 供未来接入设备能力（摄像头/定位/剪贴板）与事件回调。
pub trait HostPlugin: Send + Sync {
    /// 查询宿主能力（按需返回 JSON）。
    fn host_capabilities(&self) -> serde_json::Value;
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Sink;
    impl PageEventSink for Sink {
        fn on_page_event(&self, _t: TabId, _e: &PageEvent) {}
    }

    #[test]
    fn noop_ops_errors() {
        let ops = NoopWebViewOps;
        assert!(ops.create_webview("x", Viewport::new(1, 1)).is_err());
    }

    #[test]
    fn sink_is_object_safe() {
        let s: Arc<dyn PageEventSink> = Arc::new(Sink);
        let ev = PageEvent::DomChanged;
        s.on_page_event(TabId(1), &ev);
    }

    #[test]
    fn noop_surface_ops_errors() {
        let ops = NoopSurfaceOps;
        assert!(ops.capabilities().is_err());
        assert!(ops.list().is_err());
        assert!(ops.snapshot("desktop:1", "{}").is_err());
        assert!(ops.act("desktop:1:0", r#"{"kind":"click"}"#).is_err());
        // Defaults are overridable but report unsupported / empty.
        assert!(ops.input("desktop:1", "{}").is_err());
        assert!(ops.screenshot("desktop:1").is_err());
        assert_eq!(ops.poll_events("desktop:1").unwrap(), "[]");
    }
}
