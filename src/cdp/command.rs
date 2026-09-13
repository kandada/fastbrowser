// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! CDP command builders.

use serde_json::{json, Value};

/// A CDP command.
#[derive(Debug, Clone)]
pub struct Command {
    pub method: String,
    pub params: Value,
}

impl Command {
    pub fn new(method: impl Into<String>, params: Value) -> Self {
        Command {
            method: method.into(),
            params,
        }
    }
}

/// Page domain.
pub mod page {
    use super::*;

    pub fn navigate(url: &str) -> Command {
        Command::new("Page.navigate", json!({ "url": url }))
    }
    pub fn reload(ignore_cache: bool) -> Command {
        Command::new("Page.reload", json!({ "ignoreCache": ignore_cache }))
    }
    pub fn stop_loading() -> Command {
        Command::new("Page.stopLoading", json!({}))
    }
    pub fn enable() -> Command {
        Command::new("Page.enable", json!({}))
    }
    pub fn capture_screenshot(format: &str, quality: u8, from_surface: bool) -> Command {
        Command::new(
            "Page.captureScreenshot",
            json!({
                "format": format,
                "quality": quality,
                "fromSurface": from_surface,
                "captureBeyondViewport": false,
            }),
        )
    }
    pub fn get_navigation_history() -> Command {
        Command::new("Page.getNavigationHistory", json!({}))
    }
    pub fn go_back() -> Command {
        Command::new("Page.goBack", json!({}))
    }
    pub fn go_forward() -> Command {
        Command::new("Page.goForward", json!({}))
    }
    pub fn get_frame_tree() -> Command {
        Command::new("Page.getFrameTree", json!({}))
    }
    pub fn add_script_to_evaluate_on_new_document(source: &str) -> Command {
        Command::new(
            "Page.addScriptToEvaluateOnNewDocument",
            json!({ "source": source }),
        )
    }
}

/// Runtime domain.
pub mod runtime {
    use super::*;

    pub fn evaluate(expression: &str, await_promise: bool, return_by_value: bool) -> Command {
        Command::new(
            "Runtime.evaluate",
            json!({
                "expression": expression,
                "awaitPromise": await_promise,
                "returnByValue": return_by_value,
                "userGesture": true,
            }),
        )
    }
    pub fn enable() -> Command {
        Command::new("Runtime.enable", json!({}))
    }
}

/// DOM domain.
pub mod dom {
    use super::*;

    pub fn get_document(depth: u32, pierce: bool) -> Command {
        Command::new(
            "DOM.getDocument",
            json!({ "depth": depth, "pierce": pierce }),
        )
    }
    pub fn query_selector(node_id: i64, selector: &str) -> Command {
        Command::new(
            "DOM.querySelector",
            json!({ "nodeId": node_id, "selector": selector }),
        )
    }
    pub fn query_selector_all(node_id: i64, selector: &str) -> Command {
        Command::new(
            "DOM.querySelectorAll",
            json!({ "nodeId": node_id, "selector": selector }),
        )
    }
    pub fn describe_node(node_id: i64, depth: u32) -> Command {
        Command::new(
            "DOM.describeNode",
            json!({ "nodeId": node_id, "depth": depth }),
        )
    }
    pub fn request_child_nodes(node_id: i64, depth: u32) -> Command {
        Command::new(
            "DOM.requestChildNodes",
            json!({ "nodeId": node_id, "depth": depth }),
        )
    }
    pub fn get_outer_html(node_id: i64) -> Command {
        Command::new("DOM.getOuterHTML", json!({ "nodeId": node_id }))
    }
    pub fn set_attribute_value(node_id: i64, name: &str, value: &str) -> Command {
        Command::new(
            "DOM.setAttributeValue",
            json!({ "nodeId": node_id, "name": name, "value": value }),
        )
    }
    pub fn get_document_enable() -> Command {
        Command::new("DOM.enable", json!({}))
    }
}

/// Input domain.
pub mod input {
    use super::*;

