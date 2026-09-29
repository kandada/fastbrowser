// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Content-extraction tools: extract_text/html/links/images/table/json + search/find_elements.

use serde_json::{json, Value};

use crate::tools::tool::{capped_array, capped_text, Tool};

pub fn tools() -> Vec<Tool> {
    vec![
        extract_text(),
        extract_html(),
        extract_links(),
        extract_images(),
        extract_table(),
        extract_json(),
        search(),
        find_elements(),
    ]
}

fn extract_text() -> Tool {
    Tool::new(
        "extract_text",
        "Extract visible text of the page, or of one element when 'selector'/'id'/'ref'/'element' is given. Optional 'max_chars' caps the result.",
        json!({
            "selector": {"type": "string", "required": false},
            "id": {"type": "string", "required": false},
            "ref": {"type": "string", "required": false},
            "element": {"type": "string", "required": false},
            "max_chars": {"type": "integer", "required": false}
        }),
        r#"{"max_chars": 20000}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let has_target = ["selector", "id", "ref", "element"]
                .iter()
                .any(|k| ctx.params.get(*k).is_some());
            let text = if has_target {
                let el = ctx.element_snapshot(tab)?;
                el.text.clone().unwrap_or_default()
            } else {
                ctx.runtime.engine().get_page_text(tab)?
            };
            let max = ctx.param_opt::<usize>("max_chars")?.unwrap_or(0);
            Ok(capped_text("text", &text, max))
        },
    )
}

fn extract_html() -> Tool {
    Tool::new(
        "extract_html",
        "Extract the page HTML. Optional 'max_chars' caps the result.",
        json!({"max_chars": {"type": "integer", "required": false}}),
        r#"{"max_chars": 50000}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let html = ctx.runtime.engine().get_page_html(tab)?;
            let max = ctx.param_opt::<usize>("max_chars")?.unwrap_or(0);
            Ok(capped_text("html", &html, max))
        },
    )
}

fn extract_links() -> Tool {
    Tool::new(
        "extract_links",
        "Extract links (url + text) on the page, or within one element when 'selector'/'id'/'ref'/'element' is given. Optional 'limit' caps the count.",
        json!({
            "selector": {"type": "string", "required": false},
            "id": {"type": "string", "required": false},
            "ref": {"type": "string", "required": false},
            "element": {"type": "string", "required": false},
            "limit": {"type": "integer", "required": false}
        }),
        r#"{"limit": 200}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let has_target = ["selector", "id", "ref", "element"]
                .iter()
                .any(|k| ctx.params.get(*k).is_some());
            let links: Value = if has_target {
                // Scope to the element's subtree: tag it, then collect links.
                let r = ctx.element_ref()?;
                let _ = ctx.eval_opt(tab, &crate::engine::inject::live_resolve_js(&r));
                ctx.eval_opt(tab, &crate::engine::inject::element_links_js())
                    .unwrap_or_else(|| Value::Array(vec![]))
            } else {
                serde_json::to_value(ctx.runtime.engine().get_links(tab)?)
                    .unwrap_or_else(|_| Value::Array(vec![]))
            };
            let max = ctx.param_opt::<usize>("limit")?.unwrap_or(0);
            Ok(capped_array("links", links, max))
        },
    )
}

fn extract_images() -> Tool {
    Tool::new(
        "extract_images",
        "Extract images (src + alt) on the page. Optional 'limit' caps the count. By default obvious site-chrome assets (logos, QR codes, sprites, watermarks) are filtered out; pass \"all\": true to include them.",
        json!({
            "limit": {"type": "integer", "required": false},
            "all": {"type": "boolean", "default": false}
        }),
        r#"{"limit": 200}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let images = serde_json::to_value(ctx.runtime.engine().get_images(tab)?)
                .unwrap_or_else(|_| Value::Array(vec![]));
            let max = ctx.param_opt::<usize>("limit")?.unwrap_or(0);
            let all = ctx.param_opt::<bool>("all")?.unwrap_or(false);
            let images = if all {
                images
            } else {
                Value::Array(
                    images
                        .as_array()
                        .cloned()
                        .unwrap_or_default()
                        .into_iter()
                        .filter(|img| {
                            !is_chrome_asset(img.get("src").and_then(|s| s.as_str()).unwrap_or(""))
                        })
                        .collect(),
                )
            };
            Ok(capped_array("images", images, max))
        },
    )
}

