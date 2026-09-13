// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Form tools: fill_form / select_option / upload_file / checkbox / radio.

use serde_json::{json, Value};

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
                        "id": e.id.to_string(),
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
        "Fill multiple fields at once. 'values' maps snapshot ids to text, e.g. {\"a\":\"alice\",\"b\":\"pw\"}.",
        json!({"values": {"type": "object", "required": true}}),
        r#"{"values": {"a": "alice", "b": "secret"}}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let values: serde_json::Map<String, Value> = ctx.param("values")?;
            let mut filled = Vec::new();
            for (k, v) in values {
                let c = k.chars().next().unwrap_or('?');
                let text = v.as_str().ok_or_else(|| {
                    crate::engine::EngineError::new(
                        crate::engine::ErrorKind::InvalidArgument,
                        format!("fill_form value for '{k}' must be a string, got {v:?}"),
                    )
                })?;
                ctx.runtime
                    .engine()
                    .set_element_value(tab, &crate::engine::ElementRef::snapshot(c), text)?;
                filled.push(k);
            }
            Ok(json!({"filled": filled, "ok": true}))
        },
    )
}

fn select_option() -> Tool {
    Tool::new(
        "select_option",
        "Select an <option> value from a <select> element (by 'id' or 'ref').",
        json!({
            "id": {"type": "string", "required": false},
            "value": {"type": "string", "required": true}
        }),
        r#"{"id": "e", "value": "pro"}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let value = ctx.param_str("value")?;
            let r = ctx.element_ref()?;
            ctx.runtime.engine().select_option(tab, &r, &value)?;
            Ok(json!({"selected": value, "ok": true}))
        },
    )
}

fn upload_file() -> Tool {
    Tool::new(
        "upload_file",
        "Set file(s) on an <input type=file> element (by 'id' or 'ref').",
        json!({
            "id": {"type": "string", "required": false},
            "paths": {"type": "array", "items": {"type": "string"}, "required": true}
        }),
        r#"{"id": "f", "paths": ["/tmp/a.pdf"]}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let paths: Vec<String> = ctx.param("paths")?;
            let r = ctx.element_ref()?;
            ctx.runtime.engine().set_file_input(tab, &r, &paths)?;
            Ok(json!({"files": paths, "ok": true}))
        },
    )
}

fn checkbox() -> Tool {
    Tool::new(
        "checkbox",
        "Set a checkbox's checked state (by 'id' or 'ref').",
        json!({
            "id": {"type": "string", "required": false},
            "checked": {"type": "boolean", "default": true}
        }),
        r#"{"id": "d", "checked": true}"#,
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
        "Select a radio button (by 'id' or 'ref').",
        json!({"id": {"type": "string", "required": false}}),
        r#"{"id": "d"}"#,
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