    pub fn dispatch_mouse_event(
        kind: &str,
        x: f64,
        y: f64,
        button: &str,
        click_count: u8,
    ) -> Command {
        Command::new(
            "Input.dispatchMouseEvent",
            json!({
                "type": kind,
                "x": x,
                "y": y,
                "button": button,
                "clickCount": click_count,
            }),
        )
    }
    pub fn insert_text(text: &str) -> Command {
        Command::new("Input.insertText", json!({ "text": text }))
    }
    pub fn dispatch_key_event(kind: &str, key: &str, code: &str, text: &str) -> Command {
        Command::new(
            "Input.dispatchKeyEvent",
            json!({
                "type": kind,
                "key": key,
                "code": code,
                "text": text,
            }),
        )
    }
    pub fn dispatch_touch_event(kind: &str, points: Value) -> Command {
        Command::new(
            "Input.dispatchTouchEvent",
            json!({ "type": kind, "touchPoints": points }),
        )
    }
    pub fn dispatch_mouse_wheel(x: f64, y: f64, dx: f64, dy: f64) -> Command {
        Command::new(
            "Input.dispatchMouseEvent",
            json!({
                "type": "mouseWheel",
                "x": x,
                "y": y,
                "deltaX": dx,
                "deltaY": dy,
            }),
        )
    }
}

/// Network domain.
pub mod network {
    use super::*;

    pub fn enable() -> Command {
        Command::new("Network.enable", json!({}))
    }
    pub fn set_blocked_urls(urls: Vec<String>) -> Command {
        Command::new("Network.setBlockedURLs", json!({ "urls": urls }))
    }
    pub fn set_cache_disabled(disabled: bool) -> Command {
        Command::new(
            "Network.setCacheDisabled",
            json!({ "cacheDisabled": disabled }),
        )
    }
    pub fn set_request_interception(patterns: Vec<String>) -> Command {
        Command::new(
            "Fetch.enable",
            json!({
                "patterns": patterns
                    .iter()
                    .map(|u| json!({ "urlPattern": u, "requestStage": "Request" }))
                    .collect::<Vec<_>>(),
            }),
        )
    }
}

/// Emulation domain.
pub mod emulation {
    use super::*;

    pub fn set_device_metrics_override(
        width: u32,
        height: u32,
        scale: f64,
        mobile: bool,
    ) -> Command {
        Command::new(
            "Emulation.setDeviceMetricsOverride",
            json!({
                "width": width,
                "height": height,
                "deviceScaleFactor": scale,
                "mobile": mobile,
            }),
        )
    }
    pub fn set_user_agent_override(ua: &str) -> Command {
        Command::new("Emulation.setUserAgentOverride", json!({ "userAgent": ua }))
    }
}

/// Target domain.
pub mod target {
    use super::*;

    pub fn create_target(url: &str, new_window: bool) -> Command {
        Command::new(
            "Target.createTarget",
            json!({ "url": url, "newWindow": new_window }),
        )
    }
    pub fn close_target(target_id: &str) -> Command {
        Command::new("Target.closeTarget", json!({ "targetId": target_id }))
    }
    pub fn get_targets() -> Command {
        Command::new("Target.getTargets", json!({}))
    }
    pub fn activate_target(target_id: &str) -> Command {
        Command::new("Target.activateTarget", json!({ "targetId": target_id }))
    }
    pub fn attach_to_target(target_id: &str, flatten: bool) -> Command {
        Command::new(
            "Target.attachToTarget",
            json!({ "targetId": target_id, "flatten": flatten }),
        )
    }
}

/// CSS domain.
pub mod css {
    use super::*;

    pub fn add_style_text(text: &str) -> Command {
        Command::new(
            "CSS.addRule",
            json!({ "styleSheetId": "", "ruleText": text }),
        )
    }
    pub fn enable() -> Command {
        Command::new("CSS.enable", json!({}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigate_command_shape() {
        let c = page::navigate("https://example.com");
        assert_eq!(c.method, "Page.navigate");
        assert_eq!(c.params["url"], "https://example.com");
    }

    #[test]
    fn blocked_urls_shape() {
        let c = network::set_blocked_urls(vec!["*ads*".into()]);
        assert_eq!(c.params["urls"][0], "*ads*");
    }
}
