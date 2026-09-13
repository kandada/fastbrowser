// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! C ABI 出口（对应 `fastbrowser_c/include/fastbrowser.h`）。
//!
//! 风格对齐 fastshell：
//! - 全局单例 `OnceLock<Mutex<Fastbrowser>>`
//! - 进出全为 JSON 字符串，`fastbrowser_free_string` 释放
//! - 流式回调（页面事件 / 离屏帧）经 C 函数指针推给宿主
//! - 移动端通过 `fastbrowser_register_webview_ops` 注入宿主 WebView 实现

use std::ffi::{c_char, CStr, CString};
use std::os::raw::{c_int, c_uchar};
use std::sync::{Arc, Mutex, OnceLock};

use serde_json::{json, Value};

use crate::engine::host::{PageEventSink, ViewFrameSink, WebViewOps};
use crate::engine::{EngineError, Image, InputEvent, Result, TabId, ViewFrame, Viewport};
use crate::sdk::Fastbrowser;

// ── 全局单例 ───────────────────────────────────────────────────

static SDK: OnceLock<Mutex<Fastbrowser>> = OnceLock::new();

fn sdk() -> &'static Mutex<Fastbrowser> {
    SDK.get_or_init(|| Mutex::new(Fastbrowser::new()))
}

// ── 字符串 / JSON 工具 ─────────────────────────────────────────

unsafe fn read_cstr(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    CStr::from_ptr(ptr).to_str().ok().map(|s| s.to_string())
}

fn to_c(value: &Value) -> *mut c_char {
    CString::new(value.to_string())
        .expect("json contains no interior nul")
        .into_raw()
}

/// 执行一个返回 JSON 的调用；错误统一转 `{"error":{"kind","message"}}`。
fn guard<T>(f: impl FnOnce(&mut Fastbrowser) -> Result<T>) -> *mut c_char
where
    T: serde::Serialize,
{
    let mut sdk = sdk().lock().unwrap_or_else(|e| e.into_inner());
    match f(&mut sdk) {
        Ok(v) => match serde_json::to_value(v) {
            Ok(value) => to_c(&value),
            Err(e) => to_c(&EngineError::from_serde(e).to_json()),
        },
        Err(e) => to_c(&e.to_json()),
    }
}

fn ok_json() -> Value {
    json!({ "ok": true })
}

// ── 生命周期 ───────────────────────────────────────────────────

/// 内核版本号（静态字符串）。
#[no_mangle]
pub extern "C" fn fastbrowser_version() -> *mut c_char {
    CString::new(crate::VERSION).unwrap().into_raw()
}

/// 释放由 fastbrowser_* 返回的字符串。
///
/// # Safety
/// `ptr` 必须是由 fastbrowser_* 返回且尚未释放的指针；空指针安全。
#[no_mangle]
pub unsafe extern "C" fn fastbrowser_free_string(ptr: *mut c_char) {
    if !ptr.is_null() {
        drop(CString::from_raw(ptr));
    }
}

// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.
/// 初始化内核。`config_json` 为 Config 的 JSON。
///
/// # Safety
/// `config_json` 必须是有效的 NUL 结尾 UTF-8 字符串指针。
#[no_mangle]
pub unsafe extern "C" fn fastbrowser_init(config_json: *const c_char) -> *mut c_char {
    let cfg_str = match read_cstr(config_json) {
        Some(s) => s,
        None => return to_c(&EngineError::invalid("config_json is null").to_json()),
    };
    let config: crate::config::Config = match serde_json::from_str(&cfg_str) {
        Ok(c) => c,
        Err(e) => return to_c(&EngineError::from_serde(e).to_json()),
    };
    guard(|sdk| {
        sdk.init(config)?;
        Ok(json!({"ok": true}))
    })
}

/// 关闭内核。
#[no_mangle]
pub extern "C" fn fastbrowser_shutdown() -> *mut c_char {
    guard(|sdk| {
        sdk.shutdown();
        Ok(json!({"ok": true}))
    })
}

// ── 打开 / 导航 ────────────────────────────────────────────────

