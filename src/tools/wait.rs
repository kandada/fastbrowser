// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! 等待/断言域工具：wait_for_element / wait_for_navigation / assert_*。
//!
//! 等待类工具统一走 `engine::wait_until`（"先查当前、再等未来事件"）：
//! 命中立即返回；未命中则阻塞等待事件（`EventNotifier` 唤醒），事件到来即时
//! 复查；无事件时按有界间隔兜底（兼容定时器/动画等不产生事件的场景）。

use std::time::Duration;

use serde_json::{json, Value};

use crate::engine::PageEvent;
use crate::tools::tool::Tool;

pub fn tools() -> Vec<Tool> {
    vec![
        wait_for_element(),
        wait_for_navigation(),
        wait_for_load_state(),
        wait_for_condition(),
        wait_for_text(),
        assert_element_exists(),
        assert_text_contains(),
        assert_url_contains(),
        assert_title(),
    ]
}

fn wait_for_load_state() -> Tool {
    Tool::new(
        "wait_for_load_state",
        "Wait until the page reaches a load state: 'load' | 'domcontentloaded' | 'networkidle'.",
        json!({"state": {"type": "string", "enum": ["load", "domcontentloaded", "networkidle"], "default": "load"}, "timeout_ms": {"type": "number", "default": 10000}}),
        r#"{"state": "networkidle", "timeout_ms": 5000}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let state = ctx
                .param_opt::<String>("state")?
                .unwrap_or_else(|| "load".into());
            let timeout = ctx.param_opt::<u64>("timeout_ms")?.unwrap_or(10000);
            let mut last_ready = String::new();
            crate::engine::wait_until(
                ctx.runtime.engine(),
                tab,
                Duration::from_millis(timeout),
                &format!("wait_for_load_state('{state}')"),
                || {
                    let ready = ctx
                        .eval_opt(tab, "document.readyState||''")
                        .and_then(|v| v.as_str().map(String::from))
                        .unwrap_or_default();
                    last_ready = ready.clone();
                    let idle = if state == "networkidle" {
                        ctx.runtime.engine().drain_events(tab).is_empty()
                    } else {
                        true
                    };
                    let reached = match state.as_str() {
                        "networkidle" => last_ready == "complete" && idle,
                        "domcontentloaded" => {
                            last_ready == "interactive" || last_ready == "complete"
                        }
                        _ => last_ready == "complete",
                    };
                    Ok(reached)
                },
            )?;
            Ok(json!({"state": state, "ready": last_ready}))
        },
    )
}

fn wait_for_condition() -> Tool {
    Tool::new(
        "wait_for_condition",
        "Wait until a JavaScript predicate evaluates to truthy.",
        json!({"script": {"type": "string", "required": true}, "timeout_ms": {"type": "number", "default": 5000}}),
        r#"{"script": "document.querySelectorAll('a').length >= 3"}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let script = ctx.param_str("script")?;
            let timeout = ctx.param_opt::<u64>("timeout_ms")?.unwrap_or(5000);
            crate::engine::wait_until(
                ctx.runtime.engine(),
                tab,
                Duration::from_millis(timeout),
                "wait_for_condition",
                || {
                    let truthy = ctx
                        .eval_opt(tab, &script)
                        .map(|v| match v {
                            Value::Null | Value::Bool(false) => false,
                            Value::Number(n) => n.as_f64() != Some(0.0),
                            Value::String(s) => !s.is_empty(),
                            _ => true,
                        })
                        .unwrap_or(false);
                    Ok(truthy)
                },
            )?;
            Ok(json!({"condition": true}))
        },
    )
}

