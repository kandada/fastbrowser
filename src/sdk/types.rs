// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Public types: Config (re-exported), SdkInfo, and re-exported tool/snapshot types.

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use crate::config::Config;
pub use crate::engine::{Image, PageSnapshot, TabInfo, ViewHandle, Viewport};
pub use crate::tools::tool::ToolSpec;

/// Kernel info (get_info).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SdkInfo {
    pub version: String,
    pub platform: String,
    pub engine: String,
    pub initialized: bool,
    pub tabs: usize,
    pub active_tab: Option<u32>,
    pub profiles: usize,
    pub tools: usize,
    /// Engine names compiled into this build (e.g. `["mock","chromium","bundled"]`).
    /// Lets callers (and the `fastbrowser info` CLI) detect missing features
    /// without launching a browser.
    pub engines: Vec<String>,
    /// Canonical engine the caller requested.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub engine_requested: String,
    /// Engine tier actually used (may differ from `engine_requested` on `auto`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub engine_used: String,
    /// `true` when `auto` degraded (e.g. fell back to `mock`).
    #[serde(default)]
    pub degraded: bool,
    /// Actionable guidance when degraded / a real browser was missing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

impl SdkInfo {
    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}
