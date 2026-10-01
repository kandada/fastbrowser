// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Page-analysis tools: screenshot / get_page_title / get_current_url / get_page_text / get_element_info.

use serde_json::{json, Value};

use crate::engine::Capability;
use crate::tools::tool::{capped_text, Tool};

pub fn tools() -> Vec<Tool> {
    vec![
        screenshot(),
        screenshot_element(),
        get_page_title(),
        get_current_url(),
        get_page_text(),
        get_element_info(),
        get_element_text(),
        get_attributes(),
        is_visible(),
        is_enabled(),
        get_focused_element(),
        get_selected_text(),
        get_page_meta(),
        get_scroll_position(),
        set_scroll_position(),
        get_performance_metrics(),
        get_accessibility_tree(),
        get_console_logs(),
        get_network_log(),
    ]
}

fn screenshot() -> Tool {
    Tool::new(
        "screenshot",
        "Capture the active tab. 'format' controls the returned image encoding: 'rgba' (default, lossless raw RGBA base64), 'png' (lossless PNG bytes), or 'jpeg' (smaller, lossy, ~80 quality). png/jpeg are smaller over the wire and cheaper for the agent to consume.",
        json!({
            "format": {"type": "string", "enum": ["rgba", "png", "jpeg"], "default": "rgba", "required": false}
        }),
        r#"{"format": "jpeg"}"#,
        // Screenshot relies on CDP captureScreenshot; the system webview engine does not support it, so hide it from the LLM.
        |ctx| {
            let tab = ctx.target_tab()?;
            let format = ctx.param_opt::<String>("format")?.unwrap_or_else(|| "rgba".into());
            if format != "rgba" {
                // Encoded-bytes path (png/jpeg): smaller and avoids a second decode; fall back to RGBA if unsupported.
                if let Ok((w, h, data)) = ctx.runtime.engine().capture_encoded(tab, &format) {
                    return Ok(json!({
                        "width": w,
                        "height": h,
                        "format": format,
                        "base64": base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &data),
                    }));
                }
            }
            let img = ctx.runtime.engine().screenshot(tab)?;
            Ok(json!({
                "width": img.width,
                "height": img.height,
                "format": "rgba",
                "base64": img.to_base64(),
            }))
        },
    )
    .requires(&[Capability::Screenshot])
}

fn get_page_title() -> Tool {
    Tool::new(
        "get_page_title",
        "Return the current page title.",
        json!({}),
        r#"{}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let title = ctx.runtime.engine().page_title(tab)?;
            Ok(json!({"title": title}))
        },
    )
}

fn get_current_url() -> Tool {
    Tool::new(
        "get_current_url",
        "Return the current page URL.",
        json!({}),
        r#"{}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let url = ctx.runtime.engine().page_url(tab)?;
            Ok(json!({"url": url}))
        },
    )
}

fn get_page_text() -> Tool {
    Tool::new(
        "get_page_text",
        "Return the visible text of the whole page. Optional 'max_chars' caps the result.",
        json!({"max_chars": {"type": "integer", "required": false}}),
        r#"{"max_chars": 20000}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let text = ctx.runtime.engine().get_page_text(tab)?;
            let max = ctx.param_opt::<usize>("max_chars")?.unwrap_or(0);
            Ok(capped_text("text", &text, max))
        },
    )
}

fn screenshot_element() -> Tool {
    Tool::new(
        "screenshot_element",
        "Capture a screenshot cropped to one element (by 'selector', 'id' or 'ref'). Returns base64 (rgba; use the 'screenshot' tool's `format` for png/jpeg).",
        json!({
            "selector": {"type": "string", "required": false},
            "id": {"type": "string", "required": false},
            "ref": {"type": "string", "required": false}
        }),
        r##"{"selector": "#hero"}"##,
        |ctx| {
            let tab = ctx.target_tab()?;
            let el = ctx.element_snapshot(tab)?;
            let img = ctx.runtime.engine().screenshot(tab)?;
            let (w, h) = (img.width as isize, img.height as isize);
            let x0 = el.rect.x.max(0.0) as isize;
            let y0 = el.rect.y.max(0.0) as isize;
            if x0 >= w || y0 >= h {
                return Ok(json!({ "width": 0, "height": 0, "format": "rgba", "base64": "" }));
            }
            let cw = ((el.rect.width as isize).min(w - x0)).max(0) as usize;
            let ch = ((el.rect.height as isize).min(h - y0)).max(0) as usize;
            let mut crop = Vec::with_capacity(cw * ch * 4);
            for row in 0..ch {
                let src = ((y0 as usize + row) * w as usize + x0 as usize) * 4;
                let end = src + cw * 4;
                crop.extend_from_slice(&img.rgba[src..end]);
            }
            Ok(json!({
                "width": cw, "height": ch, "format": "rgba",
                "base64": base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &crop),
            }))
        },
    )
    .requires(&[Capability::Screenshot])
}

fn get_element_text() -> Tool {
    Tool::new(
        "get_element_text",
        "Return the visible text of one element (by 'selector', 'ref', 'element' or snapshot 'id').",
        json!({
            "selector": {"type": "string", "required": false},
            "id": {"type": "string", "required": false},
            "ref": {"type": "string", "required": false},
            "element": {"type": "string", "required": false}
        }),
        r#"{"id": "c"}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let el = ctx.element_snapshot(tab)?;
            Ok(json!({"id": el.display_id(), "text": el.text.unwrap_or_default()}))
        },
    )
}