fn wait_for_text() -> Tool {
    Tool::new(
        "wait_for_text",
        "Wait until the page visible text contains the given substring.",
        json!({"text": {"type": "string", "required": true}, "timeout_ms": {"type": "number", "default": 5000}}),
        r#"{"text": "Welcome", "timeout_ms": 3000}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let needle = ctx.param_str("text")?;
            let timeout = ctx.param_opt::<u64>("timeout_ms")?.unwrap_or(5000);
            crate::engine::wait_until(
                ctx.runtime.engine(),
                tab,
                Duration::from_millis(timeout),
                &format!("wait_for_text('{needle}')"),
                || {
                    let text = ctx.page_text(tab).unwrap_or_default();
                    Ok(text.contains(&needle))
                },
            )?;
            Ok(json!({"found": true}))
        },
    )
}

fn assert_url_contains() -> Tool {
    Tool::new(
        "assert_url_contains",
        "Assert the current URL contains the given substring.",
        json!({"contains": {"type": "string", "required": true}}),
        r#"{"contains": "example.com"}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let needle = ctx.param_str("contains")?;
            let url = ctx.runtime.engine().page_url(tab)?;
            if url.contains(&needle) {
                Ok(json!({"ok": true, "url": url}))
            } else {
                Err(crate::engine::EngineError::new(
                    crate::engine::ErrorKind::Dom,
                    format!("assert_url_contains: '{needle}' not in '{url}'"),
                ))
            }
        },
    )
}

fn assert_title() -> Tool {
    Tool::new(
        "assert_title",
        "Assert the page title contains the given substring.",
        json!({"contains": {"type": "string", "required": true}}),
        r#"{"contains": "Login"}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let needle = ctx.param_str("contains")?;
            let title = ctx.runtime.engine().page_title(tab)?;
            if title.contains(&needle) {
                Ok(json!({"ok": true, "title": title}))
            } else {
                Err(crate::engine::EngineError::new(
                    crate::engine::ErrorKind::Dom,
                    format!("assert_title: '{needle}' not in '{title}'"),
                ))
            }
        },
    )
}

fn wait_for_element() -> Tool {
    Tool::new(
        "wait_for_element",
        "Wait until an element matching 'id' (letter) or 'selector' (css/xpath/text) appears, up to timeout_ms.",
        json!({
            "id": {"type": "string", "required": false},
            "selector": {"type": "string", "required": false},
            "timeout_ms": {"type": "number", "default": 5000}
        }),
        r#"{"selector": "a", "timeout_ms": 3000}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let timeout = ctx.param_opt::<u64>("timeout_ms")?.unwrap_or(5000);
            let id = ctx.param_opt::<String>("id")?;
            let selector = ctx.param_opt::<String>("selector")?;
            // 选择器是否只能用整页快照判断（xpath / mock 引擎无法 JS 求值）
            let mut need_snapshot = false;
            crate::engine::wait_until(
                ctx.runtime.engine(),
                tab,
                Duration::from_millis(timeout),
                "wait_for_element",
                || {
                    if let Some(id) = &id {
                        // 轻量：快照 id 是否已标注 → document.querySelector('[data-fb=..]')
                        let c = id.chars().next().unwrap_or('?');
                        let js = format!("!!(document.querySelector('[data-fb=\"{c}\"]'))");
                        if ctx.eval_opt(tab, &js) == Some(Value::Bool(true)) {
                            return Ok(true);
                        }
                    } else if let Some(sel) = &selector {
                        // 轻量：css/tag 选择器 → querySelector；非法（如 xpath）回退快照
                        let sel_json = serde_json::to_string(sel).unwrap_or_else(|_| "\"\"".into());
                        let js = format!(
                            "(function(){{try{{return !!document.querySelector({sel_json});}}catch(e){{return null;}}}})()"
                        );
                        match ctx.eval_opt(tab, &js) {
                            Some(Value::Bool(true)) => return Ok(true),
                            Some(Value::Bool(false)) => {} // 尚未出现，继续等
                            _ => need_snapshot = true, // null（xpath）或引擎无法求值 → 快照兜底
                        }
                    }
                    if need_snapshot {
                        let snap = ctx.runtime.engine().snapshot(tab)?;
                        let found = if let Some(id) = &id {
                            snap.element_by_id(id.chars().next().unwrap_or('?')).is_some()
                        } else if let Some(sel) = &selector {
                            snap.interactive
                                .iter()
                                .any(|e| e.tag == *sel || e.refs.iter().any(|r| r.value == *sel))
                        } else {
                            false
                        };
                        if found {
                            return Ok(true);
                        }
                    }
                    Ok(false)
                },
            )?;
            Ok(json!({"found": true}))
        },
    )
}

