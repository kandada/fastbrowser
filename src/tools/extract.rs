// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Content-extraction tools: extract_text/html/links/images/table/json + search/find_elements.

use serde_json::{json, Value};

use crate::tools::tool::Tool;

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
        "Extract visible text of the page, or of one element when 'id'/'ref' is given.",
        json!({"id": {"type": "string", "required": false}}),
        r#"{}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let has_id = ctx.params.get("id").is_some();
            let has_ref = ctx.params.get("ref").is_some();
            let text = if has_id || has_ref {
                let el = ctx.element_snapshot(tab)?;
                el.text.clone().unwrap_or_default()
            } else {
                ctx.runtime.engine().get_page_text(tab)?
            };
            Ok(json!({"text": text}))
        },
    )
}

fn extract_html() -> Tool {
    Tool::new(
        "extract_html",
        "Extract the page HTML.",
        json!({}),
        r#"{}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let html = ctx.runtime.engine().get_page_html(tab)?;
            Ok(json!({"html": html}))
        },
    )
}

fn extract_links() -> Tool {
    Tool::new(
        "extract_links",
        "Extract all links (url + text) on the page.",
        json!({}),
        r#"{}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let links = ctx.runtime.engine().get_links(tab)?;
            Ok(json!({"links": links}))
        },
    )
}

fn extract_images() -> Tool {
    Tool::new(
        "extract_images",
        "Extract all images (src + alt) on the page.",
        json!({}),
        r#"{}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let images = ctx.runtime.engine().get_images(tab)?;
            Ok(json!({"images": images}))
        },
    )
}

fn extract_table() -> Tool {
    Tool::new(
        "extract_table",
        "Extract the first data table on the page as rows of cells.",
        json!({}),
        r#"{}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
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
            Ok(json!({"result": value}))
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
                    const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
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
        "Find elements matching a CSS 'selector'. Returns tag/text/href/rect for each, with snapshot id when annotated.",
        json!({
            "selector": {"type": "string", "required": true},
            "limit": {"type": "integer", "default": 20}
        }),
        r#"{"selector": "a.product-link", "limit": 5}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let selector = ctx.param_str("selector")?;
            let limit = ctx.param_opt::<usize>("limit")?.unwrap_or(20);
            let sel = serde_json::to_string(&selector).unwrap_or_else(|_| "\"\"".into());
            let js = format!(
                r#"(()=>{{
                    const sel = {sel};
                    const out = [];
                    for (const el of document.querySelectorAll(sel)) {{
                        if (out.length >= {limit}) break;
                        const r = el.getBoundingClientRect();
                        out.push({{
                            tag: el.tagName.toLowerCase(),
                            text: (el.innerText || el.textContent || '').trim().slice(0, 120),
                            href: el.getAttribute('href'),
                            id: el.getAttribute('data-fb'),
                            rect: {{ x: r.x, y: r.y, width: r.width, height: r.height }}
                        }});
                    }}
                    return out;
                }})()"#,
                sel = sel,
                limit = limit,
            );
            let v = ctx.eval_opt(tab, &js).unwrap_or(Value::Null);
            Ok(json!({"elements": v, "count": v.as_array().map(|a| a.len()).unwrap_or(0)}))
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
        assert_eq!(im["images"][0]["src"], "https://example.com/logo.png");
        let tb = call(&extract_table(), &r, json!({}));
        assert_eq!(tb["table"][0][0], "Name");
        let h = call(&extract_html(), &r, json!({}));
        assert!(h["html"].as_str().unwrap().contains("<body>"));
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
}
