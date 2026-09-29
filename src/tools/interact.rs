// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Interaction tools: click / dblclick / type / press / hover / drag / scroll / swipe.

use serde_json::json;

use crate::engine::{
    InputEvent, KeyEvent, MouseButton, MouseEvent, MouseKind, Result, TouchEvent, WheelEvent,
};
use crate::tools::tool::{Tool, ToolContext};

pub fn tools() -> Vec<Tool> {
    vec![
        click(),
        dblclick(),
        right_click(),
        type_text(),
        press(),
        hover(),
        drag(),
        scroll(),
        click_coords(),
        swipe(),
        focus(),
        blur(),
        clear_input(),
    ]
}

fn right_click() -> Tool {
    Tool::new(
        "right_click",
        "Right-click an element (by 'selector', 'ref' or snapshot 'id').",
        json!({
            "selector": {"type": "string", "required": false},
            "id": {"type": "string", "required": false},
            "ref": {"type": "string", "required": false}
        }),
        r#"{"selector": ".ctx"}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let r = ctx.element_ref()?;
            ctx.runtime.engine().right_click_element(tab, &r)?;
            Ok(json!({"clicked": r.value, "button": "right", "ok": true}))
        },
    )
}

fn focus() -> Tool {
    Tool::new(
        "focus",
        "Focus an element (by 'selector', 'ref' or snapshot 'id').",
        json!({
            "selector": {"type": "string", "required": false},
            "id": {"type": "string", "required": false},
            "ref": {"type": "string", "required": false}
        }),
        r#"{"selector": "input"}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let r = ctx.element_ref()?;
            let id = snapshot_id_of(ctx, tab, &r)?;
            let v = ctx.eval_opt(tab, &crate::engine::inject::action_js(id, "el.focus()"));
            if v.as_ref().and_then(|x| x.as_str()) == Some("notfound") {
                return Err(crate::engine::EngineError::new(
                    crate::engine::ErrorKind::Dom,
                    "element not found",
                ));
            }
            Ok(json!({"focused": r.value, "ok": true}))
        },
    )
}

fn blur() -> Tool {
    Tool::new(
        "blur",
        "Blur an element (by 'selector', 'ref' or snapshot 'id'), or the currently focused element when no target is given.",
        json!({
            "selector": {"type": "string", "required": false},
            "id": {"type": "string", "required": false},
            "ref": {"type": "string", "required": false}
        }),
        r#"{}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let has_target = ctx.params.get("selector").is_some()
                || ctx.params.get("id").is_some()
                || ctx.params.get("ref").is_some();
            if has_target {
                // Tag the target with the live id, then blur it (falls back to
                // the active element when the target can't be resolved).
                if let Ok(r) = ctx.element_ref() {
                    let _ = ctx.eval_opt(tab, &crate::engine::inject::live_resolve_js(&r));
                    let _ = ctx.eval_opt(
                        tab,
                        &crate::engine::inject::action_js(
                            crate::engine::inject::LIVE_ID,
                            "el.blur()",
                        ),
                    );
                    return Ok(json!({"blurred": true}));
                }
            }
            let _ = ctx.eval_opt(
                tab,
                "document.activeElement?document.activeElement.blur():null",
            );
            Ok(json!({"blurred": true}))
        },
    )
}

fn clear_input() -> Tool {
    Tool::new(
        "clear_input",
        "Clear the value of an input/textarea element (by 'selector', 'ref' or snapshot 'id').",
        json!({
            "selector": {"type": "string", "required": false},
            "id": {"type": "string", "required": false},
            "ref": {"type": "string", "required": false}
        }),
        r#"{"selector": "input"}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let r = ctx.element_ref()?;
            // Use the engine's native set_element_value; mock and real engines share semantics.
            ctx.runtime.engine().set_element_value(tab, &r, "")?;
            Ok(json!({"cleared": r.value, "ok": true}))
        },
    )
}

