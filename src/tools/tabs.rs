// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Multi-tab tools: new_tab / close_tab / switch_tab / list_tabs.

use serde_json::json;

use crate::engine::TabId;
use crate::tools::tool::Tool;

pub fn tools() -> Vec<Tool> {
    vec![
        new_tab(),
        new_window(),
        close_tab(),
        switch_tab(),
        list_tabs(),
        get_active_tab(),
        get_tab(),
        duplicate_tab(),
        close_other_tabs(),
    ]
}

/// The tab the agent is currently bound to (the user's active page). This is the
/// "where am I" primitive: the agent can call it any time to learn its current
/// tab id/url/title without any per-turn system-prompt injection.
fn get_active_tab() -> Tool {
    Tool::new(
        "get_active_tab",
        "Return the tab the agent is currently bound to (the page you are operating on): \
         id, url, title and CDP target_id. Use list_tabs to see every open tab.",
        json!({}),
        r#"{}"#,
        |ctx| {
            let tab = ctx.active_tab()?;
            let info = ctx
                .runtime
                .engine()
                .list_tabs()
                .into_iter()
                .find(|t| t.id == tab);
            Ok(match info {
                Some(t) => json!({
                    "tab": t.id.as_u32(),
                    "url": t.url,
                    "title": t.title,
                    "target_id": t.target_id,
                }),
                None => json!({ "tab": tab.as_u32() }),
            })
        },
    )
}

fn get_tab() -> Tool {
    Tool::new(
        "get_tab",
        "Find a tab whose url or title contains the given substring. Returns tab info.",
        json!({"url_contains": {"type": "string", "required": false}, "title_contains": {"type": "string", "required": false}}),
        r#"{"title_contains": "Search"}"#,
        |ctx| {
            let url_needle = ctx.param_opt::<String>("url_contains")?;
            let title_needle = ctx.param_opt::<String>("title_contains")?;
            let tabs = ctx.runtime.engine().list_tabs();
            let found = tabs.into_iter().find(|t| {
                let u = url_needle
                    .as_ref()
                    .map(|n| t.url.contains(n))
                    .unwrap_or(true);
                let ti = title_needle
                    .as_ref()
                    .map(|n| t.title.contains(n))
                    .unwrap_or(true);
                u && ti
            });
            match found {
                Some(t) => Ok(json!({"tab": t})),
                None => Err(crate::engine::EngineError::new(
                    crate::engine::ErrorKind::TabNotFound,
                    "no tab matches the given criteria",
                )),
            }
        },
    )
}

fn duplicate_tab() -> Tool {
    Tool::new(
        "duplicate_tab",
        "Duplicate the active tab (same URL in a new tab).",
        json!({}),
        r#"{}"#,
        |ctx| {
            let tab = ctx.active_tab()?;
            let url = ctx
                .runtime
                .engine()
                .snapshot(tab)
                .map(|s| s.url)
                .unwrap_or_default();
            let new_tab = ctx.runtime.create_tab_in_profile(&url, true)?;
            Ok(json!({"tab": new_tab.as_u32(), "url": url}))
        },
    )
}

fn close_other_tabs() -> Tool {
    Tool::new(
        "close_other_tabs",
        "Close every tab except the active one.",
        json!({}),
        r#"{}"#,
        |ctx| {
            let active = ctx.runtime.engine().active_tab();
            let tabs = ctx.runtime.engine().list_tabs();
            let mut closed = 0;
            for t in tabs {
                if Some(t.id) != active {
                    ctx.runtime.engine().close_tab(t.id)?;
                    closed += 1;
                }
            }
            Ok(json!({"closed": closed}))
        },
    )
}

fn new_tab() -> Tool {
    Tool::new(
        "new_tab",
        "Open a new tab with 'url' and activate it. Returns the new tab id.",
        json!({"url": {"type": "string", "required": true}}),
        r#"{"url": "https://github.com"}"#,
        |ctx| {
            let url = ctx.param_str("url")?;
            let tab = ctx.runtime.open(&url)?;
            Ok(json!({"tab": tab.as_u32(), "url": url}))
        },
    )
}