/// Heuristic: does this image URL look like site chrome (logo / QR / sprite /
/// watermark) rather than page content? Used to keep `extract_images` output
/// focused on actual content images.
fn is_chrome_asset(src: &str) -> bool {
    let s = src.to_ascii_lowercase();
    const MARKERS: &[&str] = &[
        "logo",
        "qrcode",
        "qr-code",
        "qr_code",
        "favicon",
        "sprite",
        "placeholder",
        "watermark",
        "sitelogo",
    ];
    MARKERS.iter().any(|m| s.contains(m))
}

fn extract_table() -> Tool {
    Tool::new(
        "extract_table",
        "Extract a data table as rows of cells. By default the first table; pass 'selector' (CSS) and/or 'index' to choose another.",
        json!({
            "selector": {"type": "string", "required": false},
            "index": {"type": "integer", "required": false}
        }),
        r##"{"selector": "#results"}"##,
        |ctx| {
            let tab = ctx.target_tab()?;
            let selector = ctx.param_opt::<String>("selector")?.filter(|s| !s.is_empty());
            let index = ctx.param_opt::<usize>("index")?.unwrap_or(0);
            let js_for = |q: String, i: usize| {
                format!(
                    "(()=>{{const els=document.querySelectorAll({q});const t=els[{i}];if(!t)return [];const rows=[];t.querySelectorAll('tr').forEach(function(tr){{var r=[];tr.querySelectorAll('th,td').forEach(function(c){{r.push((c.innerText||c.textContent||'').trim());}});rows.push(r);}});return rows;}})()"
                )
            };
            if let Some(sel) = selector {
                let q = serde_json::to_string(&sel).unwrap_or_else(|_| "\"table\"".into());
                let v = ctx
                    .eval_opt(tab, &js_for(q, index))
                    .filter(|v| v.is_array())
                    .unwrap_or(Value::Array(vec![]));
                return Ok(json!({"table": v}));
            }
            if index > 0 {
                let v = ctx
                    .eval_opt(tab, &js_for("\"table\"".into(), index))
                    .filter(|v| v.is_array())
                    .unwrap_or(Value::Array(vec![]));
                return Ok(json!({"table": v}));
            }
            let table = ctx.runtime.engine().get_table(tab)?;
            Ok(json!({"table": table}))
        },
    )
}

fn extract_json() -> Tool {
    Tool::new(
        "extract_json",
        "Run a JavaScript expression and return its JSON result (e.g. document.querySelectorAll('a')).",
        json!({"script": {"type": "string", "required": true}}),
        r#"{"script": "document.querySelectorAll('a')"}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let script = ctx.param_str("script")?;
            let value = ctx.runtime.engine().evaluate(tab, &script)?;
            Ok(json!({"result": crate::tools::tool::surface_js_error(value)?}))
        },
    )
}

fn search() -> Tool {
    Tool::new(
        "search",
        "Search the page text for 'query' (plain substring, or regex when 'regex' is true). Returns matching snippets with element ids when available.",
        json!({
            "query": {"type": "string", "required": true},
            "regex": {"type": "boolean", "default": false},
            "limit": {"type": "integer", "default": 20}
        }),
        r#"{"query": "price", "limit": 10}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let query = ctx.param_str("query")?;
            let regex = ctx.param_opt::<bool>("regex")?.unwrap_or(false);
            let limit = ctx.param_opt::<usize>("limit")?.unwrap_or(20);
            let query_json = serde_json::to_string(&query).unwrap_or_else(|_| "\"\"".into());
            let re = if regex {
                format!("new RegExp({query_json})")
            } else {
                query_json.clone()
            };
            let js = format!(
                r#"(()=>{{
                    const isRegex = {regex};
                    const queryStr = {query_json};
                    const q = {re};
                    const MAX = {limit};
                    const matches = [];
                    const root = document.body || document.documentElement;
                    if (!root) return matches;
                    const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
                    while (walker.nextNode()) {{
                        const t = walker.currentNode.textContent || '';
                        if (t.trim().length === 0) continue;
                        const ok = isRegex ? q.test(t) : t.includes(queryStr);
                        if (ok) {{
                            matches.push({{ snippet: t.trim().slice(0, 160) }});
                            if (matches.length >= MAX) break;
                        }}
                    }}
                    return matches;
                }})()"#,
            );
            let v = ctx.eval_opt(tab, &js).unwrap_or(Value::Null);
            Ok(json!({"matches": v, "count": v.as_array().map(|a| a.len()).unwrap_or(0)}))
        },
    )
}