fn snapshot_id_of(
    ctx: &ToolContext,
    tab: crate::engine::TabId,
    r: &crate::engine::ElementRef,
) -> Result<char> {
    let snap = ctx.runtime.engine().snapshot(tab)?;
    match r.kind {
        crate::engine::RefKind::Snapshot => {
            let c = r
                .value
                .chars()
                .next()
                .ok_or_else(|| crate::engine::EngineError::invalid("bad snapshot id"))?;
            if snap.element_by_id(c).is_none() {
                return Err(crate::engine::EngineError::new(
                    crate::engine::ErrorKind::Dom,
                    format!("element {r} not found"),
                ));
            }
            Ok(c)
        }
        _ => {
            if let Some(id) = snap
                .interactive
                .iter()
                .find(|e| e.matches_ref(r))
                .map(|e| e.id)
            {
                return Ok(id);
            }
            // Live fallback (CSS/XPath/text/role not in the cached snapshot).
            ctx.eval_opt(tab, &crate::engine::inject::live_resolve_js(r))
                .and_then(|v| v.as_str().and_then(|s| s.chars().next()))
                .ok_or_else(|| {
                    crate::engine::EngineError::new(
                        crate::engine::ErrorKind::Dom,
                        format!("element {r} not found"),
                    )
                })
        }
    }
}

fn click() -> Tool {
    Tool::new(
        "click",
        "Click an element. Target by 'selector' (CSS; also `text=`, `role=`, `xpath=`, `//...`), 'ref' (snapshot letter or {kind,value}), or snapshot 'id' (letter a..z).",
        json!({
            "selector": {"type": "string", "required": false, "description": "CSS or Playwright-style selector"},
            "id": {"type": "string", "description": "snapshot letter", "required": false},
            "ref": {"type": "string", "required": false, "description": "snapshot letter (string) or {kind: css|xpath|text|role|snapshot, value}"},
            "element": {"type": "string", "required": false, "description": "human-readable hint (Playwright MCP)"}
        }),
        r#"{"selector": "button.submit"}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let r = ctx.element_ref()?;
            ctx.runtime.engine().click_element(tab, &r)?;
            Ok(json!({"clicked": r.value, "ok": true}))
        },
    )
}

fn dblclick() -> Tool {
    Tool::new(
        "dblclick",
        "Double click an element (by 'selector', 'ref' or snapshot 'id').",
        json!({
            "selector": {"type": "string", "required": false},
            "id": {"type": "string", "required": false},
            "ref": {"type": "string", "required": false}
        }),
        r#"{"id": "b"}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let r = ctx.element_ref()?;
            ctx.runtime.engine().double_click_element(tab, &r)?;
            Ok(json!({"clicked": r.value, "ok": true}))
        },
    )
}

fn type_text() -> Tool {
    Tool::new(
        "type",
        "Type text into an input/textarea (by 'selector', 'ref' or snapshot 'id'). Uses the browser's real keyboard input channel (Input.insertText) so React controlled inputs and IME behave as if typed by a user. Use 'clear' to clear first.",
        json!({
            "selector": {"type": "string", "required": false},
            "id": {"type": "string", "required": false},
            "ref": {"type": "string", "required": false},
            "text": {"type": "string", "required": true},
            "clear": {"type": "boolean", "default": true}
        }),
        r#"{"selector": "input[name=q]", "text": "hello"}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let text = ctx.param_str("text")?;
            let clear = ctx.param_opt::<bool>("clear")?.unwrap_or(true);
            let has_target = ["selector", "id", "ref", "element"]
                .iter()
                .any(|k| ctx.params.get(*k).map(|v| !v.is_null()).unwrap_or(false));
            if has_target {
                let r = ctx.element_ref()?;
                ctx.runtime.engine().type_text(tab, &r, &text, clear)?;
                return Ok(json!({"typed": text, "ok": true}));
            }
            // No target → type into the currently focused element.
            if ctx.runtime.engine().type_text_focused(tab, &text).is_ok() {
                return Ok(json!({"typed": text, "focused": true, "ok": true}));
            }
            let js = crate::engine::inject::set_text_at_point_js(-1.0, -1.0, &text, clear);
            let v = ctx.eval_opt(tab, &js);
            if matches!(v.as_ref().and_then(|x| x.as_str()), Some("ok")) {
                Ok(json!({"typed": text, "focused": true, "ok": true}))
            } else {
                Err(crate::engine::EngineError::invalid(
                    "type: no target given and no focused editable element — pass selector/ref/id, or focus one first",
                ))
            }
        },
    )
}

