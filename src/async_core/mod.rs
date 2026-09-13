// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! 异步核心 + 异步壳（`AsyncFastbrowser`）。
//!
//! 目标架构（"异步核心 + 双壳"）：
//! ```text
//! 异步壳  AsyncFastbrowser（async fn + 事件推送 + 多标签编排/并发）
//!    │        驱动
//! 异步核心  每标签事件流广播 + tokio 编排（select/timeout）+ 阻塞池并发
//!    │        复用（不重写）
//! 同步内核  Fastbrowser / BrowserEngine / C ABI / CLI
//! ```
//!
//! 关键设计：
//! - **真异步的并发部分**：后台泵任务（tokio task）把每标签的页面事件推送到
//!   `tokio::sync::broadcast` 频道；编排（`wait_for_event` / `wait_any` /
//!   `wait_for_navigation` / `run_concurrently` / `open_many`）全部基于
//!   `tokio::time::timeout` 与 `JoinSet`——这是 tokio 原生的异步编排，不是轮询包装。
//! - **引擎操作走阻塞池**：浏览器引擎是同步内核（含 Actionability 等待等），
//!   异步壳用 `tokio::task::spawn_blocking` 调度到 tokio 阻塞池，调用方（async
//!   执行器）绝不阻塞线程；不同标签页的操作在阻塞池上**真并发**（per-tab 锁 +
//!   CDP 流水线保证）。
//! - **与同步壳共存**：同一个 `Fastbrowser` 实例可同时被同步壳与异步壳使用；
//!   事件泵只在存在异步订阅者（`receiver_count > 0`）时才抽取事件，避免与同步
//!   工具的 `drain_events` 争抢。
//!
//! 启用：feature `async-core`（依赖 tokio）。

pub mod orchestrate;
pub mod shell;

pub use shell::AsyncFastbrowser;
