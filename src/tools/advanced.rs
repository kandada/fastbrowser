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
        "Execute JavaScript in the page context and return the JSON result. Provide 'script' (an expression, IIFE, or statement block with a top-level `return`), or 'function' (Playwright-style: a function that will be CALLED, e.g. `() => document.title`).",
        json!({
            "script": {"type": "string", "required": false},
            "function": {"type": "string", "required": false, "description": "Playwright-style function to call"}
        }),
        r#"{"script": "document.title"}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            // Playwright MCP `browser_evaluate` sends a *function* to call
            // (not an expression to evaluate). Call it and take the result.
            if let Some(f) = ctx.params.get("function").and_then(|v| v.as_str()) {
                if !f.trim().is_empty() {
                    let call = call_function(f);
                    let value = ctx.runtime.engine().evaluate(tab, &call)?;
                    return Ok(json!({"result": crate::tools::tool::surface_js_error(value)?}));
                }
            }
            let script = ctx.param_str("script")?;
            let value = ctx.runtime.engine().evaluate(tab, &wrap_script(&script))?;
            Ok(json!({"result": crate::tools::tool::surface_js_error(value)?}))
        },
    )
}

/// Call a Playwright-style function expression: `() => x` / `function(){...}` →
/// `(<fn>)()`. Used by `execute_js` for the `function` param.
pub fn call_function(f: &str) -> String {
    format!("({f})()")
}

/// Allow both expression-style (`document.title`) and statement-style scripts
/// with a top-level `return`. A bare `return` is illegal at the top level, so
/// when the script contains one we wrap it in an IIFE; otherwise evaluate as-is
/// (no `eval`, so page CSP is not a concern).
///
/// A script that is already a complete expression — most importantly a
/// self-contained IIFE like `(function(){ … return x; })()` — must NOT be
/// wrapped again: an outer IIFE with no `return` yields `undefined`, which the
/// host serializes to `null` (this made `execute_js` silently return null).
pub fn wrap_script(script: &str) -> String {
    let s = script.trim();
    // Already a complete expression — most importantly a self-contained IIFE
    // like `(function(){ … return x; })()` — evaluate as-is. Wrapping it again
    // produces an outer IIFE with no `return` → undefined → null (this silently
    // broke `execute_js` for every IIFE the model wrote).
    if s.starts_with('(') && s.ends_with(')') {
        return script.to_string();
    }
    // Only statement blocks that actually use `return` need an IIFE wrapper.
    // A bare expression that merely mentions "return" (e.g. a string literal)
    // must not be wrapped.
    let needs_wrap = s.starts_with("return")
        || (script.contains("return") && (script.contains(';') || script.contains('\n')));
    if needs_wrap {
        format!("(function(){{ {script} }})()")
    } else if let Some((prefix, last)) = split_last_statement(script) {
        // Multi-statement script without an explicit `return`: expose the last
        // statement's value (REPL-like completion value) instead of `undefined`.
        let prefix = prefix.trim();
        let last = last.trim();
        let keyword_statement = [
            "if", "for", "while", "switch", "try", "function", "class", "var", "let", "const",
            "return", "throw", "{",
        ]
        .iter()
        .any(|kw| last == *kw || last.starts_with(&format!("{kw} ")));
        if !last.is_empty()
            && !prefix.is_empty()
            && !last.ends_with(';')
            && !keyword_statement
            && !last.contains(';')
        {
            return format!("(function(){{ {prefix}; return ({last}); }})()");
        }
        script.to_string()
    } else {
        script.to_string()
    }
}

/// Split `s` into (before, last-statement) at the last top-level `;` or
/// newline (quote/escape/bracket aware). `None` when there is no separator.
fn split_last_statement(s: &str) -> Option<(&str, &str)> {
    let b = s.as_bytes();
    let n = b.len();
    let mut depth = 0i32;
    let mut in_s = false;
    let mut in_d = false;
    let mut last: Option<usize> = None;
    let mut i = 0;
    while i < n {
        let c = b[i];
        if in_s {
            if c == b'\'' {
                in_s = false;
            }
            i += 1;
            continue;
        }
        if in_d {
            if c == b'\\' && i + 1 < n {
                i += 2;
                continue;
            }
            if c == b'"' {
                in_d = false;
            }
            i += 1;
            continue;
        }
        match c {
            b'\\' if i + 1 < n => {
                i += 2;
                continue;
            }
            b'\'' => in_s = true,
            b'"' => in_d = true,
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b';' | b'\n' if depth == 0 => last = Some(i),
            _ => {}
        }
        i += 1;
    }
    let pos = last?;
    Some((&s[..pos], &s[pos + 1..]))
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
            Ok(json!({"result": crate::tools::tool::surface_js_error(value)?}))
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

    #[test]
    fn wrap_script_returns_last_statement_value() {
        // Multi-statement script without an explicit `return` → last value.
        let w = wrap_script("a=1; b=2");
        assert!(w.contains("return (b=2)"), "got {w}");
        // A single expression is left untouched.
        assert_eq!(wrap_script("document.title"), "document.title");
        // An explicit `return` still becomes an IIFE.
        assert!(wrap_script("x(); return 1;").starts_with("(function(){"));
        // A trailing block statement is not turned into `return (if …)`.
        let w = wrap_script("a=1; if (x) { y=2; }");
        assert!(!w.contains("return (if"), "got {w}");
    }

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
    fn wrap_script_handles_return() {
        // Expression → unchanged (evaluated as-is).
        assert_eq!(wrap_script("document.title"), "document.title");
        // Top-level return → wrapped in an IIFE so it is legal.
        let w = wrap_script("return document.title");
        assert!(w.starts_with("(function(){"));
        assert!(w.contains("return document.title"));
    }

    #[test]
    fn wrap_script_leaves_iife_untouched() {
        // Regression: an agent-supplied IIFE already contains `return`; wrapping
        // it again produced an outer IIFE with no return → undefined → null.
        let iife = "(function(){ const x = 1; return x; })()";
        assert_eq!(wrap_script(iife), iife);
        let async_iife = "(async () => { return 42; })()";
        assert_eq!(wrap_script(async_iife), async_iife);
        let arrow_iife = "(() => { return document.title; })()";
        assert_eq!(wrap_script(arrow_iife), arrow_iife);
        // A bare expression that merely mentions `return` in a string stays as-is.
        assert_eq!(wrap_script("'return'"), "'return'");
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