fn press() -> Tool {
    Tool::new(
        "press",
        "Send a keyboard key (e.g. Enter, Tab, Escape, ArrowDown).",
        json!({"key": {"type": "string", "required": true}}),
        r#"{"key": "Enter"}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let key = ctx.param_str("key")?;
            ctx.runtime
                .engine()
                .inject_event(tab, KeyEvent::press(&key).into())?;
            Ok(json!({"key": key, "ok": true}))
        },
    )
}

fn hover() -> Tool {
    Tool::new(
        "hover",
        "Move the mouse to the center of an element (by 'selector', 'ref' or snapshot 'id').",
        json!({
            "selector": {"type": "string", "required": false},
            "id": {"type": "string", "required": false},
            "ref": {"type": "string", "required": false}
        }),
        r#"{"selector": ".menu"}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let el = ctx.element_snapshot(tab)?;
            let (cx, cy) = el.rect.center();
            ctx.runtime
                .engine()
                .inject_event(tab, MouseEvent::move_to(cx, cy).into())?;
            Ok(json!({"x": cx, "y": cy, "ok": true}))
        },
    )
}

fn drag() -> Tool {
    Tool::new(
        "drag",
        "Drag element 'from' to element 'to' (by snapshot id, ref, or a CSS/Playwright selector).",
        json!({
            "from": {"type": "string", "required": true},
            "to": {"type": "string", "required": true}
        }),
        r#"{"from": "a", "to": "b"}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let from_id = ctx.param_str("from")?;
            let to_id = ctx.param_str("to")?;
            let from =
                ctx.element_snapshot_for(tab, crate::tools::tool::target_from_str(&from_id))?;
            let to = ctx.element_snapshot_for(tab, crate::tools::tool::target_from_str(&to_id))?;
            let (fx, fy) = from.rect.center();
            let (tx, ty) = to.rect.center();
            let eng = ctx.runtime.engine();
            eng.inject_event(
                tab,
                MouseEvent {
                    kind: MouseKind::Down,
                    x: fx,
                    y: fy,
                    button: MouseButton::Left,
                    modifiers: crate::engine::Modifiers::none(),
                    click_count: 1,
                }
                .into(),
            )?;
            eng.inject_event(tab, MouseEvent::move_to(tx, ty).into())?;
            eng.inject_event(
                tab,
                MouseEvent {
                    kind: MouseKind::Up,
                    x: tx,
                    y: ty,
                    button: MouseButton::Left,
                    modifiers: crate::engine::Modifiers::none(),
                    click_count: 1,
                }
                .into(),
            )?;
            Ok(json!({"from": from_id, "to": to_id, "ok": true}))
        },
    )
}

fn click_coords() -> Tool {
    Tool::new(
        "click_coords",
        "Click at absolute viewport coordinates (x, y) in CSS pixels. Used by the preview canvas when the user clicks directly on the page.",
        json!({
            "x": {"type": "number", "required": true},
            "y": {"type": "number", "required": true}
        }),
        r#"{"x": 120, "y": 80}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let x = ctx.param_opt::<f64>("x")?.unwrap_or(0.0);
            let y = ctx.param_opt::<f64>("y")?.unwrap_or(0.0);
            let eng = ctx.runtime.engine();
            eng.inject_event(
                tab,
                MouseEvent {
                    kind: MouseKind::Down,
                    x,
                    y,
                    button: MouseButton::Left,
                    modifiers: crate::engine::Modifiers::none(),
                    click_count: 1,
                }
                .into(),
            )?;
            eng.inject_event(
                tab,
                MouseEvent {
                    kind: MouseKind::Up,
                    x,
                    y,
                    button: MouseButton::Left,
                    modifiers: crate::engine::Modifiers::none(),
                    click_count: 1,
                }
                .into(),
            )?;
            Ok(json!({"x": x, "y": y, "clicked": true, "ok": true}))
        },
    )
}

