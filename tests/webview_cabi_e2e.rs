// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! C ABI `fastbrowser_register_webview_ops` 注入路径 + null/坏输入安全性测试。
//!
//! 独立测试二进制（与 Rust 侧 webview 测试分进程），确保各自的全局单例
//! （`WEBVIEW_OPS` / FFI `SDK`）互不干扰。

#![cfg(feature = "engine-webview")]

use std::collections::HashMap;
use std::ffi::{c_char, CStr, CString};
use std::sync::{Mutex, OnceLock};

use serde_json::{json, Value};

/// 所有 FFI 用例共享一把锁（FFI 全局单例，需串行）。
static FFI_LOCK: Mutex<()> = Mutex::new(());

// ── 模拟宿主 C 函数表（按 handle 维护 url/title）──────────────

struct CState {
    url: String,
    title: String,
}

fn c_state() -> &'static Mutex<HashMap<i64, CState>> {
    static ST: OnceLock<Mutex<HashMap<i64, CState>>> = OnceLock::new();
    ST.get_or_init(|| Mutex::new(HashMap::new()))
}

fn snapshot_json(title: &str, url: &str) -> String {
    json!({
        "elements": [
            {"id":"a","tag":"h1","text":title,"rect":{"x":0,"y":0,"width":100,"height":30},"visible":true},
            {"id":"b","tag":"input","text":"","value":"","input_type":"text","name":"username","placeholder":"Username","rect":{"x":0,"y":50,"width":200,"height":30},"visible":true},
            {"id":"c","tag":"button","text":"Submit","rect":{"x":0,"y":90,"width":100,"height":30},"visible":true},
            {"id":"d","tag":"a","text":"Register","href":url,"rect":{"x":0,"y":130,"width":80,"height":30},"visible":true}
        ],
        "meta": {"total":4,"truncated":false,"viewport_h":600,"scroll_h":600,"scroll_y":0},
        "frames": []
    })
    .to_string()
}

use fastbrowser::sdk::ffi::FbWebViewOps;

fn c_read(ptr: *const c_char) -> String {
    if ptr.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(ptr).to_str().unwrap_or("").to_string() }
}

extern "C" fn c_create(url: *const c_char, _w: u32, _h: u32) -> i64 {
    let handle = 100 + c_state().lock().unwrap_or_else(|e| e.into_inner()).len() as i64;
    let url = c_read(url);
    let title = if url.contains("login") {
        "Login"
    } else {
        "Generic"
    }
    .to_string();
    c_state()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(handle, CState { url, title });
    handle
}
extern "C" fn c_destroy(h: i64) -> i32 {
    c_state()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&h);
    0
}
extern "C" fn c_evaluate(h: i64, script: *const c_char) -> *mut c_char {
    let s = c_read(script);
    let st = c_state().lock().unwrap_or_else(|e| e.into_inner());
    let t = st
        .get(&h)
        .map(|t| (t.url.clone(), t.title.clone()))
        .unwrap_or_default();
    let val = if s.contains("document.title") {
        json!(t.1).to_string()
    } else if s.contains("FB_EXTRACT_V3") {
        // 宿主把 JS 结果（JSON.stringify 得到的字符串）按 JSON 编码回传：
        // 即 Value::String(snapshot_json) 的 JSON 文本。
        json!(snapshot_json(&t.1, &t.0)).to_string()
    } else if s.contains("location.href") {
        json!(t.0).to_string()
    } else if s.contains("__fbFind") {
        json!("ok").to_string()
    } else {
        json!("done").to_string()
    };
    CString::new(val).unwrap().into_raw()
}
extern "C" fn c_navigate(h: i64, url: *const c_char) -> i32 {
    let url = c_read(url);
    if let Some(t) = c_state()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_mut(&h)
    {
        t.url = url;
    }
    0
}
extern "C" fn c_screenshot(_h: i64, w: *mut u32, h: *mut u32) -> *mut u8 {
    unsafe {
        *w = 2;
        *h = 2;
    }
    let buf: Vec<u8> = vec![
        0u8, 255, 0, 255, 255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255,
    ];
    let ptr = buf.as_ptr() as *mut u8;
    std::mem::forget(buf); // 测试用：泄漏 16 字节
    ptr
}
extern "C" fn c_screenshot_free(_p: *mut u8) {}
extern "C" fn c_set_viewport(_h: i64, _w: u32, _h2: u32) -> i32 {
    0
}
extern "C" fn c_native_view(h: i64) -> i64 {
    h
}
extern "C" fn c_dispatch(_h: i64, _e: *const c_char) -> i32 {
    0
}
extern "C" fn c_goback(_h: i64) -> i32 {
    0
}
extern "C" fn c_gofwd(_h: i64) -> i32 {
    0
}
extern "C" fn c_free(p: *mut c_char) {
    if !p.is_null() {
        unsafe {
            drop(CString::from_raw(p));
        }
    }
}

