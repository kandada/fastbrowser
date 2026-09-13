// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Engine abstraction layer ("core") — no platform dependencies.
//!
//! Defines contracts and types only; imports no concrete browser implementation:
//! - `trait`: the unified BrowserEngine interface
//! - `snapshot`: interactive-element snapshot (the primary data model for LLM perception)
//! - `tab/input/view/events/cookie`: core domain types
//! - `host`: host callback (plugin) trait used by mobile bridges and event pushing

pub mod cookie;
pub mod error;
pub mod events;
pub mod host;
pub mod inject;
pub mod input;
pub mod notify;
pub mod snapshot;
pub mod tab;
pub mod trait_;
pub mod view;

pub use cookie::Cookie;
pub use error::{EngineError, ErrorKind, Result};
pub use events::{ConsoleLevel, DialogInfo, PageEvent};
pub use host::{EncodedFrameSink, PageEventSink, ViewFrameSink, WebViewOps};
pub use input::{
    InputEvent, KeyEvent, KeyKind, Modifiers, MouseButton, MouseEvent, MouseKind, TouchEvent,
    TouchKind, TouchPoint, WheelEvent,
};
pub use notify::{wait_until, EventNotifier};
pub use snapshot::{
    ElementRef, FrameSnapshot, ImageInfo, InteractiveElement, LinkInfo, PageSnapshot, RefKind,
    SnapshotMeta,
};
pub use tab::{ContextId, HistoryEntry, TabId, TabInfo, TabOptions};
pub use trait_::{BrowserEngine, Capability, EngineCapabilities};
pub use view::{
    EncodedViewFrame, FrameStreamOptions, Image, Rect, RenderingMode, ViewFrame, ViewHandle,
    Viewport,
};

/// Per-tab event buffer cap. If the agent never calls `drain_events`, page events
/// (navigation/requests/console/DOM mutations) may accumulate; cap it to bound memory
/// in long sessions. Oldest entries are dropped first (keep the newest, matching the
/// agent's "read the latest state" semantics).
pub const MAX_BUFFERED_EVENTS: usize = 1024;

/// Per-tab intercepted (Fetch) request cache cap (CDP engine).
pub const MAX_PAUSED_REQUESTS: usize = 256;
