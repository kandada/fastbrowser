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

/// 取该 tab 当前记录的目标 URL（`navigate`/`open` 会把它设为目标地址）。
/// 用于判断页面是否“已经真正导航到目标”：宿主 WebView 的 `load()` 是异步的，
/// 若只看 `document.readyState`，旧页早已 `complete` 会秒回、读到上一页内容。
fn tab_target_url(
    engine: &dyn crate::engine::BrowserEngine,
    tab: crate::engine::TabId,
) -> Option<String> {
    engine
        .list_tabs()
        .into_iter()
        .find(|t| t.id == tab)
        .map(|t| t.url)
        .filter(|u| !u.is_empty())
}

/// 宽松比较两个 URL 是否指向同一页面：忽略 `#fragment`、百分号编码差异与结尾 `/`。
/// 任一侧为空则视为不可判定（返回 true，即不阻塞，保持向后兼容）。
fn same_url(a: &str, b: &str) -> bool {
    if a.is_empty() || b.is_empty() {
        return true;
    }
    fn norm(s: &str) -> String {
        let s = s.split('#').next().unwrap_or(s);
        let bytes = s.as_bytes();
        let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'%' && i + 2 < bytes.len() {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
                if let Some(v) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                    out.push(v);
                    i += 3;
                    continue;
                }
            }
            out.push(bytes[i]);
            i += 1;
        }
        String::from_utf8_lossy(&out)
            .trim_end_matches('/')
            .to_string()
    }
    norm(a) == norm(b)
}

/// 读取页面当前地址；引擎不支持时返回空串（空串不参与比较）。
fn current_href(ctx: &crate::tools::tool::ToolContext, tab: crate::engine::TabId) -> String {
    ctx.eval_opt(tab, "location.href||''")
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_default()
}

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
            let ready = match wait_load_state(ctx, tab, &state, timeout) {
                Ok(r) => r,
                Err(mut e) => {
                    if state == "networkidle" {
                        e.message.push_str(
                            " — 'networkidle' never settles on pages with long-polling/streams/ads; use 'load' or 'domcontentloaded'",
                        );
                    }
                    return Err(e);
                }
            };
            Ok(json!({"state": state, "ready": ready}))
        },
    )
}

/// 等待页面到达 `state`（`load`|`domcontentloaded`|`networkidle`），返回最终
/// `document.readyState`。被 `wait_for_load_state` 与 `navigate(wait_until)` 共用。
///
/// 先确认已导航到目标 URL，否则旧页 `readyState` 已是 complete 会秒回并让后续
/// 读取到上一页内容。
/// Whether the wait's URL gate is satisfied.
///
/// The gate exists so that right after `navigate(wait_until:"none")` we do not
/// accept the *old* page's `readyState == "complete"`. It must, however, accept
/// server redirects / SPA canonicalisation — otherwise `location.href` never
/// equals the requested URL and the wait times out forever (a real regression).
///
/// - No known target → gate open.
/// - Already at the target at wait start (`expect_nav == false`) → only the
///   target counts.
/// - A navigation is pending (`start_href` ≠ target) → accept the target **or**
///   any URL the tab has moved to (a redirect).
fn url_gate_ok(target: Option<&str>, start_href: &str, href: &str) -> bool {
    let Some(t) = target else {
        return true;
    };
    if same_url(href, t) {
        return true;
    }
    let expect_nav = !start_href.is_empty() && !same_url(start_href, t);
    expect_nav && !same_url(href, start_href)
}