fn get_attributes() -> Tool {
    Tool::new(
        "get_attributes",
        "Return the attributes of one element (by 'selector', 'ref' or snapshot 'id').",
        json!({
            "selector": {"type": "string", "required": false},
            "id": {"type": "string", "required": false},
            "ref": {"type": "string", "required": false}
        }),
        r##"{"selector": "#go"}"##,
        |ctx| {
            let tab = ctx.target_tab()?;
            let el = ctx.element_snapshot(tab)?;
            Ok(json!({"id": el.display_id(), "attrs": el.attrs}))
        },
    )
}

fn is_visible() -> Tool {
    Tool::new(
        "is_visible",
        "Check whether an element (by 'selector', 'ref' or snapshot 'id') is visible.",
        json!({
            "selector": {"type": "string", "required": false},
            "id": {"type": "string", "required": false},
            "ref": {"type": "string", "required": false}
        }),
        r##"{"selector": "#go"}"##,
        |ctx| {
            let tab = ctx.target_tab()?;
            let el = ctx.element_snapshot(tab)?;
            Ok(json!({"id": el.display_id(), "visible": el.visible}))
        },
    )
}

fn is_enabled() -> Tool {
    Tool::new(
        "is_enabled",
        "Check whether an element (by 'selector', 'ref' or snapshot 'id') is visible and not disabled.",
        json!({
            "selector": {"type": "string", "required": false},
            "id": {"type": "string", "required": false},
            "ref": {"type": "string", "required": false}
        }),
        r##"{"selector": "#go"}"##,
        |ctx| {
            let tab = ctx.target_tab()?;
            let el = ctx.element_snapshot(tab)?;
            let disabled = el.attrs.contains_key("disabled")
                || el
                    .attrs
                    .get("aria-disabled")
                    .map(|v| v == "true")
                    .unwrap_or(false);
            Ok(json!({"id": el.display_id(), "enabled": el.visible && !disabled}))
        },
    )
}

fn get_focused_element() -> Tool {
    Tool::new(
        "get_focused_element",
        "Return the currently focused element (tag, id, text).",
        json!({}),
        r#"{}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let v = ctx.eval_opt(tab, "(()=>{const e=document.activeElement;if(!e)return null;return {tag:e.tagName,id:e.id||null,text:(e.innerText||'').trim().slice(0,100)};})()");
            Ok(json!({"focused": v.unwrap_or(Value::Null)}))
        },
    )
}

fn get_selected_text() -> Tool {
    Tool::new(
        "get_selected_text",
        "Return the text currently selected on the page.",
        json!({}),
        r#"{}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let v = ctx.eval_opt(
                tab,
                "window.getSelection()?window.getSelection().toString():''",
            );
            Ok(json!({"text": v.and_then(|x| x.as_str().map(String::from)).unwrap_or_default()}))
        },
    )
}

fn get_page_meta() -> Tool {
    Tool::new(
        "get_page_meta",
        "Return page metadata: title, description, keywords, canonical, og tags.",
        json!({}),
        r#"{}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let script = r#"(()=>{
                const m=(name)=>{const e=document.querySelector(`meta[name="${name}"],meta[property="og:${name}"]`);return e?e.content:null};
                const canonical=document.querySelector('link[rel="canonical"]');
                return {title:document.title,description:m('description'),keywords:m('keywords'),canonical:canonical?canonical.href:null,og_title:m('title'),og_image:m('image')};
            })()"#;
            let v = ctx.eval_opt(tab, script).unwrap_or(Value::Null);
            Ok(json!({"meta": v}))
        },
    )
}

fn get_scroll_position() -> Tool {
    Tool::new(
        "get_scroll_position",
        "Return current scroll position and document height.",
        json!({}),
        r#"{}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let v = ctx.eval_opt(tab, "({x: window.scrollX||0, y: window.scrollY||0, height: document.documentElement.scrollHeight||0})");
            Ok(json!({"scroll": v.unwrap_or(Value::Null)}))
        },
    )
}

fn set_scroll_position() -> Tool {
    Tool::new(
        "set_scroll_position",
        "Scroll to absolute (x, y). Aliases: dx→x, dy→y, left→x, top→y.",
        json!({
            "x": {"type": "number", "required": false},
            "y": {"type": "number", "required": false},
            "dx": {"type": "number", "required": false},
            "dy": {"type": "number", "required": false},
            "left": {"type": "number", "required": false},
            "top": {"type": "number", "required": false}
        }),
        r#"{"y": 400}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let x = ctx
                .param_opt::<f64>("x")?
                .or(ctx.param_opt::<f64>("dx")?)
                .or(ctx.param_opt::<f64>("left")?)
                .unwrap_or(0.0);
            let y = ctx
                .param_opt::<f64>("y")?
                .or(ctx.param_opt::<f64>("dy")?)
                .or(ctx.param_opt::<f64>("top")?)
                .unwrap_or(0.0);
            let _ = ctx.eval_opt(tab, &format!("window.scrollTo({x},{y})"));
            let actual = ctx
                .eval_opt(
                    tab,
                    "(function(){return {x:window.scrollX||0,y:window.scrollY||0};})()",
                )
                .unwrap_or(serde_json::Value::Null);
            Ok(json!({"x": x, "y": y, "ok": true, "actual": actual}))
        },
    )
}

fn get_performance_metrics() -> Tool {
    Tool::new(
        "get_performance_metrics",
        "Return performance timing metrics (navigationStart, domContentLoaded, load, etc.).",
        json!({}),
        r#"{}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let script = r#"(()=>{if(!window.performance||!window.performance.timing)return null;const t=window.performance.timing;return {navigationStart:t.navigationStart,domContentLoadedEnd:t.domContentLoadedEnd,loadEventEnd:t.loadEventEnd,domContentLoaded:Math.max(0,t.domContentLoadedEnd-t.navigationStart),load:Math.max(0,t.loadEventEnd-t.navigationStart)};})()"#;
            let v = ctx.eval_opt(tab, script).unwrap_or(Value::Null);
            Ok(json!({"metrics": v}))
        },
    )
}