fn scroll() -> Tool {
    Tool::new(
        "scroll",
        "Scroll the page by dx/dy (aliases: x→dx, y→dy, left→dx, top→dy; CSS pixels, positive = down/right).",
        json!({
            "dx": {"type": "number", "default": 0},
            "dy": {"type": "number", "default": 100},
            "x": {"type": "number", "required": false},
            "y": {"type": "number", "required": false},
            "left": {"type": "number", "required": false},
            "top": {"type": "number", "required": false}
        }),
        r#"{"dy": 200}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let dx = ctx
                .param_opt::<f64>("dx")?
                .or(ctx.param_opt::<f64>("x")?)
                .or(ctx.param_opt::<f64>("left")?)
                .unwrap_or(0.0);
            let dy = ctx
                .param_opt::<f64>("dy")?
                .or(ctx.param_opt::<f64>("y")?)
                .or(ctx.param_opt::<f64>("top")?)
                .unwrap_or(100.0);
            // Prefer a direct relative scroll: on WebView hosts a synthesized
            // wheel event often does not move the page (scrollY stays put),
            // whereas `window.scrollBy` always does. Fall back to wheel
            // injection when JS evaluation is unavailable.
            let js = format!(
                "(function(){{window.scrollBy({dx},{dy});return {{x:window.scrollX||0,y:window.scrollY||0}};}})()"
            );
            if let Some(actual) = ctx.eval_opt(tab, &js) {
                return Ok(json!({"dx": dx, "dy": dy, "ok": true, "actual": actual}));
            }
            let ev = InputEvent::Wheel(WheelEvent {
                x: 0.0,
                y: 0.0,
                delta_x: dx,
                delta_y: dy,
            });
            ctx.runtime.engine().inject_event(tab, ev)?;
            Ok(json!({"dx": dx, "dy": dy, "ok": true}))
        },
    )
}

fn swipe() -> Tool {
    Tool::new(
        "swipe",
        "Swipe from (from_x, from_y) to (to_x, to_y). Useful on touch platforms.",
        json!({
            "from_x": {"type": "number", "required": true},
            "from_y": {"type": "number", "required": true},
            "to_x": {"type": "number", "required": true},
            "to_y": {"type": "number", "required": true}
        }),
        r#"{"from_x": 400, "from_y": 400, "to_x": 400, "to_y": 100}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let fx = ctx.param::<f64>("from_x")?;
            let fy = ctx.param::<f64>("from_y")?;
            let tx = ctx.param::<f64>("to_x")?;
            let ty = ctx.param::<f64>("to_y")?;
            ctx.runtime
                .engine()
                .inject_event(tab, TouchEvent::swipe((fx, fy), (tx, ty)).into())?;
            Ok(json!({"ok": true}))
        },
    )
}