/// 打开 URL 并激活（自动建标签页）。
///
/// # Safety
/// `url` 必须是有效的 NUL 结尾 UTF-8 字符串指针。
#[no_mangle]
pub unsafe extern "C" fn fastbrowser_open(url: *const c_char) -> *mut c_char {
    let url = match read_cstr(url) {
        Some(u) => u,
        None => return to_c(&EngineError::invalid("url is null").to_json()),
    };
    guard(|sdk| sdk.open(&url))
}

/// 导航（需已有标签页；无则自动创建）。
///
/// # Safety
/// `url` 必须是有效的 NUL 结尾 UTF-8 字符串指针。
#[no_mangle]
pub unsafe extern "C" fn fastbrowser_navigate(url: *const c_char) -> *mut c_char {
    let url = match read_cstr(url) {
        Some(u) => u,
        None => return to_c(&EngineError::invalid("url is null").to_json()),
    };
    guard(|sdk| sdk.navigate(&url))
}

// ── 工具 ───────────────────────────────────────────────────────

#[no_mangle]
pub extern "C" fn fastbrowser_tool_list() -> *mut c_char {
    guard(|sdk| Ok(sdk.tool_list()))
}

/// 调用工具。`name` 与 `params_json` 均为 JSON 字符串。
///
/// # Safety
/// `name` / `params_json` 必须是有效的 NUL 结尾 UTF-8 字符串指针。
#[no_mangle]
pub unsafe extern "C" fn fastbrowser_tool_call(
    name: *const c_char,
    params_json: *const c_char,
) -> *mut c_char {
    let name = match read_cstr(name) {
        Some(n) => n,
        None => return to_c(&EngineError::invalid("name is null").to_json()),
    };
    let params: Value = match read_cstr(params_json) {
        Some(s) => serde_json::from_str(&s).unwrap_or(Value::Object(serde_json::Map::new())),
        None => Value::Object(serde_json::Map::new()),
    };
    guard(|sdk| sdk.tool_call(&name, params))
}

// ── 快照 / 截图 / 视图 ─────────────────────────────────────────

#[no_mangle]
pub extern "C" fn fastbrowser_snapshot() -> *mut c_char {
    guard(|sdk| sdk.snapshot())
}

#[no_mangle]
pub extern "C" fn fastbrowser_screenshot() -> *mut c_char {
    guard(|sdk| {
        let img = sdk.screenshot()?;
        Ok(json!({
            "width": img.width,
            "height": img.height,
            "format": "rgba",
            "base64": img.to_base64(),
        }))
    })
}

#[no_mangle]
pub extern "C" fn fastbrowser_get_view() -> *mut c_char {
    guard(|sdk| {
        let view = sdk.get_view();
        Ok(json!({"view": view}))
    })
}

#[no_mangle]
pub extern "C" fn fastbrowser_set_viewport(width: u32, height: u32) -> *mut c_char {
    guard(|sdk| {
        sdk.set_viewport(width, height)?;
        Ok(ok_json())
    })
}

/// 启动向 `fastbrowser_register_viewframe_callback` 的连续帧推送。
/// `tab` 为目标标签页；`opts_json` 为 `{fps, max_width, max_height}`（可省略）。
///
/// # Safety
/// `opts_json` 必须是有效的 NUL 结尾 UTF-8 字符串指针。
#[no_mangle]
pub unsafe extern "C" fn fastbrowser_start_frame_stream(
    tab: u32,
    opts_json: *const c_char,
) -> *mut c_char {
    let opts = match read_cstr(opts_json) {
        Some(s) => {
            serde_json::from_str::<crate::engine::FrameStreamOptions>(&s).unwrap_or_default()
        }
        None => crate::engine::FrameStreamOptions::default(),
    };
    guard(|sdk| sdk.start_frame_stream(crate::engine::TabId(tab), opts))
}

/// 停止帧推送。
#[no_mangle]
pub extern "C" fn fastbrowser_stop_frame_stream(tab: u32) -> *mut c_char {
    guard(|sdk| sdk.stop_frame_stream(crate::engine::TabId(tab)))
}

// ── 信息 ───────────────────────────────────────────────────────

#[no_mangle]
pub extern "C" fn fastbrowser_get_info() -> *mut c_char {
    guard(|sdk| Ok(sdk.get_info().to_json()))
}

#[no_mangle]
pub extern "C" fn fastbrowser_status() -> *mut c_char {
    guard(|sdk| Ok(sdk.status()))
}