fn wait_for_navigation() -> Tool {
    Tool::new(
        "wait_for_navigation",
        "Wait until the tab is done navigating (already-loaded pages return immediately; \
         otherwise wait for a NavigationCompleted event), up to timeout_ms.",
        json!({"timeout_ms": {"type": "number", "default": 10000}}),
        r#"{"timeout_ms": 5000}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let timeout = ctx.param_opt::<u64>("timeout_ms")?.unwrap_or(10000);
            crate::engine::wait_until(
                ctx.runtime.engine(),
                tab,
                Duration::from_millis(timeout),
                "wait_for_navigation",
                || {
                    // 页面已加载完成 → 视为“导航已完成”，立即返回。否则在
                    // `open`（现已同步等到真实地址）之后，页面其实早已就绪，
                    // 只等“未来的事件”会白等到超时（默认 10s）。
                    let ready = ctx
                        .eval_opt(tab, "document.readyState||''")
                        .and_then(|v| v.as_str().map(String::from))
                        .unwrap_or_default();
                    if ready == "complete" {
                        return Ok(true);
                    }
                    let evs = ctx.runtime.engine().drain_events(tab);
                    Ok(evs
                        .iter()
                        .any(|e| matches!(e, PageEvent::NavigationCompleted { .. })))
                },
            )?;
            Ok(json!({"navigated": true}))
        },
    )
}

fn assert_element_exists() -> Tool {
    Tool::new(
        "assert_element_exists",
        "Assert an element exists (by 'id' or 'selector'). Returns {exists: true} or an error.",
        json!({
            "id": {"type": "string", "required": false},
            "selector": {"type": "string", "required": false}
        }),
        r#"{"selector": "button"}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let snap = ctx.runtime.engine().snapshot(tab)?;
            let exists = match ctx.param_opt::<String>("id")? {
                Some(id) => snap
                    .element_by_id(id.chars().next().unwrap_or('?'))
                    .is_some(),
                None => {
                    let sel = ctx.param_str("selector")?;
                    snap.interactive
                        .iter()
                        .any(|e| e.tag == sel || e.refs.iter().any(|r| r.value == sel))
                }
            };
            if exists {
                Ok(json!({"exists": true}))
            } else {
                Err(crate::engine::EngineError::new(
                    crate::engine::ErrorKind::Dom,
                    "assert_element_exists: element not found",
                ))
            }
        },
    )
}