/// Default node cap when `max_nodes` is omitted (keeps the payload small).
const DEFAULT_AX_NODES: usize = 300;
/// Default tree-depth cap (0 = unlimited).
const DEFAULT_AX_DEPTH: usize = 12;
/// Default per-text-field cap for name/value/description.
const DEFAULT_AX_TEXT_LEN: usize = 200;
/// Default hard cap on the serialized payload (chars); a safety net against
/// explosion. 0 = unlimited.
const DEFAULT_AX_CHARS: usize = 20_000;

/// Recursively drop pure-noise nodes: no role, no name, no value, and no
/// surviving children. Returns `None` when the whole subtree is noise.
fn filter_ax_node(node: &Value) -> Option<Value> {
    let mut out = node.clone();
    let kids: Vec<Value> = node
        .get("children")
        .and_then(|c| c.as_array())
        .map(|a| a.iter().filter_map(filter_ax_node).collect())
        .unwrap_or_default();
    let role = node.get("role").and_then(Value::as_str).unwrap_or("");
    let name = node.get("name").and_then(Value::as_str).unwrap_or("");
    let value = node.get("value").and_then(Value::as_str).unwrap_or("");
    if role.is_empty() && name.is_empty() && value.is_empty() && kids.is_empty() {
        return None;
    }
    if let Some(obj) = out.as_object_mut() {
        if kids.is_empty() {
            obj.remove("children");
        } else {
            obj.insert("children".to_string(), Value::Array(kids));
        }
    }
    Some(out)
}

/// Count nodes in a nested AX tree.
fn count_ax_nodes(node: &Value) -> usize {
    1 + node
        .get("children")
        .and_then(|c| c.as_array())
        .map(|a| a.iter().map(count_ax_nodes).sum())
        .unwrap_or(0)
}

/// Quality probe for a nested AX tree: `(total, roleless, single_char_names)`.
/// A tree that is mostly role-less single-character nodes indicates a
/// glyph-per-node AX source (older Android WebView host AX), where
/// `ax_snapshot` gives far better semantic perception.
fn ax_quality(node: &Value, acc: &mut (usize, usize, usize)) {
    acc.0 += 1;
    if node
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or("")
        .is_empty()
    {
        acc.1 += 1;
    }
    if node
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .chars()
        .count()
        == 1
    {
        acc.2 += 1;
    }
    if let Some(kids) = node.get("children").and_then(|c| c.as_array()) {
        for k in kids {
            ax_quality(k, acc);
        }
    }
}

/// Recursively prune a nested AX node (`{role,name,...,children:[...]}`) to at
/// most `*budget` nodes (pre-order). Returns `None` once the budget is spent.
fn prune_ax_node(node: &Value, budget: &mut usize) -> Option<Value> {
    if *budget == 0 {
        return None;
    }
    *budget -= 1;
    let mut out = node.clone();
    if let Some(children) = node.get("children").and_then(|c| c.as_array()) {
        let mut kept = Vec::new();
        for ch in children {
            match prune_ax_node(ch, budget) {
                Some(p) => kept.push(p),
                None => break,
            }
        }
        if let Some(obj) = out.as_object_mut() {
            obj.insert("children".to_string(), Value::Array(kept));
        }
    }
    Some(out)
}

/// Clamp tree depth to `max` (1-based; `0` = unlimited).
fn clamp_ax_depth(node: &Value, depth: usize, max: usize) -> Value {
    let mut out = node.clone();
    let kids = node.get("children").and_then(|c| c.as_array());
    if let (Some(obj), Some(kids)) = (out.as_object_mut(), kids) {
        if max != 0 && depth >= max {
            obj.remove("children");
        } else {
            let kept: Vec<Value> = kids
                .iter()
                .map(|k| clamp_ax_depth(k, depth + 1, max))
                .collect();
            if kept.is_empty() {
                obj.remove("children");
            } else {
                obj.insert("children".to_string(), Value::Array(kept));
            }
        }
    }
    out
}

fn truncate_text(s: &str, max: usize) -> String {
    if max == 0 || s.chars().count() <= max {
        s.to_string()
    } else {
        let head: String = s.chars().take(max).collect();
        format!("{head}…")
    }
}

/// Round geometry floats to 1 decimal so a node box is ~20 chars, not ~90.
fn round_geometry(g: &Value) -> Value {
    match g.as_object() {
        Some(o) => {
            let mut m = serde_json::Map::new();
            for (k, v) in o {
                if let Some(f) = v.as_f64() {
                    m.insert(k.clone(), json!((f * 10.0).round() / 10.0));
                } else {
                    m.insert(k.clone(), v.clone());
                }
            }
            Value::Object(m)
        }
        None => g.clone(),
    }
}

