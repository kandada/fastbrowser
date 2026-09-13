// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Page-analysis tools: screenshot / get_page_title / get_current_url / get_page_text / get_element_info.

use serde_json::{json, Value};

use crate::engine::Capability;
use crate::tools::tool::Tool;

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
    .requires(&[Capability::Cdp])
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
        "Return the visible text of the whole page.",
        json!({}),
        r#"{}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let text = ctx.runtime.engine().get_page_text(tab)?;
            Ok(json!({"text": text}))
        },
    )
}

fn screenshot_element() -> Tool {
    Tool::new(
        "screenshot_element",
        "Capture a screenshot cropped to one element (by 'id' or 'ref'). Returns base64 RGBA.",
        json!({"id": {"type": "string", "required": false}}),
        r#"{"id": "c"}"#,
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
    .requires(&[Capability::Cdp])
}

fn get_element_text() -> Tool {
    Tool::new(
        "get_element_text",
        "Return the visible text of one element (by 'id' or 'ref').",
        json!({"id": {"type": "string", "required": false}}),
        r#"{"id": "c"}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let el = ctx.element_snapshot(tab)?;
            Ok(json!({"id": el.id.to_string(), "text": el.text.unwrap_or_default()}))
        },
    )
}

fn get_attributes() -> Tool {
    Tool::new(
        "get_attributes",
        "Return the attributes of one element (by 'id' or 'ref').",
        json!({"id": {"type": "string", "required": false}}),
        r#"{"id": "a"}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let el = ctx.element_snapshot(tab)?;
            Ok(json!({"id": el.id.to_string(), "attrs": el.attrs}))
        },
    )
}

fn is_visible() -> Tool {
    Tool::new(
        "is_visible",
        "Check whether an element (by 'id' or 'ref') is visible.",
        json!({"id": {"type": "string", "required": false}}),
        r#"{"id": "a"}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let el = ctx.element_snapshot(tab)?;
            Ok(json!({"id": el.id.to_string(), "visible": el.visible}))
        },
    )
}

fn is_enabled() -> Tool {
    Tool::new(
        "is_enabled",
        "Check whether an element (by 'id' or 'ref') is visible and not disabled.",
        json!({"id": {"type": "string", "required": false}}),
        r#"{"id": "a"}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let el = ctx.element_snapshot(tab)?;
            let disabled = el.attrs.contains_key("disabled")
                || el
                    .attrs
                    .get("aria-disabled")
                    .map(|v| v == "true")
                    .unwrap_or(false);
            Ok(json!({"id": el.id.to_string(), "enabled": el.visible && !disabled}))
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
        "Scroll to absolute (x, y).",
        json!({"x": {"type": "number", "default": 0}, "y": {"type": "number", "default": 0}}),
        r#"{"y": 400}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let x = ctx.param_opt::<f64>("x")?.unwrap_or(0.0);
            let y = ctx.param_opt::<f64>("y")?.unwrap_or(0.0);
            let _ = ctx.eval_opt(tab, &format!("window.scrollTo({x},{y})"));
            Ok(json!({"x": x, "y": y, "ok": true}))
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

fn get_accessibility_tree() -> Tool {
    Tool::new(
        "get_accessibility_tree",
        "Return an accessibility tree of the page (role/name/state), the LLM-native perception format. Falls back to interactive elements when the engine lacks AX support.",
        json!({}),
        r#"{}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            // Prefer the engine's AX tree (CDP Accessibility.getFullAXTree).
            if let Ok(v) = ctx.runtime.engine().accessibility_tree(tab) {
                return Ok(v);
            }
            // Fallback: interactive elements from the snapshot.
            let snap = ctx.runtime.engine().snapshot(tab)?;
            let nodes: Vec<Value> = snap
                .interactive
                .iter()
                .map(|e| {
                    json!({
                        "id": e.id.to_string(),
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
            Ok(json!({"tree": nodes, "count": nodes.len(), "fallback": true}))
        },
    )
}

fn get_element_info() -> Tool {
    Tool::new(
        "get_element_info",
        "Return detailed info about one element (by 'id' or 'ref'): tag, role, text, rect, attrs, value...",
        json!({"id": {"type": "string", "required": false}}),
        r#"{"id": "a"}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let el = ctx.element_snapshot(tab)?;
            Ok(json!({
                "id": el.id.to_string(),
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
        assert!(v["tree"][0]["role"].is_string());
    }
}
