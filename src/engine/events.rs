// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Page event stream (navigation / load / console / network / tab lifecycle).

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::engine::TabId;

/// Console level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConsoleLevel {
    Log,
    Warning,
    Error,
    Debug,
    Info,
}

impl ConsoleLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            ConsoleLevel::Log => "log",
            ConsoleLevel::Warning => "warning",
            ConsoleLevel::Error => "error",
            ConsoleLevel::Debug => "debug",
            ConsoleLevel::Info => "info",
        }
    }
}

/// Pending JS dialog info (alert/confirm/prompt/beforeunload).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DialogInfo {
    pub message: String,
    /// "alert" | "confirm" | "prompt" | "beforeunload"。
    pub kind: String,
    /// Default input for a prompt dialog.
    pub default_prompt: Option<String>,
}

/// Page event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PageEvent {
    NavigationStarted {
        url: String,
    },
    NavigationCompleted {
        url: String,
        status: u16,
    },
    Loaded {
        url: String,
    },
    TitleChanged {
        title: String,
    },
    DomChanged,
    Console {
        level: ConsoleLevel,
        message: String,
    },
    Request {
        url: String,
        method: String,
    },
    Response {
        url: String,
        status: u16,
    },
    TabOpened {
        tab: TabId,
    },
    TabClosed {
        tab: TabId,
    },
    Download {
        url: String,
    },
    /// A JS dialog opened (alert/confirm/prompt); handle it with dialog_accept/dialog_dismiss.
    Dialog {
        message: String,
        kind: String,
    },
    Error {
        message: String,
    },
}

impl PageEvent {
    /// Event type name (for the LLM / logs).
    pub fn name(&self) -> &'static str {
        match self {
            PageEvent::NavigationStarted { .. } => "navigation_started",
            PageEvent::NavigationCompleted { .. } => "navigation_completed",
            PageEvent::Loaded { .. } => "loaded",
            PageEvent::TitleChanged { .. } => "title_changed",
            PageEvent::DomChanged => "dom_changed",
            PageEvent::Console { .. } => "console",
            PageEvent::Request { .. } => "request",
            PageEvent::Response { .. } => "response",
            PageEvent::TabOpened { .. } => "tab_opened",
            PageEvent::TabClosed { .. } => "tab_closed",
            PageEvent::Download { .. } => "download",
            PageEvent::Dialog { .. } => "dialog",
            PageEvent::Error { .. } => "error",
        }
    }

    /// Serialize to uniform JSON: `{"type": "...", ...fields}`.
    pub fn to_json(&self) -> Value {
        match self {
            PageEvent::NavigationStarted { url } => json!({ "type": self.name(), "url": url }),
            PageEvent::NavigationCompleted { url, status } => {
                json!({ "type": self.name(), "url": url, "status": status })
            }
            PageEvent::Loaded { url } => json!({ "type": self.name(), "url": url }),
            PageEvent::TitleChanged { title } => json!({ "type": self.name(), "title": title }),
            PageEvent::DomChanged => json!({ "type": self.name() }),
            PageEvent::Console { level, message } => json!({
                "type": self.name(), "level": level.as_str(), "message": message
            }),
            PageEvent::Request { url, method } => {
                json!({ "type": self.name(), "url": url, "method": method })
            }
            PageEvent::Response { url, status } => {
                json!({ "type": self.name(), "url": url, "status": status })
            }
            PageEvent::TabOpened { tab } => json!({ "type": self.name(), "tab": tab }),
            PageEvent::TabClosed { tab } => json!({ "type": self.name(), "tab": tab }),
            PageEvent::Download { url } => json!({ "type": self.name(), "url": url }),
            PageEvent::Dialog { message, kind } => {
                json!({ "type": self.name(), "message": message, "kind": kind })
            }
            PageEvent::Error { message } => json!({ "type": self.name(), "message": message }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_names() {
        assert_eq!(PageEvent::DomChanged.name(), "dom_changed");
        assert_eq!(PageEvent::Loaded { url: "u".into() }.name(), "loaded");
    }

    #[test]
    fn event_json_shape() {
        let e = PageEvent::NavigationCompleted {
            url: "u".into(),
            status: 200,
        };
        let v = e.to_json();
        assert_eq!(v["type"], "navigation_completed");
        assert_eq!(v["status"], 200);
    }
}