#[allow(dead_code)]
fn snapshot_by_id(
    ctx: &ToolContext,
    tab: crate::engine::TabId,
    id: &str,
) -> Result<crate::engine::InteractiveElement> {
    let c = id
        .chars()
        .next()
        .ok_or_else(|| crate::engine::EngineError::invalid("empty id"))?;
    let snap = ctx.runtime.engine().snapshot(tab)?;
    snap.element_by_id(c).cloned().ok_or_else(|| {
        crate::engine::EngineError::new(
            crate::engine::ErrorKind::Dom,
            format!("element '{id}' not found"),
        )
    })
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
    fn scroll_accepts_x_y_aliases() {
        let r = runtime();
        let v = call(&scroll(), &r, json!({"y": 50}));
        assert_eq!(v["dy"], 50.0);
        assert_eq!(v["dx"], 0.0);
        let v = call(&scroll(), &r, json!({"x": 10, "y": 20}));
        assert_eq!(v["dx"], 10.0);
        assert_eq!(v["dy"], 20.0);
        // dx/dy still work and take precedence.
        let v = call(&scroll(), &r, json!({"dx": 5, "dy": 7, "y": 99}));
        assert_eq!(v["dx"], 5.0);
        assert_eq!(v["dy"], 7.0);
    }

    #[test]
    fn type_into_input() {
        let r = runtime();
        call(&type_text(), &r, json!({"id": "e", "text": "rust"}));
        let snap = {
            let t = r.engine().active_tab().unwrap();
            r.engine().snapshot(t).unwrap()
        };
        let el = snap.element_by_id('e').unwrap();
        assert_eq!(el.value.as_deref(), Some("rust"));
    }

    #[test]
    fn type_append_without_clear() {
        let r = runtime();
        call(&type_text(), &r, json!({"id": "e", "text": "ab"}));
        call(
            &type_text(),
            &r,
            json!({"id": "e", "text": "cd", "clear": false}),
        );
        let tab = r.engine().active_tab().unwrap();
        let snap = r.engine().snapshot(tab).unwrap();
        assert_eq!(
            snap.element_by_id('e').unwrap().value.as_deref(),
            Some("abcd")
        );
    }

    #[test]
    fn click_link_navigates() {
        let r = runtime();
        let out = call(&click(), &r, json!({"id": "c"}));
        assert_eq!(out["clicked"], "c");
        let snap = {
            let t = r.engine().active_tab().unwrap();
            r.engine().snapshot(t).unwrap()
        };
        assert_eq!(snap.url, "https://example.com/about");
    }

    #[test]
    fn press_and_scroll_ok() {
        let r = runtime();
        call(&press(), &r, json!({"key": "Enter"}));
        call(&scroll(), &r, json!({"dy": 120}));
        call(&hover(), &r, json!({"id": "d"}));
        call(&dblclick(), &r, json!({"id": "c"}));
    }

    #[test]
    fn swipe_ok() {
        let r = runtime();
        call(
            &swipe(),
            &r,
            json!({"from_x": 100.0, "from_y": 200.0, "to_x": 100.0, "to_y": 50.0}),
        );
    }

    #[test]
    fn drag_between_elements() {
        let r = runtime();
        call(&drag(), &r, json!({"from": "d", "to": "a"}));
    }

    #[test]
    fn click_coords_dispatches_at_coordinates() {
        let r = runtime();
        let out = call(&click_coords(), &r, json!({"x": 120, "y": 80}));
        assert_eq!(out["clicked"], true);
        assert_eq!(out["x"].as_f64(), Some(120.0));
        assert_eq!(out["y"].as_f64(), Some(80.0));
        // The tool must be part of the registry (used by preview canvas clicks).
        let names: Vec<String> = r
            .tool_list()
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|t| t.get("name").and_then(Value::as_str).map(String::from))
            .collect();
        assert!(
            names.contains(&"click_coords".to_string()),
            "names: {names:?}"
        );
    }

    #[test]
    fn extended_interactions() {
        let r = runtime();
        let v = call(&right_click(), &r, json!({"id": "d"}));
        assert_eq!(v["button"], "right");
        call(&focus(), &r, json!({"id": "e"}));
        call(&blur(), &r, json!({}));
        call(&type_text(), &r, json!({"id": "e", "text": "hello"}));
        call(&clear_input(), &r, json!({"id": "e"}));
        let tab = r.engine().active_tab().unwrap();
        let snap = r.engine().snapshot(tab).unwrap();
        assert_eq!(snap.element_by_id('e').unwrap().value.as_deref(), Some(""));
    }
}