fn find_elements() -> Tool {
    Tool::new(
        "find_elements",
        "Find elements matching a 'selector' (or an accessibility 'role' + optional 'name'). Supports standard CSS, plus Playwright-style prefixes (css= / text= / xpath= / id= / role= / data-testid=), `role=button[name=\"X\"]`, chained `A >> B`, and CSS pseudo-classes `:visible` / `:has-text()`. Returns tag/text/href/rect for each.",
        json!({
            "selector": {"type": "string", "required": false},
            "role": {"type": "string", "required": false},
            "name": {"type": "string", "required": false},
            "limit": {"type": "integer", "default": 20}
        }),
        r#"{"selector": "a.product-link", "limit": 5}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let selector = match ctx.param_opt::<String>("selector")? {
                Some(s) if !s.trim().is_empty() => s,
                _ => {
                    // Accessibility shorthand: role/name → Playwright selector.
                    let role = ctx
                        .param_opt::<String>("role")?
                        .filter(|s| !s.trim().is_empty());
                    let name = ctx
                        .param_opt::<String>("name")?
                        .filter(|s| !s.trim().is_empty());
                    match (role, name) {
                        (Some(r), Some(n)) => format!("role={r}[name={n:?}]"),
                        (Some(r), None) => format!("role={r}"),
                        (None, Some(n)) => format!("text={n}"),
                        (None, None) => {
                            return Err(crate::engine::EngineError::invalid(
                                "find_elements: need 'selector', 'role' or 'name'",
                            ))
                        }
                    }
                }
            };
            let limit = ctx.param_opt::<usize>("limit")?.unwrap_or(20);
            let sel = serde_json::to_string(&selector).unwrap_or_else(|_| "\"\"".into());
            let engine = crate::engine::inject::fb_query_js();
            let js = format!(
                r#"(()=>{{
                    {engine}
                    const sel = {sel};
                    const out = [];
                    let els;
                    const isPlaywright = /^(css|text|xpath|id|data-testid|testid|role|label|placeholder|alt|title|value|href|ref)=/.test(sel.trim())
                        || sel.indexOf('>>') >= 0 || sel.indexOf(':has-text') >= 0 || sel.indexOf(':text(') >= 0 || sel.indexOf(':visible') >= 0;
                    if (isPlaywright && window.__fbQueryAll) {{
                        els = window.__fbQueryAll(sel);
                    }} else {{
                        try {{ els = Array.prototype.slice.call(document.querySelectorAll(sel)); }}
                        catch (e) {{ els = []; }}
                    }}
                    for (const el of els) {{
                        if (out.length >= {limit}) break;
                        const r = el.getBoundingClientRect();
                        // Never expose the internal live sentinel (\u0001) as an id/ref.
                        const fb = el.getAttribute('data-fb');
                        const rid = (fb && fb !== '\u0001') ? fb : null;
                        out.push({{
                            tag: el.tagName.toLowerCase(),
                            text: (el.innerText || el.textContent || '').trim().slice(0, 120),
                            href: el.getAttribute('href'),
                            role: el.getAttribute('role') || null,
                            name: (el.getAttribute('aria-label') || '').trim() || null,
                            id: rid,
                            ref: rid,
                            rect: {{ x: r.x, y: r.y, width: r.width, height: r.height }}
                        }});
                    }}
                    return out;
                }})()"#,
                engine = engine,
                sel = sel,
                limit = limit,
            );
            // `elements` is ALWAYS an array: a no-match Playwright selector used
            // to return `null` from __fbQueryAll (→ "els is not iterable"), and a
            // non-JS engine (mock) returns null. Coerce defensively.
            let v = ctx.eval_opt(tab, &js).unwrap_or(Value::Null);
            let arr = match v {
                Value::Array(a) => a,
                _ => Vec::new(),
            };
            Ok(json!({"elements": arr, "count": arr.len()}))
        },
    )
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

    fn call(tool: &Tool, r: &Runtime, params: Value) -> Value {
        tool.run(&ToolContext { runtime: r, params }).unwrap()
    }

    #[test]
    fn text_links_images_table() {
        let r = runtime();
        let t = call(&extract_text(), &r, json!({}));
        assert!(t["text"]
            .as_str()
            .unwrap()
            .contains("Welcome to FastBrowser"));
        let l = call(&extract_links(), &r, json!({}));
        assert_eq!(l["links"][0]["url"], "https://example.com/about");
        let im = call(&extract_images(), &r, json!({}));
        // The mock page's only image is a logo → filtered by default.
        assert_eq!(im["count"], json!(0), "{im}");
        let im_all = call(&extract_images(), &r, json!({"all": true}));
        assert_eq!(im_all["images"][0]["src"], "https://example.com/logo.png");
        let tb = call(&extract_table(), &r, json!({}));
        assert_eq!(tb["table"][0][0], "Name");
        let h = call(&extract_html(), &r, json!({}));
        assert!(h["html"].as_str().unwrap().contains("<body>"));
    }

    #[test]
    fn find_elements_no_match_is_empty_array_not_error() {
        // Regression: a Playwright selector with no match must yield
        // {count: 0, elements: []} — never an error/null (find_elements iterates
        // __fbQueryAll, which used to return null).
        let r = runtime();
        let v = call(
            &find_elements(),
            &r,
            json!({"selector": "role=button[name=\"__nope__\"]"}),
        );
        assert_eq!(v["count"], json!(0), "{v}");
        assert!(v["elements"].is_array(), "elements must be an array: {v}");
        assert_eq!(v["elements"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn find_elements_accepts_role_and_name() {
        // `role`/`name` are synthesized into a Playwright selector (previously
        // the call failed with "missing required param 'selector'").
        let r = runtime();
        let v = call(
            &find_elements(),
            &r,
            json!({"role": "button", "name": "Go"}),
        );
        assert_eq!(v["count"], json!(0), "{v}");
        assert!(v["elements"].is_array(), "{v}");
        let v = call(&find_elements(), &r, json!({"role": "link"}));
        assert!(v["elements"].is_array(), "{v}");
        // No selector/role/name → a clear error, not a silent empty result.
        let err = find_elements().run(&ToolContext {
            runtime: &r,
            params: json!({}),
        });
        assert!(err.is_err(), "needs selector/role/name");
    }

    #[test]
    fn extract_text_and_links_declare_element_scope() {
        // `selector`/`ref`/`element` must be declared (else they'd be ignored
        // with an "unknown param" warning).
        let text_tool = extract_text();
        let text_params = text_tool.spec.params.as_object().unwrap();
        for k in ["selector", "ref", "element", "id"] {
            assert!(text_params.contains_key(k), "extract_text param {k}");
        }
        let link_tool = extract_links();
        let link_params = link_tool.spec.params.as_object().unwrap();
        for k in ["selector", "ref", "element"] {
            assert!(link_params.contains_key(k), "extract_links param {k}");
        }
    }

    #[test]
    fn extract_json_via_js() {
        let r = runtime();
        let v = call(
            &extract_json(),
            &r,
            json!({"script": "document.querySelectorAll('a').length"}),
        );
        assert_eq!(v["result"], json!(1));
    }

    #[test]
    fn chrome_asset_filter() {
        for src in [
            "https://x/logo.png",
            "https://x/ppbcqrcode.jpg",
            "https://x/site_sprite.png",
            "https://x/watermark.png",
            "https://x/favicon.ico",
        ] {
            assert!(is_chrome_asset(src), "{src} should be chrome");
        }
        for src in [
            "https://x/plant_photo.jpg",
            "https://img8.iplant.cn/image2/236/30DEC561AE293746.jpg",
        ] {
            assert!(!is_chrome_asset(src), "{src} should be content");
        }
    }
}
