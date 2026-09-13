// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Agent helper tools: done (task finalization) / send_keys (key combos).

use serde_json::json;

use crate::engine::{KeyEvent, KeyKind, Modifiers};
use crate::tools::tool::Tool;

pub fn tools() -> Vec<Tool> {
    vec![done(), send_keys(), export_replay()]
}

fn export_replay() -> Tool {
    Tool::new(
        "export_replay",
        "Export the action audit trail as a replayable Python script (successful actions only; sensitive values are redacted).",
        json!({}),
        r#"{}"#,
        |ctx| {
            let script = ctx.runtime.audit_script();
            Ok(json!({"script": script}))
        },
    )
}

fn done() -> Tool {
    Tool::new(
        "done",
        "Signal the end of a task and return the final answer to the user. Use this when the task is complete.",
        json!({
            "answer": {"type": "string", "description": "Final answer summarizing what was done", "required": true}
        }),
        r#"{"answer": "Compared prices: the lowest was Store A at $199."}"#,
        |ctx| {
            let answer = ctx.param_str("answer")?;
            Ok(json!({"done": true, "answer": answer}))
        },
    )
}

fn send_keys() -> Tool {
    Tool::new(
        "send_keys",
        "Send a key or a key-combo to the active tab. Supports modifiers: Ctrl, Alt, Shift, Meta (e.g. \"Ctrl+a\", \"Shift+Enter\", \"Enter\").",
        json!({"keys": {"type": "string", "description": "Key or combo, e.g. \"Ctrl+a\"", "required": true}}),
        r#"{"keys": "Ctrl+a"}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let keys = ctx.param_str("keys")?;
            dispatch_combo(ctx, tab, &keys)?;
            Ok(json!({"keys": keys, "ok": true}))
        },
    )
}

/// Split "Ctrl+a" / "Shift+Enter" / "Enter" into down/up event sequences and dispatch them.
fn dispatch_combo(
    ctx: &crate::tools::tool::ToolContext<'_>,
    tab: crate::engine::TabId,
    combo: &str,
) -> crate::engine::Result<()> {
    let parts: Vec<&str> = combo.split('+').map(str::trim).collect();
    let (mods, key) = parse_parts(&parts);
    let code = key;
    let eng = ctx.runtime.engine();
    // Press modifier keys (down order).
    for m in modifier_down(&mods) {
        eng.inject_event(
            tab,
            KeyEvent {
                kind: KeyKind::Down,
                key: m.0.to_string(),
                code: m.1.to_string(),
                modifiers: Modifiers::none(),
                text: String::new(),
            }
            .into(),
        )?;
    }
    // Main key down.
    eng.inject_event(
        tab,
        KeyEvent {
            kind: KeyKind::Down,
            key: key.to_string(),
            code: code.to_string(),
            modifiers: mods,
            text: String::new(),
        }
        .into(),
    )?;
    // Main key press (for text input).
    eng.inject_event(
        tab,
        KeyEvent {
            kind: KeyKind::Press,
            key: key.to_string(),
            code: code.to_string(),
            modifiers: mods,
            text: printable(key).map(|c| c.to_string()).unwrap_or_default(),
        }
        .into(),
    )?;
    // Main key up.
    eng.inject_event(
        tab,
        KeyEvent {
            kind: KeyKind::Up,
            key: key.to_string(),
            code: code.to_string(),
            modifiers: mods,
            text: String::new(),
        }
        .into(),
    )?;
    // Release modifier keys (reverse order).
    for m in modifier_up(&mods) {
        eng.inject_event(
            tab,
            KeyEvent {
                kind: KeyKind::Up,
                key: m.0.to_string(),
                code: m.1.to_string(),
                modifiers: Modifiers::none(),
                text: String::new(),
            }
            .into(),
        )?;
    }
    Ok(())
}

fn parse_parts<'a>(parts: &[&'a str]) -> (Modifiers, &'a str) {
    let mut mods = Modifiers::none();
    let mut key = parts.last().copied().unwrap_or("");
    for p in &parts[..parts.len().saturating_sub(1)] {
        match *p {
            "Ctrl" | "Control" | "ctrl" => mods.ctrl = true,
            "Alt" | "alt" | "Option" => mods.alt = true,
            "Shift" | "shift" => mods.shift = true,
            "Meta" | "Cmd" | "Command" | "Super" | "meta" => mods.meta = true,
            other => key = other,
        }
    }
    (mods, key)
}

/// Modifier key press order (CDP convention: Alt, Ctrl, Meta, Shift).
fn modifier_down(mods: &Modifiers) -> Vec<(&'static str, &'static str)> {
    let mut v = Vec::new();
    if mods.alt {
        v.push(("Alt", "AltLeft"));
    }
    if mods.ctrl {
        v.push(("Control", "ControlLeft"));
    }
    if mods.meta {
        v.push(("Meta", "MetaLeft"));
    }
    if mods.shift {
        v.push(("Shift", "ShiftLeft"));
    }
    v
}

/// Modifier key release order (reverse).
fn modifier_up(mods: &Modifiers) -> Vec<(&'static str, &'static str)> {
    let mut v = modifier_down(mods);
    v.reverse();
    v
}

fn printable(key: &str) -> Option<char> {
    let mut chars = key.chars();
    let c = chars.next()?;
    // Multi-character key names (Enter/Tab/ArrowDown, …) are not text input.
    if chars.next().is_some() {
        return None;
    }
    if c.is_alphanumeric() || c.is_ascii_punctuation() {
        Some(c)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_combo_parts() {
        let (m, k) = parse_parts(&["Ctrl", "Shift", "a"]);
        assert!(m.ctrl && m.shift && !m.alt);
        assert_eq!(k, "a");
        let (m, k) = parse_parts(&["Enter"]);
        assert!(!m.ctrl && !m.shift);
        assert_eq!(k, "Enter");
        let (m, k) = parse_parts(&["Meta", "l"]);
        assert!(m.meta);
        assert_eq!(k, "l");
    }

    #[test]
    fn modifier_order() {
        let m = Modifiers {
            ctrl: true,
            shift: true,
            ..Default::default()
        };
        let d = modifier_down(&m);
        assert_eq!(d.len(), 2);
        let u = modifier_up(&m);
        assert_eq!(u[0].0, "Shift");
        assert_eq!(u[1].0, "Control");
    }

    #[test]
    fn printable_keys() {
        assert_eq!(printable("a"), Some('a'));
        assert_eq!(printable("Enter"), None);
        assert_eq!(printable("!"), Some('!'));
    }
}
