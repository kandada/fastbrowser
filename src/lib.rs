// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! fastbrowser — 跨平台浏览器自动化内核（AI Agent 专用）。
//!
//! 分层（对齐 fastshell）：
//!   async_core/ → 异步壳（AsyncFastbrowser，feature async-core）：async 工具调用 + 事件推送 + 多标签编排
//!   engine/     → BrowserEngine trait + 全部核心类型（无平台依赖）
//!   engines/    → 具体引擎适配（mock / cef / webview，feature 门控）
//!   cdp/        → 共享 CDP 客户端（engine-cdp）
//!   tools/      → 30+ Agent 工具集（全部支持 {"tab": N}）
//!   session/    → 会话与状态管理（cookie/storage/profile）
//!   bridge/     → Runtime 装配层
//!   sdk/        → 对外 SDK + C ABI 出口

pub mod audit;
pub mod bridge;
pub mod config;
pub mod engine;
pub mod engines;
pub mod log;
pub mod png;
pub mod sdk;
pub mod session;
pub mod tools;

#[cfg(feature = "engine-cdp")]
pub mod cdp;

#[cfg(feature = "async-core")]
pub mod async_core;

#[cfg(feature = "async-core")]
pub use async_core::AsyncFastbrowser;

pub use config::Config;
pub use engine::{BrowserEngine, EngineError, ErrorKind};
pub use sdk::Fastbrowser;

/// 当前内核版本号。
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
