// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! CDP event model.

use serde_json::{json, Value};

/// A CDP event (method + params + optional sessionId).
#[derive(Debug, Clone)]
pub struct CdpEvent {
    pub method: String,
    pub params: Value,
    pub session_id: Option<String>,
}

impl CdpEvent {
    pub fn new(method: impl Into<String>, params: Value) -> Self {
        CdpEvent {
            method: method.into(),
            params,
            session_id: None,
        }
    }

    /// Events carrying a sessionId (multi-tab).
    pub fn with_session(
        method: impl Into<String>,
        params: Value,
        session_id: Option<String>,
    ) -> Self {
        CdpEvent {
            method: method.into(),
            params,
            session_id,
        }
    }

    /// Convenient for logging / serialization.
    pub fn to_json(&self) -> Value {
        json!({ "method": self.method, "params": self.params })
    }

    pub fn param(&self, key: &str) -> Option<&Value> {
        self.params.get(key)
    }

    pub fn url(&self) -> Option<String> {
        self.params
            .get("url")
            .and_then(Value::as_str)
            .map(String::from)
    }

    pub fn status(&self) -> Option<u16> {
        self.params
            .get("status")
            .and_then(Value::as_u64)
            .map(|v| v as u16)
    }
}

/// Parse a `Runtime.evaluate` response into a return value.
pub fn extract_evaluate_result(resp: &Value) -> Option<Value> {
    let result = resp.get("result")?;
    if let Some(exc) = result.get("exceptionDetails") {
        return Some(json!({ "error": exc.get("text").cloned().unwrap_or(Value::Null) }));
    }
    let value = result.get("result")?;
    if value.get("subtype").and_then(Value::as_str) == Some("error") {
        return Some(json!({ "error": value.get("description").cloned().unwrap_or(Value::Null) }));
    }
    Some(value.get("value").cloned().unwrap_or(Value::Null))
}

/// Parse a `Page.captureScreenshot` response into base64.
pub fn extract_screenshot_base64(resp: &Value) -> Option<String> {
    resp.get("result")
        .and_then(|r| r.get("data"))
        .and_then(Value::as_str)
        .map(String::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evaluate_result_extraction() {
        let resp = json!({ "result": { "result": { "type": "string", "value": "hello" } } });
        assert_eq!(extract_evaluate_result(&resp), Some(json!("hello")));
        let exc = json!({ "result": { "exceptionDetails": { "text": "SyntaxError" } } });
        assert_eq!(
            extract_evaluate_result(&exc),
            Some(json!({ "error": "SyntaxError" }))
        );
    }

    #[test]
    fn screenshot_extraction() {
        let resp = json!({ "result": { "data": "base64==" } });
        assert_eq!(
            extract_screenshot_base64(&resp),
            Some("base64==".to_string())
        );
    }
}