/// Build the lean, LLM-facing node: drop empty fields, truncate text; include
/// geometry only when asked; include description/props/backend_id only in
/// `full` detail.
fn compact_ax_node(node: &Value, include_geometry: bool, full: bool, max_text_len: usize) -> Value {
    let mut m = serde_json::Map::new();
    let s = |k: &str| node.get(k).and_then(Value::as_str).unwrap_or("");
    for key in ["role", "name", "value"] {
        let v = s(key);
        if !v.is_empty() {
            m.insert(key.to_string(), json!(truncate_text(v, max_text_len)));
        }
    }
    if full {
        let d = s("description");
        if !d.is_empty() {
            m.insert(
                "description".to_string(),
                json!(truncate_text(d, max_text_len)),
            );
        }
        if let Some(p) = node.get("props") {
            if !p.is_null() {
                m.insert("props".to_string(), p.clone());
            }
        }
        if let Some(b) = node.get("backend_id") {
            if !b.is_null() {
                m.insert("backend_id".to_string(), b.clone());
            }
        }
    }
    if include_geometry {
        if let Some(g) = node.get("geometry") {
            if !g.is_null() {
                m.insert("geometry".to_string(), round_geometry(g));
            }
        }
    }
    if let Some(kids) = node.get("children").and_then(|c| c.as_array()) {
        let kept: Vec<Value> = kids
            .iter()
            .map(|k| compact_ax_node(k, include_geometry, full, max_text_len))
            .collect();
        if !kept.is_empty() {
            m.insert("children".to_string(), Value::Array(kept));
        }
    }
    Value::Object(m)
}

fn ax_flatten(node: &Value, out: &mut Vec<Value>) {
    let mut n = node.clone();
    if let Some(o) = n.as_object_mut() {
        o.remove("children");
    }
    out.push(n);
    if let Some(kids) = node.get("children").and_then(|c| c.as_array()) {
        for k in kids {
            ax_flatten(k, out);
        }
    }
}

fn ax_render_text(node: &Value, depth: usize, out: &mut String) {
    let s = |k: &str| node.get(k).and_then(Value::as_str).unwrap_or("");
    let role = s("role");
    let name = s("name");
    let value = s("value");
    out.push_str(&"  ".repeat(depth));
    out.push_str(if role.is_empty() { "text" } else { role });
    if !name.is_empty() {
        out.push_str(&format!(" \"{name}\""));
    }
    if !value.is_empty() {
        out.push_str(&format!(" = \"{value}\""));
    }
    out.push('\n');
    if let Some(kids) = node.get("children").and_then(|c| c.as_array()) {
        for k in kids {
            ax_render_text(k, depth + 1, out);
        }
    }
}

/// Shrink a compact tree until its JSON fits `cap` chars (`0` = unlimited).
fn fit_ax_chars(mut tree: Value, cap: usize) -> Value {
    if cap == 0 || tree.is_null() {
        return tree;
    }
    let mut guard = 0;
    while serde_json::to_string(&tree).map(|s| s.len()).unwrap_or(0) > cap && guard < 32 {
        let n = count_ax_nodes(&tree);
        if n <= 1 {
            break;
        }
        let mut budget = (n * 4 / 5).max(1);
        tree = prune_ax_node(&tree, &mut budget).unwrap_or(Value::Null);
        guard += 1;
    }
    tree
}

