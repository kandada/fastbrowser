// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Navigation tools: navigate / back / forward / reload / stop.

use serde_json::json;

use crate::engine::Capability;
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

/// Normalize an http(s) URL for equivalence checks: lowercase scheme/host,
/// drop a default port and fragment, and treat an empty path as `/` while
/// ignoring a single trailing slash. Best-effort (no full URL parser).
fn norm_url(u: &str) -> String {
    let u = u.trim();
    let (scheme, rest) = match u.split_once("://") {
        Some((s, r)) => (s.to_ascii_lowercase(), r),
        None => (String::new(), u),
    };
    let (authority_raw, path_raw) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, ""),
    };
    let mut authority = authority_raw.to_ascii_lowercase();
    // Strip a default port so host:443/https and host:80/http compare equal.
    if scheme == "https" && authority.ends_with(":443") {
        authority.truncate(authority.len() - 4);
    } else if scheme == "http" && authority.ends_with(":80") {
        authority.truncate(authority.len() - 3);
    }
    let path = path_raw.split('#').next().unwrap_or("");
    let path = path.strip_suffix('/').unwrap_or(path);
    if scheme.is_empty() {
        format!("{authority}{path}")
    } else {
        format!("{scheme}://{authority}{path}")
    }
}

fn navigate() -> Tool {
    Tool::new(
        "navigate",
        "Navigate the active tab (or 'tab') to a URL. By default waits for the page to load ('wait_until', Playwright-style); pass \"none\" to return immediately. Returns the new page url and title.",
        json!({
            "url": {"type": "string", "description": "Absolute URL to visit", "required": true},
            "wait_until": {"type": "string", "enum": ["none", "domcontentloaded", "load", "networkidle"], "default": "load", "description": "Load state to wait for before returning (default 'load')."}
        }),
        r#"{"url": "https://github.com"}"#,
        |ctx| {
            let url = ctx.param_str("url")?;
            if url.trim().is_empty() {
                return Err(crate::engine::EngineError::invalid(
                    "navigate: 'url' is required",
                ));
            }
            // Reject an unsupported scheme explicitly instead of silently
            // no-op'ing (a scheme-less value is left to the host).
            let scheme: String = url
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
                .collect();
            let has_scheme = url.as_bytes().get(scheme.len()) == Some(&b':');
            const OK_SCHEMES: &[&str] =
                &["http", "https", "file", "data", "about", "blob", "ftp"];
            if has_scheme && !OK_SCHEMES.contains(&scheme.to_ascii_lowercase().as_str()) {
                return Err(crate::engine::EngineError::invalid(format!(
                    "navigate: unsupported URL scheme '{scheme}:' in {url:?} (use http(s)://, file://, data: …)"
                )));
            }
            // URL before navigating — used to detect a silently-ignored
            // navigation (unreachable host without an error page).
            let pre_url = ctx
                .runtime
                .engine()
                .active_tab()
                .and_then(|t| ctx.runtime.engine().page_url(t).ok())
                .unwrap_or_default();

            // If there is no active tab yet, create it *with* the URL in one
            // serialized step (`ensure_tab_url`). This must go through the
            // runtime's first-tab lock: a concurrently-issued tool (e.g. a
            // batched `get_page_title`) would otherwise create its own blank
            // tab and leave the active tab pointing at the wrong page.
            let tab = match ctx.tab_param()? {
                Some(t) => {
                    ctx.runtime.engine().navigate(t, &url)?;
                    t
                }
                None => match ctx.runtime.ensure_tab_url(&url)? {
                    // The tab was created by us and is already loading the URL.
                    (t, true) => t,
                    // A tab already existed / was created concurrently (blank or
                    // at another URL) → navigate it now.
                    (t, false) => {
                        ctx.runtime.engine().navigate(t, &url)?;
                        t
                    }
                },
            };
            let wait_until = ctx
                .param_opt::<String>("wait_until")?
                .unwrap_or_else(|| "load".into());
            let mut out = json!({"ok": true});
            if wait_until != "none" {
                // 对齐 Playwright `goto` 的自动等待；失败（超时）不致命。
                if let Ok(ready) =
                    crate::tools::wait::wait_load_state(ctx, tab, &wait_until, 10_000)
                {
                    out["ready"] = json!(ready);
                }
                out["waited"] = json!(wait_until);
            }
            out["url"] = json!(ctx.runtime.engine().page_url(tab)?);
            out["title"] = json!(ctx.runtime.engine().page_title(tab)?);
            // Detect a navigation that landed on the browser's error page
            // (unreachable host, DNS failure, blocked) instead of reporting a
            // misleading `ok:true`. `page_url` reads the live `location.href`,
            // so a failed load shows `chrome-error://` / `about:neterror`.
            let live_url = out["url"].as_str().unwrap_or("").to_string();
            let live_title = out["title"].as_str().unwrap_or("").to_string();
            let is_error = live_url.starts_with("chrome-error://")
                || live_url.starts_with("about:neterror")
                || live_url.contains("neterror")
                // Chinese Chrome error titles ("webpage cannot be opened" /
                // "this site cannot be reached"). Kept as \u escapes so the
                // source stays ASCII (English-only code check) while matching
                // the zh-locale error page.
                || live_title.contains("\u{7f51}\u{9875}\u{65e0}\u{6cd5}\u{6253}\u{5f00}")
                || live_title.contains("\u{65e0}\u{6cd5}\u{8bbf}\u{95ee}\u{6b64}\u{7f51}\u{7ad9}")
                || live_title.contains("This site can")
                || live_title.contains("can't be reached")
                || live_title.contains("This page isn");
            // Silently-ignored navigation: still on the previous page after
            // waiting, with no error page shown. Compare *normalized* URLs so a
            // benign difference (trailing slash, case, fragment, default port)
            // — e.g. navigating to `https://example.com` while already on
            // `https://example.com/` — is not misreported as a failed load.
            let requested_is_http = url.starts_with("http://") || url.starts_with("https://");
            let stale = !is_error
                && requested_is_http
                && !pre_url.is_empty()
                && norm_url(&live_url) == norm_url(&pre_url)
                && norm_url(&url) != norm_url(&live_url);
            if is_error || stale {
                out["ok"] = json!(false);
                out["error"] = json!(format!(
                    "navigation failed:{} (url={live_url}, title={live_title})",
                    if stale {
                        " still on the previous page (host unreachable or blocked?)"
                    } else {
                        " the page could not be loaded"
                    }
                ));
            }
            Ok(out)
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
            let _ = crate::tools::wait::wait_load_state(ctx, tab, "domcontentloaded", 5_000);
            Ok(json!({
                "ok": true,
                "url": ctx.runtime.engine().page_url(tab)?,
                "title": ctx.runtime.engine().page_title(tab)?,
            }))
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
            let _ = crate::tools::wait::wait_load_state(ctx, tab, "domcontentloaded", 5_000);
            Ok(json!({
                "ok": true,
                "url": ctx.runtime.engine().page_url(tab)?,
                "title": ctx.runtime.engine().page_title(tab)?,
            }))
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
            let _ = crate::tools::wait::wait_load_state(ctx, tab, "domcontentloaded", 5_000);
            Ok(json!({
                "ok": true,
                "url": ctx.runtime.engine().page_url(tab)?,
                "title": ctx.runtime.engine().page_title(tab)?,
            }))
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
    // webview 引擎未实现 get_history → 从清单隐藏、调用时明确报错。
    .requires(&[Capability::History])
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
    fn navigate_rejects_unsupported_scheme() {
        let r = runtime();
        let err = navigate().run(&ToolContext {
            runtime: &r,
            params: json!({"url": "javascript:alert(1)"}),
        });
        assert!(err.is_err(), "javascript: must be rejected");
        // Supported schemes still work.
        assert!(navigate()
            .run(&ToolContext {
                runtime: &r,
                params: json!({"url": "data:text/html,<h1>x</h1>"}),
            })
            .is_ok());
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

    #[test]
    fn norm_url_treats_equivalent_urls_as_same() {
        // trailing slash
        assert_eq!(
            norm_url("https://example.com"),
            norm_url("https://example.com/")
        );
        // trailing slash on a path
        assert_eq!(
            norm_url("https://example.com/a/"),
            norm_url("https://example.com/a")
        );
        // case-insensitive scheme/host
        assert_eq!(
            norm_url("HTTPS://Example.COM/A"),
            norm_url("https://example.com/A")
        );
        // default port
        assert_eq!(
            norm_url("https://example.com:443/x"),
            norm_url("https://example.com/x")
        );
        assert_eq!(norm_url("http://h:80"), norm_url("http://h/"));
        // fragment ignored
        assert_eq!(
            norm_url("https://e.com/p#frag"),
            norm_url("https://e.com/p")
        );
        // genuinely different pages
        assert_ne!(norm_url("https://e.com/a"), norm_url("https://e.com/b"));
    }

    #[test]
    fn back_and_forward_report_live_url_and_title() {
        let r = runtime();
        let _ = navigate()
            .run(&ToolContext {
                runtime: &r,
                params: json!({"url": "https://example.com/a"}),
            })
            .unwrap();
        let _ = navigate()
            .run(&ToolContext {
                runtime: &r,
                params: json!({"url": "https://example.com/b"}),
            })
            .unwrap();
        let b = back()
            .run(&ToolContext {
                runtime: &r,
                params: json!({}),
            })
            .unwrap();
        assert!(b.get("url").is_some(), "back: {b}");
        assert!(b.get("title").is_some(), "back: {b}");
        let f = forward()
            .run(&ToolContext {
                runtime: &r,
                params: json!({}),
            })
            .unwrap();
        assert!(f.get("url").is_some(), "forward: {f}");
    }
}
