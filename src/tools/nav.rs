// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Navigation tools: navigate / back / forward / reload / stop.

use serde_json::json;

use crate::tools::tool::Tool;

pub fn tools() -> Vec<Tool> {
    vec![
        navigate(),
        back(),
        forward(),
        reload(),
        stop(),
        get_history(),
    ]
}

fn navigate() -> Tool {
    Tool::new(
        "navigate",
        "Navigate the active tab (or 'tab') to a URL. Returns the new page url and title.",
        json!({"url": {"type": "string", "description": "Absolute URL to visit", "required": true}}),
        r#"{"url": "https://github.com"}"#,
        |ctx| {
            let url = ctx.param_str("url")?;
            let tab = ctx.tab(ctx.tab_param()?)?;
            ctx.runtime.engine().navigate(tab, &url)?;
            let page_url = ctx.runtime.engine().page_url(tab)?;
            let title = ctx.runtime.engine().page_title(tab)?;
            Ok(json!({"ok": true, "url": page_url, "title": title}))
        },
    )
}

fn back() -> Tool {
    Tool::new(
        "back",
        "Go back in the active tab's history. No-op at the first entry.",
        json!({}),
        r#"{}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            ctx.runtime.engine().back(tab)?;
            let snap = ctx.runtime.engine().snapshot(tab)?;
            Ok(json!({"ok": true, "url": snap.url}))
        },
    )
}

fn forward() -> Tool {
    Tool::new(
        "forward",
        "Go forward in the active tab's history. No-op at the last entry.",
        json!({}),
        r#"{}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            ctx.runtime.engine().forward(tab)?;
            let snap = ctx.runtime.engine().snapshot(tab)?;
            Ok(json!({"ok": true, "url": snap.url}))
        },
    )
}

fn reload() -> Tool {
    Tool::new(
        "reload",
        "Reload the active tab.",
        json!({}),
        r#"{}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            ctx.runtime.engine().reload(tab)?;
            Ok(json!({"ok": true}))
        },
    )
}

fn stop() -> Tool {
    Tool::new(
        "stop",
        "Stop loading the active tab.",
        json!({}),
        r#"{}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            ctx.runtime.engine().stop(tab)?;
            Ok(json!({"ok": true}))
        },
    )
}

fn get_history() -> Tool {
    Tool::new(
        "get_history",
        "Return the active tab's navigation history (url/title/transition).",
        json!({}),
        r#"{}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let history = ctx.runtime.engine().get_history(tab)?;
            Ok(json!({"history": history, "count": history.len()}))
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::tools::tool::ToolContext;

    use crate::bridge::Runtime;
    use crate::config::Config;
    use crate::engine::{TabId, TabOptions};

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
    fn navigate_then_back() {
        let r = runtime();
        let v = navigate()
            .run(&ToolContext {
                runtime: &r,
                params: json!({"url": "https://example.com/login"}),
            })
            .unwrap();
        assert_eq!(v["url"], "https://example.com/login");
        assert_eq!(v["title"], "Login");
        let _ = back()
            .run(&ToolContext {
                runtime: &r,
                params: json!({}),
            })
            .unwrap();
        let v = forward()
            .run(&ToolContext {
                runtime: &r,
                params: json!({}),
            })
            .unwrap();
        assert_eq!(v["url"], "https://example.com/login");
        let _ = reload()
            .run(&ToolContext {
                runtime: &r,
                params: json!({}),
            })
            .unwrap();
        let _ = stop()
            .run(&ToolContext {
                runtime: &r,
                params: json!({}),
            })
            .unwrap();
    }

    #[test]
    fn navigate_requires_url() {
        let r = runtime();
        let res = navigate().run(&ToolContext {
            runtime: &r,
            params: json!({}),
        });
        assert!(res.is_err());
        let _ = TabId(0);
    }
}