/// 动作审计日志（JSON 数组，新→旧）。
#[no_mangle]
pub extern "C" fn fastbrowser_audit() -> *mut c_char {
    guard(|sdk| Ok(sdk.audit()))
}

/// 清空动作审计日志。
#[no_mangle]
pub extern "C" fn fastbrowser_clear_audit() -> *mut c_char {
    guard(|sdk| {
        sdk.clear_audit();
        Ok(ok_json())
    })
}

// ── 权限（对齐 fastshell）──────────────────────────────────────

/// 授予/拒绝权限（resource 形如 "network:<host>"、"camera"…）。
///
/// # Safety
/// `resource` 必须是有效的 NUL 结尾 UTF-8 字符串指针。
#[no_mangle]
pub unsafe extern "C" fn fastbrowser_set_permission(resource: *const c_char, allowed: c_uchar) {
    if let Some(r) = read_cstr(resource) {
        crate::sdk::device_callback::set_net_permission(&r, allowed != 0);
    }
}

// ── 流式回调（页面事件 / 离屏帧）──────────────────────────────

type CEventCallback = Option<extern "C" fn(tab: u32, event_json: *const c_char)>;
type CFrameCallback = Option<extern "C" fn(tab: u32, frame_json: *const c_char)>;

struct CEventSink {
    cb: CEventCallback,
}
impl PageEventSink for CEventSink {
    fn on_page_event(&self, tab: TabId, event: &crate::engine::PageEvent) {
        if let Some(cb) = self.cb {
            let s = CString::new(event.to_json().to_string()).unwrap();
            cb(tab.as_u32(), s.as_ptr());
        }
    }
}

struct CFrameSink {
    cb: CFrameCallback,
}
impl ViewFrameSink for CFrameSink {
    fn on_view_frame(&self, tab: TabId, frame: &ViewFrame) {
        if let Some(cb) = self.cb {
            let v = json!({"width": frame.width, "height": frame.height, "seq": frame.seq, "base64": base64_rgba(&frame.rgba)});
            let s = CString::new(v.to_string()).unwrap();
            cb(tab.as_u32(), s.as_ptr());
        }
    }
}

fn base64_rgba(rgba: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(rgba)
}

/// 注册页面事件回调（建议在 init 之前调用）。
#[no_mangle]
pub extern "C" fn fastbrowser_register_event_callback(cb: CEventCallback) -> c_int {
    sdk()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .register_event_sink(Arc::new(CEventSink { cb }));
    0
}

/// 注册离屏帧回调（建议在 init 之前调用）。
#[no_mangle]
pub extern "C" fn fastbrowser_register_viewframe_callback(cb: CFrameCallback) -> c_int {
    sdk()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .register_frame_sink(Arc::new(CFrameSink { cb }));
    0
}

// ── 移动端 WebViewOps 注入（C ABI 版）──────────────────────────

/// C 侧 WebView 操作函数表（对应 host::WebViewOps）。
#[repr(C)]
pub struct FbWebViewOps {
    pub create: Option<extern "C" fn(url: *const c_char, width: u32, height: u32) -> i64>,
    pub destroy: Option<extern "C" fn(handle: i64) -> c_int>,
    pub evaluate: Option<extern "C" fn(handle: i64, script: *const c_char) -> *mut c_char>,
    pub navigate: Option<extern "C" fn(handle: i64, url: *const c_char) -> c_int>,
    pub screenshot: Option<extern "C" fn(handle: i64, out_w: *mut u32, out_h: *mut u32) -> *mut u8>,
    pub screenshot_free: Option<extern "C" fn(ptr: *mut u8)>,
    pub set_viewport: Option<extern "C" fn(handle: i64, width: u32, height: u32) -> c_int>,
    pub native_view: Option<extern "C" fn(handle: i64) -> i64>,
    pub dispatch_event: Option<extern "C" fn(handle: i64, event_json: *const c_char) -> c_int>,
    pub go_back: Option<extern "C" fn(handle: i64) -> c_int>,
    pub go_forward: Option<extern "C" fn(handle: i64) -> c_int>,
    pub free_string: Option<extern "C" fn(ptr: *mut c_char)>,
}

struct CbWebViewOps {
    ops: FbWebViewOps,
}

