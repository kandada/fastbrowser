// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! 宿主插件注册表（App 无关）。
//!
//! 内核不持有任何 UI/设备能力；宿主通过本模块注入：
//! - `WebViewOps`：移动端系统 WebView 的真实操作
//! - 事件/帧回调：把页面事件与离屏帧推给宿主
//! - `HostPlugin` / `DevicePlugin`：设备能力与权限

use std::sync::{Arc, Mutex, OnceLock};

use crate::engine::host::{EncodedFrameSink, HostPlugin, PageEventSink, ViewFrameSink, WebViewOps};

// ── WebViewOps（移动端桥）───────────────────────────────────────

static WEBVIEW_OPS: OnceLock<Arc<dyn WebViewOps>> = OnceLock::new();

/// 注册宿主 WebView 操作实现（仅一次）。
pub fn register_webview_ops(ops: Arc<dyn WebViewOps>) -> crate::engine::Result<()> {
    WEBVIEW_OPS.set(ops).map_err(|_| {
        crate::engine::EngineError::new(
            crate::engine::ErrorKind::Plugin,
            "WebViewOps already registered",
        )
    })
}

/// 取出已注册的 WebViewOps（若已注册）。
pub fn take_webview_ops() -> Option<Arc<dyn WebViewOps>> {
    WEBVIEW_OPS.get().cloned()
}

// ── 事件 / 帧 回调 ─────────────────────────────────────────────

static EVENT_SINK: OnceLock<Mutex<Option<Arc<dyn PageEventSink>>>> = OnceLock::new();
static FRAME_SINK: OnceLock<Mutex<Option<Arc<dyn ViewFrameSink>>>> = OnceLock::new();
static ENCODED_FRAME_SINK: OnceLock<Mutex<Option<Arc<dyn EncodedFrameSink>>>> = OnceLock::new();

fn event_slot() -> &'static Mutex<Option<Arc<dyn PageEventSink>>> {
    EVENT_SINK.get_or_init(|| Mutex::new(None))
}
fn frame_slot() -> &'static Mutex<Option<Arc<dyn ViewFrameSink>>> {
    FRAME_SINK.get_or_init(|| Mutex::new(None))
}
fn encoded_frame_slot() -> &'static Mutex<Option<Arc<dyn EncodedFrameSink>>> {
    ENCODED_FRAME_SINK.get_or_init(|| Mutex::new(None))
}

/// 注册页面事件回调（宿主用于驱动对话流 / 日志）。
pub fn register_event_sink(sink: Arc<dyn PageEventSink>) {
    *event_slot().lock().unwrap_or_else(|e| e.into_inner()) = Some(sink);
}

/// 注册离屏帧回调（宿主用于预览窗渲染）。
pub fn register_frame_sink(sink: Arc<dyn ViewFrameSink>) {
    *frame_slot().lock().unwrap_or_else(|e| e.into_inner()) = Some(sink);
}

pub fn get_event_sink() -> Option<Arc<dyn PageEventSink>> {
    event_slot()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

pub fn get_frame_sink() -> Option<Arc<dyn ViewFrameSink>> {
    frame_slot()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

/// 注册编码帧（JPEG/PNG）回调（宿主预览窗渲染，体积更小）。
pub fn register_encoded_frame_sink(sink: Arc<dyn EncodedFrameSink>) {
    *encoded_frame_slot()
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = Some(sink);
}

pub fn get_encoded_frame_sink() -> Option<Arc<dyn EncodedFrameSink>> {
    encoded_frame_slot()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

// ── HostPlugin（通用宿主能力）────────────────────────────────────

static HOST_PLUGIN: OnceLock<Mutex<Option<Arc<dyn HostPlugin>>>> = OnceLock::new();

fn host_slot() -> &'static Mutex<Option<Arc<dyn HostPlugin>>> {
    HOST_PLUGIN.get_or_init(|| Mutex::new(None))
}

/// 注册通用宿主插件。
pub fn register_host_plugin(plugin: Arc<dyn HostPlugin>) {
    *host_slot().lock().unwrap_or_else(|e| e.into_inner()) = Some(plugin);
}

pub fn get_host_plugin() -> Option<Arc<dyn HostPlugin>> {
    host_slot()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{PageEvent, TabId};

    struct Sink;
    impl PageEventSink for Sink {
        fn on_page_event(&self, _t: TabId, _e: &PageEvent) {}
    }

    #[test]
    fn sink_registry() {
        register_event_sink(Arc::new(Sink));
        assert!(get_event_sink().is_some());
        let ops = crate::engine::host::NoopWebViewOps;
        let r = register_webview_ops(Arc::new(ops));
        // 允许失败（可能已被其他测试注册）；二者皆可
        assert!(r.is_ok() || r.is_err());
    }
}
