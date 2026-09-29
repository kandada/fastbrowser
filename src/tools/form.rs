// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Form tools: fill_form / select_option / upload_file / checkbox / radio.

use serde_json::{json, Value};

use crate::engine::Capability;
use crate::tools::tool::Tool;

pub fn tools() -> Vec<Tool> {
    vec![
        fill_form(),
        select_option(),
        upload_file(),
        checkbox(),
        radio(),
        extract_forms(),
    ]
}

fn extract_forms() -> Tool {
    Tool::new(
        "extract_forms",
        "List all form controls (inputs/selects/textareas) with name, type, value and state.",
        json!({}),
        r#"{}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let snap = ctx.runtime.engine().snapshot(tab)?;
            let controls: Vec<Value> = snap
                .interactive
                .iter()
                .filter(|e| matches!(e.tag.as_str(), "input" | "select" | "textarea" | "button"))
                .map(|e| {
                    json!({
                        "id": e.display_id(),
                        "dom_id": e.attrs.get("id").cloned(),
                        "tag": e.tag,
                        "name": e.attrs.get("name").cloned().unwrap_or_default(),
                        "input_type": e.input_type,
                        "value": e.value,
                        "checked": e.checked,
                        "options": e.selectable_options,
                        "selected": e.selected_option,
                        "placeholder": e.attrs.get("placeholder").cloned(),
                    })
                })
                .collect();
            Ok(json!({"forms": controls, "count": controls.len()}))
        },
    )
}

fn fill_form() -> Tool {
    Tool::new(
        "fill_form",
        "Fill multiple fields at once. 'values' maps each field to text. Keys may be snapshot ids (single letter, e.g. {\"a\":\"alice\"}), CSS/Playwright selectors ({\"#name\":\"alice\"}), or a field name/`[name=...]` ({\"username\":\"alice\"}); snapshot ids only resolve right after a snapshot() — prefer selectors for stability.",
        json!({"values": {"type": "object", "required": true}}),
        r#"{"values": {"a": "alice", "b": "secret"}}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let values: serde_json::Map<String, Value> = ctx.param("values")?;
            let mut filled = Vec::new();
            for (k, v) in values {
                let text = v.as_str().ok_or_else(|| {
                    crate::engine::EngineError::new(
                        crate::engine::ErrorKind::InvalidArgument,
                        format!("fill_form value for '{k}' must be a string, got {}", v),
                    )
                })?;
                let r = fill_target(&k);
                ctx.runtime.engine().set_element_value(tab, &r, text)?;
                filled.push(k);
            }
            Ok(json!({"filled": filled, "ok": true}))
        },
    )
}

/// Resolve a `fill_form` key to an element target. A single letter is a snapshot
/// id (back-compat); anything else is treated as an explicit selector, or a
/// field name matched via `#name` / `[name="name"]`.
fn fill_target(k: &str) -> crate::engine::ElementRef {
    use crate::engine::ElementRef;
    let mut it = k.chars();
    if let (Some(c), None) = (it.next(), it.next()) {
        if c.is_ascii_alphabetic() {
            return ElementRef::snapshot(c);
        }
    }
    if let Some(rest) = k.strip_prefix("css=") {
        return ElementRef::css(rest);
    }
    if let Some(rest) = k.strip_prefix("text=") {
        return ElementRef::text(rest);
    }
    if let Some(rest) = k.strip_prefix("role=") {
        return ElementRef::role(rest);
    }
    if k.starts_with('#')
        || k.starts_with('.')
        || k.starts_with('[')
        || k.starts_with("//")
        || k.contains(">>")
    {
        return ElementRef::selector(k);
    }
    ElementRef::selector(format!("#{k}, [name=\"{k}\"]"))
}

fn select_option() -> Tool {
    Tool::new(
        "select_option",
        "Select an <option> from a <select> (by 'selector', 'ref' or snapshot 'id'). Provide 'value' (the option value, e.g. \"g\"), 'label' (the visible text, e.g. \"Green\"), or 'values' (array, first used) — Playwright MCP sends 'values'.",
        json!({
            "selector": {"type": "string", "required": false},
            "id": {"type": "string", "required": false},
            "ref": {"type": "string", "required": false},
            "value": {"type": "string", "required": false},
            "label": {"type": "string", "required": false},
            "values": {"type": "array", "items": {"type": "string"}, "required": false}
        }),
        r#"{"selector": "select#lang", "value": "pro"}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let value = match ctx.params.get("value").and_then(|v| v.as_str()) {
                Some(v) if !v.is_empty() => v.to_string(),
                _ => ctx
                    .params
                    .get("values")
                    .and_then(|v| v.as_array())
                    .and_then(|a| a.first())
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
                    .or_else(|| {
                        ctx.params
                            .get("label")
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.is_empty())
                            .map(|s| s.to_string())
                    })
                    .ok_or_else(|| {
                        crate::engine::EngineError::invalid("need 'value', 'label' or 'values'")
                    })?,
            };
            let r = ctx.element_ref()?;
            ctx.runtime.engine().select_option(tab, &r, &value)?;
            Ok(json!({"selected": value, "ok": true}))
        },
    )
}