unsafe impl Send for CbWebViewOps {}
unsafe impl Sync for CbWebViewOps {}

fn err_unsupported(what: &str) -> EngineError {
    EngineError::unsupported(format!("C WebViewOps.{what} is null"))
}

impl WebViewOps for CbWebViewOps {
    fn create_webview(&self, url: &str, viewport: Viewport) -> Result<u64> {
        let cb = self.ops.create.ok_or_else(|| err_unsupported("create"))?;
        let c = CString::new(url).map_err(|_| EngineError::invalid("url has interior nul"))?;
        let h = cb(c.as_ptr(), viewport.width, viewport.height);
        if h < 0 {
            Err(EngineError::new(
                crate::engine::ErrorKind::Plugin,
                "create_webview failed",
            ))
        } else {
            Ok(h as u64)
        }
    }

    fn destroy_webview(&self, handle: u64) -> Result<()> {
        let cb = self.ops.destroy.ok_or_else(|| err_unsupported("destroy"))?;
        cb(handle as i64);
        Ok(())
    }

    fn evaluate_js(&self, handle: u64, script: &str) -> Result<Value> {
        let cb = self
            .ops
            .evaluate
            .ok_or_else(|| err_unsupported("evaluate"))?;
        let c =
            CString::new(script).map_err(|_| EngineError::invalid("script has interior nul"))?;
        let out = cb(handle as i64, c.as_ptr());
        if out.is_null() {
            return Err(EngineError::new(
                crate::engine::ErrorKind::Plugin,
                "evaluate_js returned null",
            ));
        }
        let json_str =
            unsafe { CStr::from_ptr(out).to_str().map(|s| s.to_string()) }.map_err(|_| {
                EngineError::new(crate::engine::ErrorKind::Plugin, "evaluate_js bad utf8")
            })?;
        if let Some(f) = self.ops.free_string {
            f(out);
        }
        serde_json::from_str(&json_str).map_err(|_| {
            EngineError::new(
                crate::engine::ErrorKind::Plugin,
                format!("evaluate_js bad json: {json_str}"),
            )
        })
    }

    fn navigate_webview(&self, handle: u64, url: &str) -> Result<()> {
        let cb = self
            .ops
            .navigate
            .ok_or_else(|| err_unsupported("navigate"))?;
        let c = CString::new(url).map_err(|_| EngineError::invalid("url has interior nul"))?;
        cb(handle as i64, c.as_ptr());
        Ok(())
    }

    fn screenshot_webview(&self, handle: u64) -> Result<Image> {
        let cb = self
            .ops
            .screenshot
            .ok_or_else(|| err_unsupported("screenshot"))?;
        let free = self
            .ops
            .screenshot_free
            .ok_or_else(|| err_unsupported("screenshot_free"))?;
        let mut w = 0u32;
        let mut h = 0u32;
        let ptr = cb(handle as i64, &mut w, &mut h);
        if ptr.is_null() {
            return Err(EngineError::new(
                crate::engine::ErrorKind::Plugin,
                "screenshot returned null",
            ));
        }
        let len = (w as usize) * (h as usize) * 4;
        let buf = unsafe { std::slice::from_raw_parts(ptr, len) }.to_vec();
        free(ptr);
        Ok(Image::new(w, h, buf))
    }

    fn set_viewport_webview(&self, handle: u64, viewport: Viewport) -> Result<()> {
        let cb = self
            .ops
            .set_viewport
            .ok_or_else(|| err_unsupported("set_viewport"))?;
        cb(handle as i64, viewport.width, viewport.height);
        Ok(())
    }

    fn native_view(&self, handle: u64) -> Result<crate::engine::ViewHandle> {
        let cb = self
            .ops
            .native_view
            .ok_or_else(|| err_unsupported("native_view"))?;
        let v = cb(handle as i64);
        if v < 0 {
            Err(EngineError::new(
                crate::engine::ErrorKind::Plugin,
                "native_view failed",
            ))
        } else {
            Ok(crate::engine::ViewHandle::Native(v as u64))
        }
    }

