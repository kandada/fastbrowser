// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Advanced-control tools: execute_js / evaluate_xpath / inject_css / block_request / intercept_request.

use serde_json::json;

use crate::engine::Capability;
use crate::tools::tool::Tool;

pub fn tools() -> Vec<Tool> {
    vec![
        execute_js(),
        evaluate_xpath(),
        inject_css(),
        block_request(),
        intercept_request(),
        set_basic_auth(),
    ]
}

fn execute_js() -> Tool {
    Tool::new(
        "execute_js",
        "Execute arbitrary JavaScript in the page context and return the JSON result.",
        json!({"script": {"type": "string", "required": true}}),
        r#"{"script": "document.title"}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let script = ctx.param_str("script")?;
            let value = ctx.runtime.engine().evaluate(tab, &script)?;
            Ok(json!({"result": value}))
        },
    )
}

fn evaluate_xpath() -> Tool {
    Tool::new(
        "evaluate_xpath",
        "Evaluate an XPath expression (e.g. //a, count(//button)) and return matches.",
        json!({"expr": {"type": "string", "required": true}}),
        r#"{"expr": "//a"}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let expr = ctx.param_str("expr")?;
            let value = ctx.runtime.engine().execute_xpath(tab, &expr)?;
            Ok(json!({"result": value}))
        },
    )
}

fn inject_css() -> Tool {
    Tool::new(
        "inject_css",
        "Inject a <style> CSS rule into the page.",
        json!({"css": {"type": "string", "required": true}}),
        r#"{"css": "body { background: #fff; }"}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let css = ctx.param_str("css")?;
            ctx.runtime.engine().inject_css(tab, &css)?;
            Ok(json!({"injected": true, "css_len": css.len()}))
        },
    )
}

fn block_request() -> Tool {
    Tool::new(
        "block_request",
        "Block network requests matching URL patterns (e.g. \"*ads*\"). 'enabled' toggles.",
        json!({
            "patterns": {"type": "array", "items": {"type": "string"}, "required": true},
            "enabled": {"type": "boolean", "default": true}
        }),
        r#"{"patterns": ["*ads*"]}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let patterns: Vec<String> = ctx.param("patterns")?;
            let enabled = ctx.param_opt::<bool>("enabled")?.unwrap_or(true);
            ctx.runtime
                .engine()
                .block_requests(tab, &patterns, enabled)?;
            Ok(json!({"patterns": patterns, "enabled": enabled}))
        },
    )
    .requires(&[Capability::NetworkControl])
}

fn intercept_request() -> Tool {
    Tool::new(
        "intercept_request",
        "Intercept network requests matching URL patterns. 'enabled' toggles.",
        json!({
            "patterns": {"type": "array", "items": {"type": "string"}, "required": true},
            "enabled": {"type": "boolean", "default": true}
        }),
        r#"{"patterns": ["*.png"]}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let patterns: Vec<String> = ctx.param("patterns")?;
            let enabled = ctx.param_opt::<bool>("enabled")?.unwrap_or(true);
            ctx.runtime
                .engine()
                .intercept_requests(tab, &patterns, enabled)?;
            Ok(json!({"patterns": patterns, "enabled": enabled}))
        },
    )
    .requires(&[Capability::NetworkControl])
}

fn set_basic_auth() -> Tool {
    Tool::new(
        "set_basic_auth",
        "Set Basic Auth credentials for the active tab; subsequent 401 auth challenges are answered automatically.",
        json!({
            "username": {"type": "string", "required": true},
            "password": {"type": "string", "required": true}
        }),
        r#"{"username": "alice", "password": "secret"}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let u = ctx.param_str("username")?;
            let p = ctx.param_str("password")?;
            ctx.runtime.engine().set_basic_auth(tab, &u, &p)?;
            Ok(json!({"ok": true}))
        },
    )
    .requires(&[Capability::Cdp])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::Runtime;
    use crate::config::Config;
    use crate::engine::Result;
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

    fn call(tool: &Tool, r: &Runtime, params: Value) -> Result<Value> {
        tool.run(&ToolContext { runtime: r, params })
    }

    #[test]
    fn js_and_xpath() {
        let r = runtime();
        let v = call(
            &execute_js(),
            &r,
            json!({"script": "document.querySelectorAll('a').length"}),
        )
        .unwrap();
        assert_eq!(v["result"], json!(1));
        let v = call(&evaluate_xpath(), &r, json!({"expr": "count(//button)"})).unwrap();
        assert_eq!(v["result"], json!(1));
    }

    #[test]
    fn css_and_net() {
        let r = runtime();
        let v = call(&inject_css(), &r, json!({"css": "body{}"})).unwrap();
        assert!(v["injected"].as_bool().unwrap());
        let _ = call(&block_request(), &r, json!({"patterns": ["*ads*"]}));
        let _ = call(&intercept_request(), &r, json!({"patterns": ["*.png"]}));
    }
}
