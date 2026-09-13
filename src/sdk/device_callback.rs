// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! 设备回调（对齐 fastshell 的 device_callback 语义）。
//!
//! 用于未来接入摄像头 / 麦克风 / 定位 / 剪贴板等设备能力，
//! 以及网络权限询问的进程级兜底。

use std::sync::{Arc, Mutex, OnceLock};

use serde_json::{json, Value};

/// 设备插件：宿主实现后注入，内核在需要设备能力时调用。
pub trait DevicePlugin: Send + Sync {
    /// 请求权限，返回是否授权。
    fn request_permission(&self, resource: &str) -> bool;
    /// 宿主能力描述。
    fn host_capabilities(&self) -> Value;
}

static GLOBAL_DEVICE_PLUGIN: OnceLock<Mutex<Option<Arc<dyn DevicePlugin>>>> = OnceLock::new();

fn slot() -> &'static Mutex<Option<Arc<dyn DevicePlugin>>> {
    GLOBAL_DEVICE_PLUGIN.get_or_init(|| Mutex::new(None))
}

/// 注入进程级设备插件。
pub fn set_global_device_plugin(plugin: Arc<dyn DevicePlugin>) {
    *slot().lock().unwrap_or_else(|e| e.into_inner()) = Some(plugin);
}

/// 取出全局设备插件。
pub fn global_device_plugin() -> Option<Arc<dyn DevicePlugin>> {
    slot().lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// 询问权限（插件缺省时默认拒绝，安全优先）。
pub fn request_permission(resource: &str) -> bool {
    global_device_plugin()
        .map(|p| p.request_permission(resource))
        .unwrap_or(false)
}

// ── 网络权限的进程级兜底表（对齐 fastshell 的全局 permission）─────

static NET_PERMISSIONS: OnceLock<Mutex<std::collections::HashMap<String, bool>>> = OnceLock::new();

fn net_slot() -> &'static Mutex<std::collections::HashMap<String, bool>> {
    NET_PERMISSIONS.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

/// 授予/拒绝网络访问（resource 形如 "network:<host>"）。
pub fn set_net_permission(resource: &str, allowed: bool) {
    net_slot()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(resource.to_string(), allowed);
}

pub fn check_net_permission(resource: &str) -> Option<bool> {
    net_slot()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(resource)
        .copied()
}

/// 默认的设备能力描述。
pub fn default_capabilities() -> Value {
    json!({
        "camera": false,
        "microphone": false,
        "location": false,
        "clipboard": false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_table() {
        set_net_permission("network:example.com", true);
        assert_eq!(check_net_permission("network:example.com"), Some(true));
        assert!(!request_permission("camera"));
    }
}