fn assert_text_contains() -> Tool {
    Tool::new(
        "assert_text_contains",
        "Assert the page visible text contains the given substring. Returns {contains: true} or an error.",
        json!({"text": {"type": "string", "required": true}}),
        r#"{"text": "Login"}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let needle = ctx.param_str("text")?;
            let text = ctx.runtime.engine().get_page_text(tab)?;
            if text.contains(&needle) {
                Ok(json!({"contains": true}))
            } else {
                Err(crate::engine::EngineError::new(
                    crate::engine::ErrorKind::Dom,
                    format!("assert_text_contains: '{needle}' not found"),
                ))
            }
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
    fn wait_and_assert() {
        let r = runtime();
        let v = call(
            &wait_for_element(),
            &r,
            json!({"selector": "button", "timeout_ms": 100}),
        )
        .unwrap();
        assert_eq!(v["found"], true);
        let v = call(&assert_element_exists(), &r, json!({"selector": "a"})).unwrap();
        assert_eq!(v["exists"], true);
        let v = call(&assert_text_contains(), &r, json!({"text": "Welcome"})).unwrap();
        assert_eq!(v["contains"], true);
    }

    #[test]
    fn navigation_wait() {
        let r = runtime();
        let t = r.engine().active_tab().unwrap();
        r.engine().navigate(t, "https://example.com/login").unwrap();
        let v = call(&wait_for_navigation(), &r, json!({"timeout_ms": 100})).unwrap();
        assert_eq!(v["navigated"], true);
    }

    /// 回归：页面已加载完成时 `wait_for_navigation` 必须**立即**返回，不能白等满超时。
    /// （`open` 现在会同步等到真实地址；旧实现只等“未来事件”，会白等默认 10s，
    /// 导致 Agent 明明页面已就绪却一直等返回。）
    #[test]
    fn wait_for_navigation_returns_immediately_when_already_loaded() {
        use std::time::{Duration, Instant};
        let r = runtime();
        // mock 页面默认 readyState=complete。
        let start = Instant::now();
        let v = call(&wait_for_navigation(), &r, json!({"timeout_ms": 10000})).unwrap();
        assert_eq!(v["navigated"], true);
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "already-loaded page must return immediately, not wait the timeout (elapsed {:?})",
            start.elapsed()
        );
    }

    #[test]
    fn wait_timeout_errors() {
        let r = runtime();
        let res = call(
            &wait_for_element(),
            &r,
            json!({"selector": "nonexistent", "timeout_ms": 50}),
        );
        assert!(res.is_err());
    }

    #[test]
    fn load_state_and_condition() {
        let r = runtime();
        let v = call(
            &wait_for_load_state(),
            &r,
            json!({"state": "load", "timeout_ms": 200}),
        )
        .unwrap();
        assert_eq!(v["ready"], "complete");
        let v = call(
            &wait_for_condition(),
            &r,
            json!({"script": "document.querySelectorAll('a').length >= 1", "timeout_ms": 200}),
        )
        .unwrap();
        assert_eq!(v["condition"], true);
        let v = call(
            &wait_for_text(),
            &r,
            json!({"text": "Welcome", "timeout_ms": 200}),
        )
        .unwrap();
        assert_eq!(v["found"], true);
    }

    #[test]
    fn assert_url_and_title() {
        let r = runtime();
        let v = call(
            &assert_url_contains(),
            &r,
            json!({"contains": "example.com"}),
        )
        .unwrap();
        assert_eq!(v["ok"], true);
        let v = call(&assert_title(), &r, json!({"contains": "Example"})).unwrap();
        assert_eq!(v["ok"], true);
        assert!(call(&assert_url_contains(), &r, json!({"contains": "nope"})).is_err());
    }

    /// 端到端：事件驱动等待——后台线程触发 DOM 变更（push → notify），
    /// `wait_for_text` 应被事件唤醒并立即返回，而非空转到超时。
    #[test]
    fn wait_for_text_wakes_on_background_event() {
        use std::sync::Arc;
        use std::thread;
        use std::time::{Duration, Instant};

        use crate::engine::ElementRef;

        let r = Arc::new(runtime());
        let tab = r.engine().active_tab().unwrap();
        let r2 = r.clone();
        let h = thread::spawn(move || {
            thread::sleep(Duration::from_millis(30));
            r2.engine()
                .set_element_value(tab, &ElementRef::snapshot('e'), "target")
                .unwrap();
        });
        let start = Instant::now();
        let v = call(
            &wait_for_text(),
            r.as_ref(),
            json!({"text": "target", "timeout_ms": 5000}),
        )
        .unwrap();
        h.join().unwrap();
        assert_eq!(v["found"], true);
        // 事件驱动：应远早于 5s 超时返回（正确性兜底 + 时序 sanity check）
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "wait_for_text should wake on event, not spin until timeout (elapsed {:?})",
            start.elapsed()
        );
    }
}