pub fn wait_load_state(
    ctx: &crate::tools::tool::ToolContext,
    tab: crate::engine::TabId,
    state: &str,
    timeout_ms: u64,
) -> crate::engine::Result<String> {
    let target = tab_target_url(ctx.runtime.engine(), tab);
    // Where the tab is at the start of the wait. If it is not yet at the target
    // a navigation is pending, and redirects are allowed (see `url_gate_ok`).
    let start_href = current_href(ctx, tab);
    let mut last_ready = String::new();
    // The mobile webview host emits no navigation events and `back`/`forward`/
    // client-side navigations leave the recorded target URL stale, so the URL
    // gate can never pass. Once we are on a *real, fully-loaded* page that has
    // not changed since the wait began, accept after a short grace instead of
    // hanging until the timeout.
    let mut settled_since: Option<std::time::Instant> = None;
    let real_page = |u: &str| !u.is_empty() && !u.eq_ignore_ascii_case("about:blank");
    crate::engine::wait_until(
        ctx.runtime.engine(),
        tab,
        Duration::from_millis(timeout_ms),
        &format!("wait_for_load_state('{state}')"),
        || {
            let href = current_href(ctx, tab);
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
            let reached = match state {
                "networkidle" => last_ready == "complete" && idle,
                "domcontentloaded" => last_ready == "interactive" || last_ready == "complete",
                _ => last_ready == "complete",
            };
            if !reached {
                settled_since = None;
                return Ok(false);
            }
            match target.as_deref() {
                None => Ok(true),
                Some(t) if url_gate_ok(Some(t), &start_href, &href) => Ok(true),
                Some(_) => {
                    // Gate says "not the target". If we are on a real page that is
                    // exactly where the wait started (the recorded target is just
                    // stale — e.g. after `back`), accept once it has been stable
                    // for a short grace.
                    if real_page(&href) && same_url(&href, &start_href) {
                        let since = *settled_since.get_or_insert_with(std::time::Instant::now);
                        if since.elapsed() >= Duration::from_millis(1200) {
                            return Ok(true);
                        }
                    } else {
                        settled_since = None;
                    }
                    Ok(false)
                }
            }
        },
    )?;
    Ok(last_ready)
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
        "Wait until an element matching 'id' (letter) or 'selector' (css / xpath= / text= / role= / data-testid= / Playwright selectors) appears, up to timeout_ms.",
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
                        // Shared selector engine → Playwright selectors work here too.
                        let js = crate::engine::inject::element_exists_js(sel);
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
            let target = tab_target_url(ctx.runtime.engine(), tab);
            let start_href = current_href(ctx, tab);
            let mut settled_since: Option<std::time::Instant> = None;
            let real_page = |u: &str| !u.is_empty() && !u.eq_ignore_ascii_case("about:blank");
            crate::engine::wait_until(
                ctx.runtime.engine(),
                tab,
                Duration::from_millis(timeout),
                "wait_for_navigation",
                || {
                    let href = current_href(ctx, tab);
                    let at_target = match target.as_deref() {
                        None => true,
                        Some(t) => same_url(&href, t),
                    };
                    // 已到目标地址：readyState 完成。
                    let ready = ctx
                        .eval_opt(tab, "document.readyState||''")
                        .and_then(|v| v.as_str().map(String::from))
                        .unwrap_or_default();
                    if at_target && ready == "complete" {
                        return Ok(true);
                    }
                    let evs = ctx.runtime.engine().drain_events(tab);
                    if evs
                        .iter()
                        .any(|e| matches!(e, PageEvent::NavigationCompleted { .. }))
                    {
                        return Ok(true);
                    }
                    // The recorded target is stale (back/forward/client-side nav);
                    // accept a real, fully-loaded page that has not changed since
                    // the wait began, after a short grace.
                    if ready == "complete" && real_page(&href) && same_url(&href, &start_href) {
                        let since = *settled_since.get_or_insert_with(std::time::Instant::now);
                        if since.elapsed() >= Duration::from_millis(1200) {
                            return Ok(true);
                        }
                    } else {
                        settled_since = None;
                    }
                    Ok(false)
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
                    let snap_hit = snap
                        .interactive
                        .iter()
                        .any(|e| e.tag == sel || e.refs.iter().any(|r| r.value == sel));
                    // Live DOM fallback: supports CSS/text=/role=/xpath= that are
                    // not part of the cached interactive snapshot.
                    snap_hit || ctx.element_snapshot(tab).is_ok()
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
        "Assert the page visible text — or one element's text (by 'selector'/'id'/'ref') — contains the given substring. Returns {contains: true} or an error.",
        json!({
            "text": {"type": "string", "required": true},
            "selector": {"type": "string", "required": false},
            "id": {"type": "string", "required": false},
            "ref": {"type": "string", "required": false},
            "element": {"type": "string", "required": false}
        }),
        r#"{"text": "Login"}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let needle = ctx.param_str("text")?;
            let has_target = ["selector", "id", "ref", "element"]
                .iter()
                .any(|k| ctx.params.get(*k).is_some());
            let text = if has_target {
                ctx.element_snapshot(tab)?.text.unwrap_or_default()
            } else {
                ctx.runtime.engine().get_page_text(tab)?
            };
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

    /// 回归：`navigate` 后若页面仍停在上一页，`wait_for_*` 必须能区分出
    /// “当前地址 ≠ 目标地址”，否则会秒回并读到旧内容。
    #[test]
    fn same_url_distinguishes_stale_page_from_target() {
        assert!(same_url("https://a.com/x", "https://a.com/x"));
        assert!(same_url("https://a.com/x#frag", "https://a.com/x"));
        assert!(same_url("https://a.com/x/", "https://a.com/x"));
        // 百分号编码 vs 原始 UTF-8
        assert!(same_url(
            "https://zh.wikipedia.org/wiki/%E5%85%88%E7%88%B6%E9%81%97%E4%BC%A0",
            "https://zh.wikipedia.org/wiki/\u{5148}\u{7236}\u{9057}\u{4f20}"
        ));
        // 不同页面必须区分（会话里“导航到 A 却读到 B”的根因）
        assert!(!same_url(
            "https://baike.baidu.com/item/a",
            "https://zhuanlan.zhihu.com/p/1"
        ));
        // 空串不可判定 → 不阻塞（引擎不支持读取 href 时保持向后兼容）
        assert!(same_url("", "https://a.com"));
        assert!(same_url("https://a.com", ""));
    }

    #[test]
    fn url_gate_accepts_redirects_but_not_the_old_page() {
        // No known target → gate open.
        assert!(url_gate_ok(None, "", "https://a.com"));
        // Already at the target at wait start → target (or its fragment form) OK.
        let t = Some("https://new.example/x");
        assert!(url_gate_ok(
            t,
            "https://new.example/x",
            "https://new.example/x"
        ));
        assert!(url_gate_ok(
            t,
            "https://new.example/x",
            "https://new.example/x#frag"
        ));
        // Pending navigation: the OLD page (== start) must NOT satisfy the gate.
        let start = "https://old.example/";
        assert!(
            !url_gate_ok(t, start, start),
            "old page must not pass the gate"
        );
        assert!(!url_gate_ok(t, start, "https://old.example/#x"));
        // Reaching the target passes.
        assert!(url_gate_ok(t, start, "https://new.example/x"));
        // A server redirect / SPA canonicalisation passes (previously timed out).
        assert!(url_gate_ok(t, start, "https://new.example/x?redirected=1"));
        assert!(url_gate_ok(t, start, "https://cdn.other/landing"));
    }
}
