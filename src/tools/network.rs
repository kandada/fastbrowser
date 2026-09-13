// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Network request interception tools (used with intercept_request).

use serde_json::{json, Value};

use crate::engine::Capability;
use crate::tools::tool::Tool;

pub fn tools() -> Vec<Tool> {
    vec![
        list_pending_requests(),
        fulfill_request(),
        modify_response(),
        continue_request(),
        abort_request(),
    ]
}

fn list_pending_requests() -> Tool {
    Tool::new(
        "list_pending_requests",
        "List requests currently paused by Fetch interception (request_id, url, method).",
        json!({}),
        r#"{}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let reqs = ctx.runtime.engine().pending_requests(tab);
            Ok(json!({"requests": reqs, "count": reqs.len()}))
        },
    )
    .requires(&[Capability::NetworkControl])
}

fn fulfill_request() -> Tool {
    Tool::new(
        "fulfill_request",
        "Respond to a paused request with a custom status and body (base64). Headers is an optional JSON array of {name,value}.",
        json!({
            "request_id": {"type": "string", "description": "id from list_pending_requests", "required": true},
            "status": {"type": "integer", "default": 200},
            "body_b64": {"type": "string", "description": "base64-encoded response body", "required": false}
        }),
        r#"{"request_id": "1234", "status": 200, "body_b64": "aGVsbG8="}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let request_id = ctx.param_str("request_id")?;
            let status = ctx.param_opt::<u16>("status")?.unwrap_or(200);
            let body_b64 = ctx.param_opt::<String>("body_b64")?;
            let headers = ctx.param_opt::<Value>("headers")?;
            let body = match body_b64 {
                Some(b) => {
                    use base64::Engine;
                    Some(
                        base64::engine::general_purpose::STANDARD
                            .decode(b.as_bytes())
                            .map_err(|e| crate::engine::EngineError::invalid(format!("body_b64: {e}")))?,
                    )
                }
                None => None,
            };
            ctx.runtime
                .engine()
                .fulfill_request(tab, &request_id, status, body, headers)?;
            Ok(json!({"fulfilled": request_id, "status": status, "ok": true}))
        },
    )
    .requires(&[Capability::NetworkControl])
}

fn modify_response() -> Tool {
    Tool::new(
        "modify_response",
        "Replace the response of a paused request with a custom status/body/headers.",
        json!({
            "request_id": {"type": "string", "required": true},
            "status": {"type": "integer", "default": 200},
            "body_b64": {"type": "string", "required": false}
        }),
        r#"{"request_id": "1234", "status": 200, "body_b64": "aGVsbG8="}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let request_id = ctx.param_str("request_id")?;
            let status = ctx.param_opt::<u16>("status")?.unwrap_or(200);
            let body_b64 = ctx.param_opt::<String>("body_b64")?;
            let headers = ctx.param_opt::<Value>("headers")?;
            let body = match body_b64 {
                Some(b) => {
                    use base64::Engine;
                    Some(
                        base64::engine::general_purpose::STANDARD
                            .decode(b.as_bytes())
                            .map_err(|e| {
                                crate::engine::EngineError::invalid(format!("body_b64: {e}"))
                            })?,
                    )
                }
                None => None,
            };
            ctx.runtime
                .engine()
                .modify_response(tab, &request_id, status, body, headers)?;
            Ok(json!({"modified": request_id, "status": status, "ok": true}))
        },
    )
    .requires(&[Capability::NetworkControl])
}

fn continue_request() -> Tool {
    Tool::new(
        "continue_request",
        "Let a paused request proceed to the network normally.",
        json!({"request_id": {"type": "string", "required": true}}),
        r#"{"request_id": "1234"}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let request_id = ctx.param_str("request_id")?;
            ctx.runtime.engine().continue_request(tab, &request_id)?;
            Ok(json!({"continued": request_id, "ok": true}))
        },
    )
    .requires(&[Capability::NetworkControl])
}

fn abort_request() -> Tool {
    Tool::new(
        "abort_request",
        "Abort a paused request (fail it client-side).",
        json!({"request_id": {"type": "string", "required": true}}),
        r#"{"request_id": "1234"}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let request_id = ctx.param_str("request_id")?;
            ctx.runtime.engine().abort_request(tab, &request_id)?;
            Ok(json!({"aborted": request_id, "ok": true}))
        },
    )
    .requires(&[Capability::NetworkControl])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::Runtime;
    use crate::config::Config;
    use crate::engine::TabOptions;
    use crate::tools::tool::ToolContext;
    use serde_json::Value;

    fn runtime() -> Runtime {
        let r = Runtime::new(
            Box::new(crate::engines::mock::MockEngine::new()),
            Config::default(),
        );
        r.engine()
            .create_tab("https://example.com", &TabOptions::default())
            .unwrap();
        r
    }

    #[test]
    fn list_pending_empty_on_mock() {
        let r = runtime();
        let v = list_pending_requests()
            .run(&ToolContext {
                runtime: &r,
                params: json!({}),
            })
            .unwrap();
        assert_eq!(v["count"], 0);
    }

    #[test]
    fn interceptor_tools_error_on_mock() {
        let r = runtime();
        let res = fulfill_request().run(&ToolContext {
            runtime: &r,
            params: json!({"request_id": "x"}),
        });
        assert!(res.is_err());
        let _ = Value::Null;
    }
}
