// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Tool set: exposes 30+ LLM-callable tools.
//!
//! One module per functional domain; function naming follows thought.md §3.2.

pub mod advanced;
pub mod agent;
pub mod cookies;
pub mod device;
pub mod dialog;
pub mod extract;
pub mod form;
pub mod interact;
pub mod nav;
pub mod network;
pub mod page;
pub mod pdf;
pub mod registry;
pub mod tabs;
pub mod tool;
pub mod wait;

pub use registry::ToolRegistry;
pub use tool::{Tool, ToolContext, ToolSpec};

/// Tool-call entry point (shared by the SDK and CLI).
pub fn call(
    runtime: &crate::bridge::Runtime,
    name: &str,
    params: serde_json::Value,
) -> crate::engine::Result<serde_json::Value> {
    let registry = ToolRegistry::new();
    registry.call(runtime, name, params)
}