fn get_accessibility_tree() -> Tool {
    Tool::new(
        "get_accessibility_tree",
        "Return an accessibility tree of the page (role/name/state), the LLM-native \
         perception format. Pruned by default (noise filtered; node/depth/text caps; \
         geometry omitted) to bound tokens. Use detail=\"full\" + include_geometry=true \
         + max_nodes=0 for the complete tree. 'view' selects shape (tree|flat|text). \
         Falls back to interactive elements when the engine lacks AX support.",
        json!({
            "max_nodes": {"type": "integer", "required": false, "description": "Max nodes (0 = unlimited). Default 300."},
            "max_depth": {"type": "integer", "required": false, "description": "Max tree depth (0 = unlimited). Default 12."},
            "interesting_only": {"type": "boolean", "required": false, "description": "Drop role-less/empty containers (default true)."},
            "max_text_len": {"type": "integer", "required": false, "description": "Max chars per name/value/description (default 200; 0 = unlimited)."},
            "include_geometry": {"type": "boolean", "required": false, "description": "Include node geometry (default false; only needed for coordinate actions)."},
            "detail": {"type": "string", "enum": ["compact", "full"], "required": false, "description": "compact (default) keeps role/name/value; full adds description/props/backend_id."},
            "view": {"type": "string", "enum": ["tree", "flat", "text"], "required": false, "description": "Output shape (default tree)."},
            "max_chars": {"type": "integer", "required": false, "description": "Hard cap on serialized output chars (default 20000; 0 = unlimited)."}
        }),
        r#"{"max_nodes": 300}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let max_nodes = ctx
                .param_opt::<usize>("max_nodes")?
                .or(ctx.param_opt::<usize>("maxNodes")?)
                .or(ctx.param_opt::<usize>("limit")?)
                .unwrap_or(DEFAULT_AX_NODES);
            let max_depth = ctx
                .param_opt::<usize>("max_depth")?
                .or(ctx.param_opt::<usize>("maxDepth")?)
                .unwrap_or(DEFAULT_AX_DEPTH);
            let interesting_only = ctx.param_opt::<bool>("interesting_only")?.unwrap_or(true);
            let max_text_len = ctx
                .param_opt::<usize>("max_text_len")?
                .unwrap_or(DEFAULT_AX_TEXT_LEN);
            let include_geometry = ctx.param_opt::<bool>("include_geometry")?.unwrap_or(false);
            let full = matches!(ctx.param_opt::<String>("detail")?.as_deref(), Some("full"));
            let view = ctx
                .param_opt::<String>("view")?
                .unwrap_or_else(|| "tree".to_string());
            let max_chars = ctx
                .param_opt::<usize>("max_chars")?
                .unwrap_or(DEFAULT_AX_CHARS);

            // Prefer the engine's AX tree (CDP Accessibility.getFullAXTree);
            // geometry is only fetched when `include_geometry` (perf).
            if let Ok(v) = ctx
                .runtime
                .engine()
                .accessibility_tree_opts(tab, include_geometry)
            {
                let obj = v;
                let count = obj.get("count").and_then(Value::as_u64).unwrap_or(0);
                let engine_truncated = obj
                    .get("truncated")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let root = obj.get("root").cloned().unwrap_or(Value::Null);
                if root.is_null() {
                    return Ok(obj);
                }
                // 1) drop pure-noise nodes; 2) depth cap; 3) node cap.
                let mut tree = if interesting_only {
                    filter_ax_node(&root).unwrap_or(Value::Null)
                } else {
                    root
                };
                if !tree.is_null() && max_depth != 0 {
                    tree = clamp_ax_depth(&tree, 1, max_depth);
                }
                if !tree.is_null() && max_nodes != 0 {
                    let mut budget = max_nodes;
                    tree = prune_ax_node(&tree, &mut budget).unwrap_or(Value::Null);
                }
                // 4) lean fields; 5) hard char budget.
                let mut tree = if tree.is_null() {
                    Value::Null
                } else {
                    compact_ax_node(&tree, include_geometry, full, max_text_len)
                };
                tree = fit_ax_chars(tree, max_chars);

                // Low-quality (role-less, per-character) probe on the result.
                let mut low = false;
                if !tree.is_null() {
                    let mut acc = (0usize, 0usize, 0usize);
                    ax_quality(&tree, &mut acc);
                    let (total, roleless, charname) = acc;
                    low = total >= 20 && roleless * 2 > total && charname * 2 > total;
                }

                let returned = if tree.is_null() {
                    0
                } else {
                    count_ax_nodes(&tree)
                };
                let mut out = json!({
                    "count": count,
                    "returned": returned,
                    "truncated": engine_truncated,
                });
                match view.as_str() {
                    "flat" => {
                        let mut arr = Vec::new();
                        if !tree.is_null() {
                            ax_flatten(&tree, &mut arr);
                        }
                        out["nodes"] = json!(arr);
                    }
                    "text" => {
                        let mut s = String::new();
                        if !tree.is_null() {
                            ax_render_text(&tree, 0, &mut s);
                        }
                        out["text"] = json!(s);
                    }
                    _ => out["root"] = tree,
                }
                if low {
                    out["quality"] = json!("low");
                    out["hint"] = json!(
                        "engine AX here is role-less/per-character (low signal); \
                         prefer `ax_snapshot` for role/name/state perception"
                    );
                }
                return Ok(out);
            }
            // Fallback: interactive elements from the snapshot.
            let snap = ctx.runtime.engine().snapshot(tab)?;
            let all: Vec<Value> = snap
                .interactive
                .iter()
                .map(|e| {
                    json!({
                        "id": e.display_id(),
                        "ref": e.display_id(),
                        "role": e.role.clone().unwrap_or_else(|| e.tag.clone()),
                        "tag": e.tag,
                        "text": e.text,
                        "href": e.href,
                        "checked": e.checked,
                        "selected": e.selected_option,
                        "input_type": e.input_type,
                        "visible": e.visible,
                    })
                })
                .collect();
            let total = all.len();
            let nodes: Vec<Value> = if max_nodes > 0 {
                all.into_iter().take(max_nodes).collect()
            } else {
                all
            };
            Ok(json!({
                "tree": nodes,
                "count": total,
                "fallback": true,
                "truncated": max_nodes > 0 && total > max_nodes
            }))
        },
    )
}

fn get_element_info() -> Tool {
    Tool::new(
        "get_element_info",
        "Return detailed info about one element (by 'selector', 'ref' or snapshot 'id'): tag, role, text, rect, attrs, value...",
        json!({
            "selector": {"type": "string", "required": false},
            "id": {"type": "string", "required": false},
            "ref": {"type": "string", "required": false},
            "element": {"type": "string", "required": false}
        }),
        r##"{"selector": "#go"}"##,
        |ctx| {
            let tab = ctx.target_tab()?;
            let el = ctx.element_snapshot(tab)?;
            Ok(json!({
                "id": el.display_id(),
                "tag": el.tag,
                "role": el.role,
                "text": el.text,
                "href": el.href,
                "rect": el.rect,
                "attrs": el.attrs,
                "value": el.value,
                "input_type": el.input_type,
                "checked": el.checked,
                "options": el.selectable_options,
                "selected": el.selected_option,
                "visible": el.visible,
                "refs": el.refs,
            }))
        },
    )
}

fn get_console_logs() -> Tool {
    Tool::new(
        "get_console_logs",
        "Return recent page console messages (level + text). Persists across drain_events; optional 'level' filter (log/info/warning/error/debug) and 'limit' (keep the latest N).",
        json!({
            "level": {"type": "string", "required": false},
            "limit": {"type": "integer", "default": 50}
        }),
        r#"{"level": "error", "limit": 20}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let limit = ctx.param_opt::<usize>("limit")?.unwrap_or(50);
            let level = ctx.param_opt::<String>("level")?.map(|s| s.to_lowercase());
            let mut logs: Vec<Value> = Vec::new();
            for ev in ctx.runtime.engine().recent_events(tab) {
                if let crate::engine::PageEvent::Console { level: l, message } = ev {
                    let ls = l.as_str();
                    if let Some(f) = &level {
                        if ls != f.as_str() {
                            continue;
                        }
                    }
                    logs.push(json!({"level": ls, "text": message}));
                }
            }
            // Fallback for engines without native console events (the webview
            // engine): install the interceptor (idempotent) and read the buffer.
            if logs.is_empty() {
                let _ = ctx.eval(tab, crate::engine::inject::console_capture_js());
                if let Ok(v) = ctx.eval(tab, "JSON.stringify(window.__fbConsoleLogs||[])") {
                    if let Some(s) = v.as_str() {
                        if let Ok(Value::Array(arr)) = serde_json::from_str::<Value>(s) {
                            for item in arr {
                                let lv = item.get("level").and_then(|x| x.as_str()).unwrap_or("log");
                                let lv = if lv == "warn" { "warning" } else { lv };
                                let msg = item.get("message").and_then(|x| x.as_str()).unwrap_or("");
                                if let Some(f) = &level {
                                    if lv != f.as_str() {
                                        continue;
                                    }
                                }
                                logs.push(json!({"level": lv, "text": msg}));
                            }
                        }
                    }
                }
            }
            let total = logs.len();
            if limit > 0 && logs.len() > limit {
                logs = logs.split_off(logs.len() - limit);
            }
            Ok(json!({"count": logs.len(), "total": total, "logs": logs}))
        },
    )
}

