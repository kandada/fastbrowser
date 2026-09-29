// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! C ABI `fastbrowser_register_surface_ops` injection path + null safety.
//!
//! Standalone test binary (its own process) so the global `SURFACE_OPS`
//! singleton does not interfere with other tests. No `surface-*` feature is
//! required: the `SurfaceOps` trait, the C adapter and the FFI are always
//! compiled; only the concrete providers are feature-gated.

use std::ffi::{c_char, CStr, CString};

use serde_json::json;

use fastbrowser::sdk::ffi::{fastbrowser_register_surface_ops, FbSurfaceOps};

fn cstr(s: String) -> *mut c_char {
    CString::new(s).unwrap().into_raw()
}

extern "C" fn op_capabilities() -> *mut c_char {
    cstr(
        json!({"native_ax": true, "coordinate_input": false, "screenshot": false, "window_management": true})
            .to_string(),
    )
}

extern "C" fn op_list() -> *mut c_char {
    cstr(
        json!([{ "id": "desktop:42", "kind": "window", "title": "Fake", "active": true }])
            .to_string(),
    )
}

extern "C" fn op_snapshot(target: *const c_char, _opts: *const c_char) -> *mut c_char {
    let target = unsafe { CStr::from_ptr(target) }.to_str().unwrap();
    cstr(
        json!({
            "surface": { "id": target, "kind": "window", "title": "Fake", "active": true },
            "generation": 0,
            "meta": {},
            "timestamp_ms": 0,
            "root": { "node_id": "n0", "ref": format!("{target}:0"), "role": "window", "name": "Fake" }
        })
        .to_string(),
    )
}

extern "C" fn op_act(_ref: *const c_char, _action: *const c_char) -> *mut c_char {
    cstr(String::new())
}

extern "C" fn op_input(_target: *const c_char, _event: *const c_char) -> i32 {
    0
}

extern "C" fn op_screenshot(_target: *const c_char) -> *mut c_char {
    std::ptr::null_mut() // capability reported false
}

extern "C" fn op_poll_events(_target: *const c_char) -> *mut c_char {
    cstr("[]".to_string())
}

#[test]
fn surface_ops_round_trip_through_c_abi() {
    // NULL table is rejected.
    assert_eq!(
        unsafe { fastbrowser_register_surface_ops(std::ptr::null()) },
        -1
    );

    let ops = FbSurfaceOps {
        capabilities: Some(op_capabilities),
        list: Some(op_list),
        snapshot: Some(op_snapshot),
        act: Some(op_act),
        input: Some(op_input),
        screenshot: Some(op_screenshot),
        poll_events: Some(op_poll_events),
        free_string: None, // adapter must tolerate a missing free_string
    };
    assert_eq!(unsafe { fastbrowser_register_surface_ops(&ops) }, 0);

    // The adapter can be taken back and driven (this is what the provider uses).
    let surface_ops = fastbrowser::sdk::plugin::take_surface_ops().expect("ops registered");

    let caps: serde_json::Value =
        serde_json::from_str(&surface_ops.capabilities().unwrap()).unwrap();
    assert_eq!(caps["native_ax"], true);
    assert_eq!(caps["window_management"], true);

    let list = surface_ops.list().unwrap();
    assert!(list.contains("desktop:42"), "{list}");

    let snap = surface_ops.snapshot("desktop:42", "{}").unwrap();
    let snap_json: serde_json::Value = serde_json::from_str(&snap).unwrap();
    assert_eq!(snap_json["surface"]["id"], "desktop:42");
    assert_eq!(snap_json["root"]["role"], "window");

    assert_eq!(surface_ops.act("desktop:42:0", "{}").unwrap(), "");
    assert!(surface_ops.input("desktop:42", "{}").is_ok());
    // screenshot callback returns NULL → adapter surfaces an error.
    assert!(surface_ops.screenshot("desktop:42").is_err());
    assert_eq!(surface_ops.poll_events("desktop:42").unwrap(), "[]");
}
