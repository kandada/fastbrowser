// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Session-management tools: cookie_get/set/clear, storage_get/set.

use serde_json::json;

use crate::engine::Cookie;
use crate::tools::tool::Tool;

pub fn tools() -> Vec<Tool> {
    vec![
        cookie_get(),
        cookie_set(),
        cookie_clear(),
        clear_cookies(),
        storage_get(),
        storage_set(),
        storage_get_all(),
        clear_storage(),
    ]
}

fn clear_cookies() -> Tool {
    Tool::new(
        "clear_cookies",
        "Clear ALL cookies of the active tab.",
        json!({}),
        r#"{}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            ctx.runtime.engine().cookie_clear(tab, None, None)?;
            Ok(json!({"cleared": true}))
        },
    )
}

fn storage_get_all() -> Tool {
    Tool::new(
        "storage_get_all",
        "Return ALL localStorage entries of the active tab.",
        json!({}),
        r#"{}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let all = ctx.runtime.engine().storage_all(tab)?;
            Ok(json!({"storage": all}))
        },
    )
}

fn clear_storage() -> Tool {
    Tool::new(
        "clear_storage",
        "Clear ALL localStorage of the active tab.",
        json!({}),
        r#"{}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            ctx.runtime.engine().storage_clear(tab)?;
            Ok(json!({"cleared": true}))
        },
    )
}

fn cookie_get() -> Tool {
    Tool::new(
        "cookie_get",
        "Get cookies of the active tab, optionally filtered by 'domain'.",
        json!({"domain": {"type": "string", "required": false}}),
        r#"{"domain": "example.com"}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let domain = ctx.param_opt::<String>("domain")?;
            let cookies = ctx.runtime.engine().cookie_get(tab, domain.as_deref())?;
            Ok(json!({"cookies": cookies}))
        },
    )
}

fn cookie_set() -> Tool {
    Tool::new(
        "cookie_set",
        "Set a cookie on the active tab. Params: name, value, domain, path?, expires?, secure?, http_only?, same_site?",
        json!({
            "name": {"type": "string", "required": true},
            "value": {"type": "string", "required": true},
            "domain": {"type": "string", "required": true}
        }),
        r#"{"name": "session", "value": "abc", "domain": "example.com"}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let cookie = Cookie {
                name: ctx.param_str("name")?,
                value: ctx.param_str("value")?,
                domain: ctx.param_str("domain")?,
                path: ctx.param_opt::<String>("path")?.unwrap_or_else(|| "/".into()),
                expires: ctx.param_opt::<i64>("expires")?,
                secure: ctx.param_opt::<bool>("secure")?.unwrap_or(false),
                http_only: ctx.param_opt::<bool>("http_only")?.unwrap_or(false),
                same_site: ctx.param_opt::<String>("same_site")?,
            };
            ctx.runtime.engine().cookie_set(tab, &cookie)?;
            Ok(json!({"set": cookie.name, "ok": true}))
        },
    )
}

fn cookie_clear() -> Tool {
    Tool::new(
        "cookie_clear",
        "Clear cookies of the active tab, filtered by 'domain' and/or 'name'.",
        json!({
            "domain": {"type": "string", "required": false},
            "name": {"type": "string", "required": false}
        }),
        r#"{"domain": "example.com"}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let domain = ctx.param_opt::<String>("domain")?;
            let name = ctx.param_opt::<String>("name")?;
            ctx.runtime
                .engine()
                .cookie_clear(tab, domain.as_deref(), name.as_deref())?;
            Ok(json!({"cleared": true}))
        },
    )
}

fn storage_get() -> Tool {
    Tool::new(
        "storage_get",
        "Read a localStorage value by 'key' from the active tab.",
        json!({"key": {"type": "string", "required": true}}),
        r#"{"key": "token"}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let key = ctx.param_str("key")?;
            let value = ctx.runtime.engine().storage_get(tab, &key)?;
            Ok(json!({"key": key, "value": value}))
        },
    )
}

fn storage_set() -> Tool {
    Tool::new(
        "storage_set",
        "Write a localStorage 'value' for 'key' on the active tab.",
        json!({
            "key": {"type": "string", "required": true},
            "value": {"type": "string", "required": true}
        }),
        r#"{"key": "token", "value": "abc"}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let key = ctx.param_str("key")?;
            let value = ctx.param_str("value")?;
            ctx.runtime.engine().storage_set(tab, &key, &value)?;
            Ok(json!({"key": key, "ok": true}))
        },
    )
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
    fn cookie_roundtrip() {
        let r = runtime();
        let _ = call(
            &cookie_set(),
            &r,
            json!({"name": "sid", "value": "x", "domain": "example.com"}),
        );
        let v = call(&cookie_get(), &r, json!({"domain": "example.com"})).unwrap();
        assert_eq!(v["cookies"][0]["name"], "sid");
        let _ = call(&cookie_clear(), &r, json!({"domain": "example.com"}));
        let v = call(&cookie_get(), &r, json!({})).unwrap();
        assert_eq!(v["cookies"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn storage_roundtrip() {
        let r = runtime();
        let _ = call(&storage_set(), &r, json!({"key": "k", "value": "v"}));
        let v = call(&storage_get(), &r, json!({"key": "k"})).unwrap();
        assert_eq!(v["value"], "v");
    }

    #[test]
    fn clear_all_and_dump() {
        let r = runtime();
        let _ = call(
            &cookie_set(),
            &r,
            json!({"name": "a", "value": "1", "domain": "example.com"}),
        );
        let _ = call(
            &cookie_set(),
            &r,
            json!({"name": "b", "value": "2", "domain": "example.org"}),
        );
        let _ = call(&storage_set(), &r, json!({"key": "k", "value": "v"}));
        let dump = call(&storage_get_all(), &r, json!({})).unwrap();
        assert_eq!(dump["storage"]["k"], "v");
        call(&clear_cookies(), &r, json!({})).unwrap();
        let v = call(&cookie_get(), &r, json!({})).unwrap();
        assert_eq!(v["cookies"].as_array().unwrap().len(), 0);
        call(&clear_storage(), &r, json!({})).unwrap();
        let v = call(&storage_get(), &r, json!({"key": "k"})).unwrap();
        assert_eq!(v["value"], Value::Null);
    }
}