    fn dispatch_event(&self, handle: u64, event: &InputEvent) -> Result<()> {
        let cb = self
            .ops
            .dispatch_event
            .ok_or_else(|| err_unsupported("dispatch_event"))?;
        let json_str = serde_json::to_string(event).map_err(|_| {
            EngineError::new(crate::engine::ErrorKind::Plugin, "dispatch_event serialize")
        })?;
        let c = CString::new(json_str).map_err(|_| EngineError::invalid("event json nul"))?;
        cb(handle as i64, c.as_ptr());
        Ok(())
    }

    fn go_back(&self, handle: u64) -> Result<()> {
        let cb = self.ops.go_back.ok_or_else(|| err_unsupported("go_back"))?;
        cb(handle as i64);
        Ok(())
    }

    fn go_forward(&self, handle: u64) -> Result<()> {
        let cb = self
            .ops
            .go_forward
            .ok_or_else(|| err_unsupported("go_forward"))?;
        cb(handle as i64);
        Ok(())
    }
}

/// 注册 C 侧 WebViewOps（init 之前调用）。返回 0 表示成功。
///
/// # Safety
/// `ops` 必须是有效且生命周期覆盖内核生命周期的 `FbWebViewOps` 指针。
#[no_mangle]
pub unsafe extern "C" fn fastbrowser_register_webview_ops(ops: *const FbWebViewOps) -> c_int {
    if ops.is_null() {
        return -1;
    }
    let ops = &*ops;
    let adapter = Arc::new(CbWebViewOps {
        ops: FbWebViewOps { ..*ops },
    });
    match crate::sdk::plugin::register_webview_ops(adapter) {
        Ok(()) => 0,
        Err(_) => -2,
    }
}

// ── 测试（直接调用 extern 函数）────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// 全局单例测试需串行，避免并行竞态。
    static FFI_LOCK: Mutex<()> = Mutex::new(());

    fn cstr(s: &str) -> CString {
        CString::new(s).unwrap()
    }

    fn take(ptr: *mut c_char) -> String {
        unsafe {
            let s = CStr::from_ptr(ptr).to_str().unwrap().to_string();
            fastbrowser_free_string(ptr);
            s
        }
    }

    fn free(ptr: *mut c_char) {
        unsafe { fastbrowser_free_string(ptr) }
    }

    #[test]
    fn version_string() {
        let v = take(fastbrowser_version());
        assert_eq!(v, crate::VERSION);
    }

    #[test]
    fn init_and_call() {
        let _g = FFI_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        unsafe {
            free(fastbrowser_init(cstr("{\"engine\":\"mock\"}").as_ptr()));
        }
        let out = take(unsafe { fastbrowser_open(cstr("https://example.com").as_ptr()) });
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["title"], "Example Page");

        let list = take(fastbrowser_tool_list());
        let lv: Value = serde_json::from_str(&list).unwrap();
        assert!(lv.as_array().unwrap().len() >= 30);

        let snap = take(fastbrowser_snapshot());
        let sv: Value = serde_json::from_str(&snap).unwrap();
        assert_eq!(sv["title"], "Example Page");

        let shot = take(fastbrowser_screenshot());
        let im: Value = serde_json::from_str(&shot).unwrap();
        assert_eq!(im["format"], "rgba");

        let call = take(unsafe {
            fastbrowser_tool_call(cstr("get_current_url").as_ptr(), cstr("{}").as_ptr())
        });
        let cv: Value = serde_json::from_str(&call).unwrap();
        assert_eq!(cv["url"], "https://example.com");

        let info = take(fastbrowser_get_info());
        let iv: Value = serde_json::from_str(&info).unwrap();
        assert_eq!(iv["engine"], "mock");

        free(fastbrowser_shutdown());
    }

    #[test]
    fn error_returns_json_error() {
        let _g = FFI_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        unsafe {
            free(fastbrowser_init(cstr("{\"engine\":\"mock\"}").as_ptr()));
        }
        // 未打开标签页时 tool_call 也会因无活动标签页报错，或自动创建；这里测未知工具
        let call =
            take(unsafe { fastbrowser_tool_call(cstr("nope").as_ptr(), cstr("{}").as_ptr()) });
        let v: Value = serde_json::from_str(&call).unwrap();
        assert!(v.get("error").is_some());
        free(fastbrowser_shutdown());
    }

    #[test]
    fn bad_init_errors() {
        let _g = FFI_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        unsafe {
            free(fastbrowser_init(cstr("not json").as_ptr()));
        }
    }
}