fn get_network_log() -> Tool {
    Tool::new(
        "get_network_log",
        "Return recent network activity (requests + responses) for the tab. Persists across drain_events; 'limit' keeps the latest N.",
        json!({"limit": {"type": "integer", "default": 50}}),
        r#"{"limit": 50}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let limit = ctx.param_opt::<usize>("limit")?.unwrap_or(50);
            let mut out: Vec<Value> = Vec::new();
            for ev in ctx.runtime.engine().recent_events(tab) {
                match ev {
                    crate::engine::PageEvent::Request { url, method } => {
                        out.push(json!({"type": "request", "method": method, "url": url}))
                    }
                    crate::engine::PageEvent::Response { url, status } => {
                        out.push(json!({"type": "response", "status": status, "url": url}))
                    }
                    _ => {}
                }
            }
            let total = out.len();
            if limit > 0 && out.len() > limit {
                out = out.split_off(out.len() - limit);
            }
            Ok(json!({"count": out.len(), "total": total, "entries": out}))
        },
    )
    // Network interception is a CDP-only capability; on the mobile webview
    // engine no request/response events are produced, so hide the tool rather
    // than return a permanently-empty log.
    .requires(&[Capability::NetworkControl])
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
    fn screenshot_format_encodes_png_and_jpeg() {
        let r = runtime();
        for (fmt, magic) in [
            ("png", &[0x89u8, 0x50, 0x4e, 0x47][..]),
            ("jpeg", &[0xffu8, 0xd8, 0xff][..]),
        ] {
            let v = call(&screenshot(), &r, json!({"format": fmt})).unwrap();
            assert_eq!(v["format"], json!(fmt), "format echoed");
            use base64::Engine;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(v["base64"].as_str().unwrap())
                .unwrap();
            assert!(
                bytes.starts_with(magic),
                "{fmt} magic: {:02x?}",
                &bytes[..bytes.len().min(4)]
            );
        }
    }

    #[test]
    fn set_scroll_position_aliases() {
        let r = runtime();
        let v = call(&set_scroll_position(), &r, json!({"dx": 5, "dy": 50})).unwrap();
        assert_eq!(v["x"], json!(5.0));
        assert_eq!(v["y"], json!(50.0));
        let v = call(&set_scroll_position(), &r, json!({"x": 1, "y": 2})).unwrap();
        assert_eq!(v["x"], json!(1.0));
        assert_eq!(v["y"], json!(2.0));
    }

    #[test]
    fn console_and_network_logs_read_persistent_buffer() {
        let r = runtime();
        // The mock seeds two console entries + one request/response pair.
        let logs = call(&get_console_logs(), &r, json!({})).unwrap();
        assert!(logs["count"].as_u64().unwrap() >= 2, "{logs}");
        let msgs: Vec<String> = logs["logs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|l| l["text"].as_str().unwrap_or("").to_string())
            .collect();
        assert!(
            msgs.iter().any(|m| m.contains("hello from mock")),
            "{msgs:?}"
        );
        // `level` filter
        let errs = call(&get_console_logs(), &r, json!({"level": "error"})).unwrap();
        assert_eq!(errs["count"], json!(1), "{errs}");
        assert!(errs["logs"][0]["text"]
            .as_str()
            .unwrap()
            .contains("mock error"));
        // `limit` keeps the latest N
        let one = call(&get_console_logs(), &r, json!({"limit": 1})).unwrap();
        assert_eq!(one["count"], json!(1));
        assert_eq!(one["total"], json!(2));

        let net = call(&get_network_log(), &r, json!({})).unwrap();
        assert!(net["count"].as_u64().unwrap() >= 2, "{net}");
        let types: Vec<String> = net["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["type"].as_str().unwrap_or("").to_string())
            .collect();
        assert!(types.contains(&"request".to_string()), "{types:?}");
        assert!(types.contains(&"response".to_string()), "{types:?}");
    }

    #[test]
    fn logs_survive_drain_events() {
        // Regression: the persistent log must NOT be consumed by drain_events
        // (wait_for_load_state(networkidle) / dialog tools drain the buffer).
        let r = runtime();
        let tab = r.engine().active_tab().unwrap();
        let _ = r.engine().drain_events(tab);
        let logs = call(&get_console_logs(), &r, json!({})).unwrap();
        assert!(logs["count"].as_u64().unwrap() >= 2, "{logs}");
        let net = call(&get_network_log(), &r, json!({})).unwrap();
        assert!(net["count"].as_u64().unwrap() >= 2, "{net}");
    }

    #[test]
    fn page_analysis_tools() {
        let r = runtime();
        let t = call(&get_page_title(), &r, json!({})).unwrap();
        assert_eq!(t["title"], "Example Page");
        let u = call(&get_current_url(), &r, json!({})).unwrap();
        assert_eq!(u["url"], "https://example.com");
        let tx = call(&get_page_text(), &r, json!({})).unwrap();
        assert!(tx["text"].as_str().unwrap().contains("Welcome"));
        let s = call(&screenshot(), &r, json!({})).unwrap();
        assert_eq!(s["format"], "rgba");
        assert!(s["base64"].as_str().unwrap().len() > 16);
        let info = call(&get_element_info(), &r, json!({"id": "c"})).unwrap();
        assert_eq!(info["href"], "https://example.com/about");
    }

    #[test]
    fn element_introspection() {
        let r = runtime();
        let t = call(&get_element_text(), &r, json!({"id": "c"})).unwrap();
        assert_eq!(t["text"], "Learn more");
        let a = call(&get_attributes(), &r, json!({"id": "e"})).unwrap();
        assert_eq!(a["attrs"]["placeholder"], "Search...");
        assert_eq!(
            call(&is_visible(), &r, json!({"id": "a"})).unwrap()["visible"],
            true
        );
        assert_eq!(
            call(&is_enabled(), &r, json!({"id": "a"})).unwrap()["enabled"],
            true
        );
        // JS introspection tools degrade gracefully under mock (no panic).
        let _ = call(&get_focused_element(), &r, json!({})).unwrap();
        let _ = call(&get_selected_text(), &r, json!({})).unwrap();
        let _ = call(&get_page_meta(), &r, json!({})).unwrap();
        let _ = call(&get_performance_metrics(), &r, json!({})).unwrap();
        let sp = call(&get_scroll_position(), &r, json!({})).unwrap();
        assert_eq!(sp["scroll"]["x"].as_f64(), Some(0.0));
        call(&set_scroll_position(), &r, json!({"y": 400})).unwrap();
    }

    #[test]
    fn screenshot_element_crops() {
        let r = runtime();
        let v = call(&screenshot_element(), &r, json!({"id": "c"})).unwrap();
        assert_eq!(v["format"], "rgba");
        assert_eq!(v["width"], 100);
        assert_eq!(v["height"], 30);
    }

    #[test]
    fn accessibility_tree() {
        let r = runtime();
        let v = call(&get_accessibility_tree(), &r, json!({})).unwrap();
        assert!(v["count"].as_u64().unwrap() >= 1);
        // Default view is the hierarchical `root` (mock root role = "webarea").
        assert!(v["root"]["role"].is_string(), "{v}");
    }

    #[test]
    fn accessibility_tree_view_and_geometry_controls() {
        let r = runtime();
        // Default: hierarchical `root`, geometry omitted even though the mock
        // engine attaches geometry to every node.
        let v = call(&get_accessibility_tree(), &r, json!({})).unwrap();
        assert!(v["root"].is_object(), "{v}");
        assert!(
            !serde_json::to_string(&v["root"])
                .unwrap()
                .contains("geometry"),
            "geometry must be omitted by default: {v}"
        );
        // include_geometry=true → geometry present.
        let v = call(
            &get_accessibility_tree(),
            &r,
            json!({"include_geometry": true}),
        )
        .unwrap();
        assert!(
            serde_json::to_string(&v["root"])
                .unwrap()
                .contains("\"geometry\""),
            "{v}"
        );
        // flat view → `nodes` array.
        let v = call(&get_accessibility_tree(), &r, json!({"view": "flat"})).unwrap();
        assert!(v["nodes"].as_array().is_some_and(|a| !a.is_empty()), "{v}");
        // text view → indented string (root role = webarea).
        let v = call(&get_accessibility_tree(), &r, json!({"view": "text"})).unwrap();
        assert!(v["text"].as_str().unwrap().contains("webarea"), "{v}");
        // detail=full adds props/backend_id.
        let v = call(&get_accessibility_tree(), &r, json!({"detail": "full"})).unwrap();
        assert!(
            serde_json::to_string(&v["root"])
                .unwrap()
                .contains("backend_id"),
            "{v}"
        );
    }

    #[test]
    fn accessibility_tree_char_budget_bounds_output() {
        let r = runtime();
        let v = call(&get_accessibility_tree(), &r, json!({"max_chars": 200})).unwrap();
        assert!(
            serde_json::to_string(&v).unwrap().len() <= 400,
            "max_chars must bound the payload: {v}"
        );
    }

    #[test]
    fn filter_ax_node_drops_noise() {
        let tree = json!({
            "role": "", "name": "",
            "children": [
                {"role": "", "name": "", "children": [{"role": "link", "name": "Home"}]},
                {"role": "", "name": "", "children": []},
                {"role": "heading", "name": "Title"}
            ]
        });
        let out = filter_ax_node(&tree).unwrap();
        let kids = out["children"].as_array().unwrap();
        assert_eq!(
            kids.len(),
            2,
            "empty leaf dropped; container + heading kept"
        );
        // The empty container survives (it still has a meaningful child).
        assert_eq!(kids[0]["children"][0]["role"], "link");
        assert_eq!(kids[1]["role"], "heading");
        // A pure-noise subtree collapses to None.
        assert!(filter_ax_node(&json!({"role": "", "name": "", "children": []})).is_none());
    }

    #[test]
    fn count_ax_nodes_counts_tree() {
        let t =
            json!({"role":"root","children":[{"role":"a"},{"role":"b","children":[{"role":"c"}]}]});
        assert_eq!(count_ax_nodes(&t), 4);
    }

    #[test]
    fn prune_ax_node_caps_nested_tree() {
        let tree = json!({
            "role": "root",
            "children": [
                {"role": "a", "children": [{"role": "a1"}, {"role": "a2"}]},
                {"role": "b", "children": [{"role": "b1"}]}
            ]
        });
        let mut budget = 3usize;
        let out = prune_ax_node(&tree, &mut budget).unwrap();
        assert_eq!(budget, 0, "budget fully consumed");
        assert_eq!(out["role"], "root");
        let kids = out["children"].as_array().unwrap();
        assert_eq!(kids.len(), 1, "only first child kept");
        assert_eq!(kids[0]["role"], "a");
        assert_eq!(kids[0]["children"].as_array().unwrap().len(), 1);
        assert_eq!(kids[0]["children"][0]["role"], "a1");
    }

    #[test]
    fn ax_quality_flags_glyph_per_node_trees() {
        // Role-less, one-char names → low quality (counts: total, roleless, char).
        let low = json!({
            "role": "", "name": "T", "children": [
                {"role": "", "name": "h"},
                {"role": "", "name": "i"}
            ]
        });
        let mut acc = (0usize, 0usize, 0usize);
        ax_quality(&low, &mut acc);
        assert_eq!(acc, (3, 3, 3));

        // Semantic tree → roles present, names longer than one char.
        let good = json!({
            "role": "RootWebArea", "name": "Example",
            "children": [{"role": "heading", "name": "Example Domain"}]
        });
        let mut acc2 = (0usize, 0usize, 0usize);
        ax_quality(&good, &mut acc2);
        assert_eq!(acc2.0, 2);
        assert_eq!(acc2.1, 0, "no role-less nodes");
        assert_eq!(acc2.2, 0, "no single-char names");
    }

    #[test]
    fn compact_ax_node_drops_geometry_and_rounds_when_kept() {
        let node = json!({
            "role": "button", "name": "Go", "value": "", "description": "",
            "props": {"x": 1}, "backend_id": 7,
            "geometry": {"x": 1.23456789, "y": 2.987654321, "width": 10.5, "height": 20.0},
            "children": []
        });
        // Default (compact, no geometry): heavy fields dropped entirely.
        let lean = compact_ax_node(&node, false, false, 200);
        let s = serde_json::to_string(&lean).unwrap();
        assert!(!s.contains("geometry"), "{s}");
        assert!(!s.contains("props"), "{s}");
        assert!(!s.contains("backend_id"), "{s}");
        assert_eq!(lean["role"], json!("button"));
        // Geometry kept only when asked, and rounded to 1 decimal.
        let geo = compact_ax_node(&node, true, false, 200);
        assert_eq!(geo["geometry"]["x"], json!(1.2));
        assert_eq!(geo["geometry"]["y"], json!(3.0));
        // Text truncation.
        let long = json!({"role": "text", "name": "z".repeat(50)});
        let cut = compact_ax_node(&long, false, false, 10);
        assert!(cut["name"].as_str().unwrap().chars().count() <= 11);
    }

    #[test]
    fn ax_big_tree_is_bounded_by_default_but_full_available() {
        // Synthetic deep/wide tree with heavy full-precision geometry.
        fn build(depth: usize, breadth: usize) -> Value {
            let kids: Vec<Value> = (0..breadth)
                .map(|i| {
                    if depth == 0 {
                        json!({
                            "role": "StaticText",
                            "name": format!("item {i}"),
                            "geometry": {"x": 1.23456789, "y": 2.3456789, "width": 3.0, "height": 4.0},
                            "children": []
                        })
                    } else {
                        build(depth - 1, breadth)
                    }
                })
                .collect();
            json!({
                "role": "generic", "name": "",
                "geometry": {"x": 9.87654321, "y": 8.7654321, "width": 1.0, "height": 1.0},
                "children": kids
            })
        }
        let tree = build(3, 6); // ~1500 nodes

        // Default pipeline: filter → compact (no geometry) → prune(300) → fit chars.
        let filtered = filter_ax_node(&tree).unwrap_or(Value::Null);
        let mut budget = DEFAULT_AX_NODES;
        let pruned = prune_ax_node(&filtered, &mut budget).unwrap_or(Value::Null);
        let lean = compact_ax_node(&pruned, false, false, DEFAULT_AX_TEXT_LEN);
        let bounded = fit_ax_chars(lean, DEFAULT_AX_CHARS);
        let bounded_s = serde_json::to_string(&bounded).unwrap();
        assert!(
            bounded_s.len() <= DEFAULT_AX_CHARS,
            "default payload must be bounded: {}",
            bounded_s.len()
        );
        assert!(
            !bounded_s.contains("geometry"),
            "geometry omitted by default"
        );

        // A tight char budget must actually shrink the payload.
        let tight = fit_ax_chars(compact_ax_node(&pruned, false, false, 200), 500);
        assert!(
            serde_json::to_string(&tight).unwrap().len() <= 500,
            "char budget honored"
        );

        // Full mode keeps geometry and can be much richer (no functionality loss).
        let full = compact_ax_node(&filtered, true, true, 0);
        let full_s = serde_json::to_string(&full).unwrap();
        assert!(full_s.contains("geometry"));
        assert!(
            full_s.len() > bounded_s.len(),
            "full is richer than default"
        );
    }

    #[test]
    fn clamp_ax_depth_limits_levels() {
        let tree = json!({
            "role": "a",
            "children": [{"role": "b", "children": [{"role": "c", "children": []}]}]
        });
        let clamped = clamp_ax_depth(&tree, 1, 2);
        assert_eq!(clamped["role"], json!("a"));
        assert_eq!(clamped["children"][0]["role"], json!("b"));
        assert!(
            clamped["children"][0].get("children").is_none(),
            "depth 2 keeps no grandchildren"
        );
    }
}
