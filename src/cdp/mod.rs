// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Shared CDP (Chrome DevTools Protocol) client.
//!
//! - `command`: command builders for Page / Runtime / DOM / Input / Network, etc.
//! - `event`: CDP event model
//! - `transport`: transport abstraction (WebSocket / test stub)
//! - `client`: request-response matching + event buffering

pub mod client;
pub mod command;
pub mod discovery;
pub mod event;
pub mod transport;

pub use client::CdpClient;
pub use command::Command;
pub use event::CdpEvent;
pub use transport::Transport;