#[test]
fn c_abi_webview_ops_registration() {
    let _g = FFI_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let ops = FbWebViewOps {
        create: Some(c_create),
        destroy: Some(c_destroy),
        evaluate: Some(c_evaluate),
        navigate: Some(c_navigate),
        screenshot: Some(c_screenshot),
        screenshot_free: Some(c_screenshot_free),
        set_viewport: Some(c_set_viewport),
        native_view: Some(c_native_view),
        dispatch_event: Some(c_dispatch),
        go_back: Some(c_goback),
        go_forward: Some(c_gofwd),
        free_string: Some(c_free),
    };

    use fastbrowser::sdk::ffi;
    let take = |p: *mut c_char| unsafe {
        let s = CStr::from_ptr(p).to_str().unwrap().to_string();
        ffi::fastbrowser_free_string(p);
        s
    };
    let cstr = |s: &str| CString::new(s).unwrap();

    unsafe {
        let rc = ffi::fastbrowser_register_webview_ops(&ops as *const FbWebViewOps);
        assert!(
            rc == 0,
            "register should succeed in this isolated binary, rc={rc}"
        );

        // 用 webview 引擎 init
        let r = take(ffi::fastbrowser_init(
            cstr(r#"{"engine":"webview"}"#).as_ptr(),
        ));
        let v: Value = serde_json::from_str(&r).unwrap();
        assert!(v.get("error").is_none(), "init failed: {v}");

        let out = take(ffi::fastbrowser_open(
            cstr("https://example.com/login").as_ptr(),
        ));
        let v: Value = serde_json::from_str(&out).unwrap();
        assert!(v["tab"].is_number());

        let snap = take(ffi::fastbrowser_snapshot());
        let v: Value = serde_json::from_str(&snap).unwrap();
        assert_eq!(v["title"], "Login");
        assert!(v["interactive"].as_array().unwrap().len() >= 4);

        let click = take(ffi::fastbrowser_tool_call(
            cstr("click").as_ptr(),
            cstr(r#"{"id":"c"}"#).as_ptr(),
        ));
        let v: Value = serde_json::from_str(&click).unwrap();
        assert_eq!(v["clicked"], "c");

        // cookie/storage 走引擎 overlay
        let cs = take(ffi::fastbrowser_tool_call(
            cstr("cookie_set").as_ptr(),
            cstr(r#"{"name":"k","value":"v","domain":"example.com"}"#).as_ptr(),
        ));
        let v: Value = serde_json::from_str(&cs).unwrap();
        assert_eq!(v["ok"], true);

        take(ffi::fastbrowser_shutdown());
    }
}

/// C ABI null 指针 / 坏输入的安全处理（返回 error JSON，不崩溃）。
#[test]
fn c_abi_null_and_bad_inputs() {
    let _g = FFI_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    use fastbrowser::sdk::ffi;
    let take = |p: *mut c_char| unsafe {
        let s = CStr::from_ptr(p).to_str().unwrap().to_string();
        ffi::fastbrowser_free_string(p);
        s
    };

    unsafe {
        // null config → error JSON
        let r = take(ffi::fastbrowser_init(std::ptr::null()));
        let v: Value = serde_json::from_str(&r).unwrap();
        assert!(v["error"]["kind"] == "invalid_argument");

        // 坏 JSON config → error JSON
        let r = take(ffi::fastbrowser_init(
            CString::new("not json").unwrap().as_ptr(),
        ));
        let v: Value = serde_json::from_str(&r).unwrap();
        assert!(v.get("error").is_some());

        // null url → error JSON（不 panic）
        let r = take(ffi::fastbrowser_open(std::ptr::null()));
        let v: Value = serde_json::from_str(&r).unwrap();
        assert!(v["error"]["kind"] == "invalid_argument");

        // 未 init 时 tool_call 未知工具 → error JSON
        let r = take(ffi::fastbrowser_tool_call(
            CString::new("nope").unwrap().as_ptr(),
            CString::new("{}").unwrap().as_ptr(),
        ));
        let v: Value = serde_json::from_str(&r).unwrap();
        assert!(v.get("error").is_some());

        take(ffi::fastbrowser_shutdown());
    }
}