fn new_window() -> Tool {
    Tool::new(
        "new_window",
        "Open a new browser window with 'url' and activate it. Returns the new tab id.",
        json!({"url": {"type": "string", "required": true}}),
        r#"{"url": "https://github.com"}"#,
        |ctx| {
            let url = ctx.param_str("url")?;
            let opts = crate::engine::TabOptions {
                active: true,
                ..Default::default()
            };
            let tab = ctx.runtime.engine().new_window(&url, &opts)?;
            Ok(json!({"tab": tab.as_u32(), "url": url}))
        },
    )
}

fn close_tab() -> Tool {
    Tool::new(
        "close_tab",
        "Close a tab (defaults to the active tab).",
        json!({"tab": {"type": "integer", "required": false}}),
        r#"{"tab": 2}"#,
        |ctx| {
            let tab = match ctx.tab_param()? {
                Some(t) => t,
                None => ctx.active_tab()?,
            };
            ctx.runtime.engine().close_tab(tab)?;
            Ok(json!({"closed": tab.as_u32()}))
        },
    )
}

fn switch_tab() -> Tool {
    Tool::new(
        "switch_tab",
        "Switch the active tab to 'tab'.",
        json!({"tab": {"type": "integer", "required": true}}),
        r#"{"tab": 3}"#,
        |ctx| {
            let tab = TabId(ctx.param::<u32>("tab")?);
            ctx.runtime.engine().switch_tab(tab)?;
            Ok(json!({"active": tab.as_u32()}))
        },
    )
}

fn list_tabs() -> Tool {
    Tool::new(
        "list_tabs",
        "List all open tabs with id, url and title.",
        json!({}),
        r#"{}"#,
        |ctx| {
            let tabs = ctx.runtime.engine().list_tabs();
            Ok(json!({"tabs": tabs}))
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
    fn tab_lifecycle() {
        let r = runtime();
        let v = call(&new_tab(), &r, json!({"url": "https://example.com/login"})).unwrap();
        let t2 = v["tab"].as_u64().unwrap() as u32;
        let tabs = call(&list_tabs(), &r, json!({})).unwrap();
        assert_eq!(tabs["tabs"].as_array().unwrap().len(), 2);
        let _ = call(&switch_tab(), &r, json!({"tab": t2}));
        assert_eq!(r.engine().active_tab().unwrap().as_u32(), t2);
        let _ = call(&close_tab(), &r, json!({"tab": t2}));
        let tabs = call(&list_tabs(), &r, json!({})).unwrap();
        assert_eq!(tabs["tabs"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn get_active_tab_reports_current() {
        let r = runtime();
        let active = r.engine().active_tab().unwrap();
        let v = call(&get_active_tab(), &r, json!({})).unwrap();
        assert_eq!(v["tab"].as_u64().unwrap(), active.as_u32() as u64);
        assert!(v["url"].as_str().unwrap().contains("example.com"));
    }

    #[test]
    fn get_active_tab_follows_switch_tab() {
        let r = runtime();
        let v = call(&new_tab(), &r, json!({"url": "https://second.example/"})).unwrap();
        let t2 = v["tab"].as_u64().unwrap() as u32;
        let _ = call(&switch_tab(), &r, json!({"tab": t2}));
        let active = call(&get_active_tab(), &r, json!({})).unwrap();
        assert_eq!(active["tab"].as_u64().unwrap(), t2 as u64);
        assert!(
            active["url"].as_str().unwrap().contains("second.example"),
            "unexpected: {active}"
        );
    }

    #[test]
    fn find_duplicate_close_others() {
        let r = runtime();
        r.engine()
            .create_tab("https://example.com", &TabOptions::default())
            .unwrap();
        r.engine()
            .create_tab("https://example.com/search", &TabOptions::default())
            .unwrap();
        // get_tab by title
        let v = call(&get_tab(), &r, json!({"title_contains": "Search"})).unwrap();
        assert!(v["tab"]["url"].as_str().unwrap().contains("search"));
        // duplicate active
        let active = r.engine().active_tab().unwrap();
        let dup = call(&duplicate_tab(), &r, json!({})).unwrap();
        assert_eq!(
            dup["tab"].as_u64().unwrap() as u32,
            r.engine().active_tab().unwrap().as_u32()
        );
        // close others keeps 1
        let _active = active;
        let closed = call(&close_other_tabs(), &r, json!({})).unwrap();
        assert!(closed["closed"].as_u64().unwrap() >= 1);
        assert_eq!(r.engine().list_tabs().len(), 1);
    }
}
