// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Schema sanity: every tool's spec must be well-formed so the model (and every
//! consumer) can rely on it. Catches drift when tools are added/edited:
//! unique names, non-empty descriptions, typed params, and an example that is
//! valid JSON whose keys are all declared.

use fastbrowser::sdk::Fastbrowser;
use fastbrowser::Config;
use serde_json::Value;
use std::collections::HashSet;

fn sdk() -> Fastbrowser {
    let s = Fastbrowser::new();
    s.init(Config::default()).unwrap();
    s.open("https://example.com/login").unwrap();
    s
}

#[test]
fn tool_schemas_are_well_formed() {
    let s = sdk();
    let list = s.tool_list();
    let arr = list.as_array().expect("tool_list is an array");
    assert!(!arr.is_empty(), "no tools registered");

    let mut seen: HashSet<String> = HashSet::new();
    let mut problems: Vec<String> = Vec::new();

    for t in arr {
        let name = t["name"].as_str().unwrap_or("<noname>").to_string();
        if !seen.insert(name.clone()) {
            problems.push(format!("duplicate tool name '{name}'"));
        }
        if t["description"].as_str().unwrap_or("").trim().is_empty() {
            problems.push(format!("{name}: empty description"));
        }

        let params = t["params"].as_object();
        if let Some(params) = params {
            for (p, def) in params {
                if !def.is_object() {
                    problems.push(format!("{name}.{p}: param def is not an object"));
                    continue;
                }
                if def.get("type").is_none() {
                    problems.push(format!("{name}.{p}: missing 'type'"));
                }
                if let Some(enum_vals) = def.get("enum").and_then(|e| e.as_array()) {
                    if enum_vals.is_empty() {
                        problems.push(format!("{name}.{p}: empty enum"));
                    }
                }
            }
        }

        // Example must be valid JSON and only reference declared params.
        let ex = t["example"].as_str().unwrap_or("");
        if !ex.trim().is_empty() {
            match serde_json::from_str::<Value>(ex) {
                Ok(v) => {
                    if let (Some(obj), Some(params)) = (v.as_object(), params) {
                        for k in obj.keys() {
                            if !params.contains_key(k) && k != "tab" {
                                problems.push(format!(
                                    "{name}: example key '{k}' is not a declared param"
                                ));
                            }
                        }
                    }
                }
                Err(e) => problems.push(format!("{name}: example is not valid JSON: {e}")),
            }
        }

        // The generated JSON Schema must be an object schema.
        let schema = &t["schema"];
        assert_eq!(
            schema["type"], "object",
            "{name}: schema.type must be 'object'"
        );
        // Required params (declared) must appear in the schema's required list.
        if let Some(params) = params {
            let required: Vec<&String> = params
                .iter()
                .filter(|(_, d)| d["required"].as_bool().unwrap_or(false))
                .map(|(k, _)| k)
                .collect();
            let schema_required: Vec<String> = schema["required"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            for r in required {
                if !schema_required.contains(r) {
                    problems.push(format!(
                        "{name}: required param '{r}' missing from schema.required"
                    ));
                }
            }
        }
    }

    assert!(
        problems.is_empty(),
        "schema problems ({}):\n{}",
        problems.len(),
        problems.join("\n")
    );
}