fn upload_file() -> Tool {
    Tool::new(
        "upload_file",
        "Set file(s) on an <input type=file> element (by 'selector', 'ref' or snapshot 'id').",
        json!({
            "selector": {"type": "string", "required": false},
            "id": {"type": "string", "required": false},
            "ref": {"type": "string", "required": false},
            "paths": {"type": "array", "items": {"type": "string"}, "required": true}
        }),
        r#"{"selector": "input[type=file]", "paths": ["/tmp/a.pdf"]}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let paths: Vec<String> = ctx.param("paths")?;
            let r = ctx.element_ref()?;
            ctx.runtime.engine().set_file_input(tab, &r, &paths)?;
            Ok(json!({"files": paths, "ok": true}))
        },
    )
    // 文件选择输入需要宿主支持（webview 未实现 → 隐藏）。
    .requires(&[Capability::FileInput])
}

fn checkbox() -> Tool {
    Tool::new(
        "checkbox",
        "Set a checkbox's checked state (by 'selector', 'ref' or snapshot 'id').",
        json!({
            "selector": {"type": "string", "required": false},
            "id": {"type": "string", "required": false},
            "ref": {"type": "string", "required": false},
            "checked": {"type": "boolean", "default": true}
        }),
        r#"{"selector": "input[type=checkbox]", "checked": true}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let checked = ctx.param_opt::<bool>("checked")?.unwrap_or(true);
            let r = ctx.element_ref()?;
            ctx.runtime.engine().check_element(tab, &r, checked)?;
            Ok(json!({"checked": checked, "ok": true}))
        },
    )
}

fn radio() -> Tool {
    Tool::new(
        "radio",
        "Select a radio button (by 'selector', 'ref' or snapshot 'id').",
        json!({
            "selector": {"type": "string", "required": false},
            "id": {"type": "string", "required": false},
            "ref": {"type": "string", "required": false}
        }),
        r#"{"selector": "input[type=radio]"}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let r = ctx.element_ref()?;
            ctx.runtime.engine().check_element(tab, &r, true)?;
            Ok(json!({"checked": true, "ok": true}))
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
            .create_tab("https://example.com/login", &TabOptions::default())
            .unwrap();
        r
    }

    fn call(tool: &Tool, r: &Runtime, params: Value) -> Result<Value> {
        tool.run(&ToolContext { runtime: r, params })
    }

    #[test]
    fn fill_target_resolves_kinds() {
        use crate::engine::RefKind;
        assert_eq!(fill_target("a").kind, RefKind::Snapshot);
        assert_eq!(fill_target("#x").kind, RefKind::Selector);
        assert_eq!(fill_target("css=#y").kind, RefKind::Css);
        let by_name = fill_target("username");
        assert_eq!(by_name.kind, RefKind::Selector);
        assert!(
            by_name.value.contains("[name=\"username\"]"),
            "{}",
            by_name.value
        );
    }

    #[test]
    fn fill_form_sets_values() {
        let r = runtime();
        let v = call(
            &fill_form(),
            &r,
            json!({"values": {"a": "alice", "b": "secret"}}),
        )
        .unwrap();
        assert_eq!(v["filled"].as_array().unwrap().len(), 2);
        let snap = {
            let t = r.engine().active_tab().unwrap();
            r.engine().snapshot(t).unwrap()
        };
        assert_eq!(
            snap.element_by_id('a').unwrap().value.as_deref(),
            Some("alice")
        );
        assert_eq!(
            snap.element_by_id('b').unwrap().value.as_deref(),
            Some("secret")
        );
    }

    #[test]
    fn select_and_checkbox() {
        let r = runtime();
        let v = call(&select_option(), &r, json!({"id": "e", "value": "pro"})).unwrap();
        assert_eq!(v["selected"], "pro");
        let _ = call(&checkbox(), &r, json!({"id": "d", "checked": true})).unwrap();
        let _ = call(&radio(), &r, json!({"id": "d"})).unwrap();
        let snap = {
            let t = r.engine().active_tab().unwrap();
            r.engine().snapshot(t).unwrap()
        };
        assert_eq!(
            snap.element_by_id('e').unwrap().selected_option.as_deref(),
            Some("pro")
        );
        assert_eq!(snap.element_by_id('d').unwrap().checked, Some(true));
    }

    #[test]
    fn select_option_accepts_label() {
        let r = runtime();
        // `label` is honored (previously the global label→value alias silently
        // remapped it, and selecting a non-existent value reported false success).
        let v = call(&select_option(), &r, json!({"id": "e", "label": "pro"})).unwrap();
        assert_eq!(v["selected"], "pro");
    }

    #[test]
    fn upload_file_tool() {
        let r = runtime();
        let v = call(
            &upload_file(),
            &r,
            json!({"id": "a", "paths": ["/tmp/x.pdf"]}),
        )
        .unwrap();
        assert_eq!(v["files"][0], "/tmp/x.pdf");
    }

    #[test]
    fn extract_forms_lists_controls() {
        let r = runtime();
        let v = call(&extract_forms(), &r, json!({})).unwrap();
        let controls = v["forms"].as_array().unwrap();
        assert!(controls.len() >= 3);
        let names: Vec<&str> = controls.iter().filter_map(|c| c["name"].as_str()).collect();
        assert!(names.contains(&"username"));
        assert!(names.contains(&"password"));
        assert!(controls.iter().any(|c| c["tag"] == "select"));
    }
}
