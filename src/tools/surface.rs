// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! 通用「无障碍表面」工具（feature `surface`）。
//!
//! 以 `ax_` 前缀暴露 Accessibility API（macOS AXUIElement / Windows UIA /
//! Linux AT-SPI）。与浏览器专有工具（navigate/click/cookies…）并列：这些工具对
//! web 与原生表面一视同仁，按 `ref` 命名空间路由到对应 provider。Agent 因此可以
//! 用同一套感知/动作语义跨表面组合操作。
//!
//! 方言支持：`ref` 既接受完整命名空间引用（`web:3:a` / `desktop:1234:0.2.1`），
//! 也接受裸目标（`a` / `#go` / `text=Go` / `role=button` / `//button`）——裸目标会
//! 自动绑定到活动/首个表面；还可用 `selector` / `id` / `element` 参数（与浏览器
//! 专有工具同义），由 `engine::snapshot::parse_selector_dialect` 统一解析。

use serde_json::{json, Value};

use crate::engine::{
    Capability, EngineError, InputEvent, MouseButton, MouseEvent, SnapshotOptions, SurfaceAction,
    SurfaceKind, SurfaceSnapshot,
};
use crate::tools::tool::{Tool, ToolContext};

fn surface<'a>(ctx: &ToolContext<'a>) -> crate::engine::Result<&'a crate::engine::SurfaceRuntime> {
    ctx.runtime.surface().ok_or_else(|| {
        EngineError::unsupported(
            "surface layer not available (compile with feature `surface` and enable Config.surface)",
        )
    })
}

fn snapshot_options(ctx: &ToolContext<'_>) -> SnapshotOptions {
    let mut opts = SnapshotOptions::default();
    {
        let cfg = ctx.runtime.config();
        if cfg.surface.max_nodes > 0 {
            opts.max_nodes = cfg.surface.max_nodes;
        }
        if cfg.surface.max_depth > 0 {
            opts.max_depth = cfg.surface.max_depth;
        }
        opts.interesting_only = cfg.surface.interesting_only;
    }
    if let Ok(Some(v)) = ctx.param_opt::<usize>("max_nodes") {
        if v > 0 {
            opts.max_nodes = v;
        }
    }
    if let Ok(Some(v)) = ctx.param_opt::<usize>("max_depth") {
        if v > 0 {
            opts.max_depth = v;
        }
    }
    if let Ok(Some(v)) = ctx.param_opt::<bool>("interesting_only") {
        opts.interesting_only = v;
    }
    if let Ok(Some(v)) = ctx.param_opt::<usize>("max_text_len") {
        opts.max_text_len = v;
    }
    opts
}

fn snapshot_json(snap: &SurfaceSnapshot) -> Value {
    json!({
        "surface": snap.surface,
        "generation": snap.generation,
        "meta": snap.meta,
        "tree": snap.root,
        "text": snap.to_llm_text(),
    })
}

/// 解析目标表面 id：显式 `target` 优先；否则活动表面 → web 表面 → 首个。
fn surface_id_for(
    rt: &crate::engine::SurfaceRuntime,
    target: &str,
) -> crate::engine::Result<String> {
    let t = target.trim();
    if !t.is_empty() && t.contains(':') {
        return Ok(t.to_string());
    }
    let surfaces = rt.list_surfaces_blocking()?;
    surfaces
        .iter()
        .find(|s| s.active)
        .or_else(|| surfaces.iter().find(|s| s.kind == SurfaceKind::Web))
        .or_else(|| surfaces.first())
        .map(|s| s.id.clone())
        .ok_or_else(|| EngineError::unsupported("no surface available"))
}

/// 把裸局部目标绑定到某表面，得到完整 ref。
fn qualify(
    rt: &crate::engine::SurfaceRuntime,
    target: &str,
    local: &str,
) -> crate::engine::Result<String> {
    let sid = surface_id_for(rt, target)?;
    Ok(format!("{sid}:{local}"))
}

/// 把 `id` 参数归一化为局部目标（单字母 → 快照；`#x`/`.x`/`[..]` → CSS；其余 → `#id`）。
fn normalize_id(id: &str) -> String {
    let looks_css = id.starts_with('#')
        || id.starts_with('.')
        || id.starts_with('[')
        || id.contains(' ')
        || id.contains('>');
    let mut chars = id.chars();
    if let Some(first) = chars.next() {
        if !looks_css && chars.next().is_none() && first.is_ascii_alphabetic() {
            return first.to_string();
        }
    }
    if looks_css {
        return id.to_string();
    }
    format!("#{id}")
}

/// 把 `{kind, value}` 形式的 ref 对象转成局部方言字符串（与浏览器工具同义）。
fn ref_object_to_local(v: &Value) -> crate::engine::Result<String> {
    let kind = v
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| EngineError::invalid("ref.kind missing"))?;
    let value = v
        .get("value")
        .and_then(Value::as_str)
        .ok_or_else(|| EngineError::invalid("ref.value missing"))?;
    Ok(match kind {
        "css" => format!("css={value}"),
        "xpath" => format!("xpath={value}"),
        "text" => format!("text={value}"),
        "role" => format!("role={value}"),
        "snapshot" => value.to_string(),
        other => {
            return Err(EngineError::invalid(format!(
                "unknown ref.kind '{other}' (css|xpath|text|role|snapshot)"
            )))
        }
    })
}

/// 统一解析元素目标（方言）：`ref`（完整/裸/对象）→ `selector` → `id` → `element`。
fn resolve_ref(
    ctx: &ToolContext<'_>,
    rt: &crate::engine::SurfaceRuntime,
) -> crate::engine::Result<String> {
    let target = ctx.param_opt::<String>("target")?.unwrap_or_default();
    if let Some(rv) = ctx.params.get("ref") {
        match rv {
            Value::String(s) if !s.trim().is_empty() => {
                // Normalize Playwright-MCP `@ref` / `ref=` prefixes.
                let r = s.trim().trim_start_matches('@');
                let r = r.strip_prefix("ref=").unwrap_or(r).trim();
                if r.contains(':') {
                    return Ok(r.to_string());
                }
                return qualify(rt, &target, r);
            }
            Value::Object(_) => {
                let local = ref_object_to_local(rv)?;
                return qualify(rt, &target, &local);
            }
            _ => {}
        }
    }
    if let Some(sel) = ctx.param_opt::<String>("selector")? {
        let sel = sel.trim();
        if !sel.is_empty() {
            return qualify(rt, &target, sel);
        }
    }
    if let Some(id) = ctx.param_opt::<String>("id")? {
        let id = id.trim();
        if !id.is_empty() {
            let local = normalize_id(id);
            return qualify(rt, &target, &local);
        }
    }
    if let Some(el) = ctx.param_opt::<String>("element")? {
        let el = el.trim();
        if !el.is_empty() {
            // `element` is a human-readable hint, but models frequently put a
            // selector/dialect in it (e.g. `role=button[name="X"]`, `#id`). If
            // it looks like one, treat it as a selector instead of literal text.
            let looks_selector = el.starts_with('#')
                || el.starts_with('.')
                || el.starts_with('[')
                || el.starts_with("//")
                || el.contains('>')
                || el.starts_with("css=")
                || el.starts_with("xpath=")
                || el.starts_with("text=")
                || el.starts_with("role=")
                || el.starts_with("label=")
                || el.starts_with("placeholder=")
                || el.starts_with("alt=")
                || el.starts_with("title=")
                || el.starts_with("value=")
                || el.starts_with("href=")
                || el.starts_with("testid=")
                || el.starts_with("data-testid=");
            if looks_selector {
                return qualify(rt, &target, el);
            }
            return qualify(rt, &target, &format!("text={el}"));
        }
    }
    Err(EngineError::invalid(
        "need an element target: 'ref' (full/bare/{kind,value}), 'selector', 'id', or 'element'",
    ))
}

/// Type `text` into the focused element (used when no element target is
/// given): real keyboard channel first, else JS on the focused element.
fn type_focused(ctx: &ToolContext<'_>, tab: crate::engine::TabId, text: &str, clear: bool) -> bool {
    if ctx.runtime.engine().type_text_focused(tab, text).is_ok() {
        return true;
    }
    let js = crate::engine::inject::set_text_at_point_js(-1.0, -1.0, text, clear);
    let v = ctx.eval_opt(tab, &js);
    matches!(v.as_ref().and_then(|x| x.as_str()), Some("ok"))
}

/// Select `value` on the focused `<select>` (no element target).
fn select_focused(
    ctx: &ToolContext<'_>,
    tab: crate::engine::TabId,
    value: &str,
) -> crate::engine::Result<()> {
    use crate::engine::{EngineError, ErrorKind};
    let js = crate::engine::inject::select_at_point_js(-1.0, -1.0, value);
    match ctx.eval_opt(tab, &js).as_ref().and_then(|x| x.as_str()) {
        Some("ok") => Ok(()),
        Some("nomatch") => Err(EngineError::invalid(format!(
            "select: no <option> matches {value:?}"
        ))),
        _ => Err(EngineError::new(
            ErrorKind::Dom,
            "select: no focused <select> element",
        )),
    }
}

/// 合法 surface 动作（错误提示与文档共用）。
const SURFACE_ACTIONS: &[&str] = &[
    "click",
    "double_click",
    "right_click",
    "focus",
    "set_value",
    "type",
    "check",
    "uncheck",
    "select",
    "scroll",
    "press_key",
    "increment",
    "decrement",
    "show_menu",
    "raise",
    "invoke",
];

/// 解析通用动作（供 `ax_act` 与便捷工具复用）。
fn action_from_params(ctx: &ToolContext<'_>, action: &str) -> crate::engine::Result<SurfaceAction> {
    let a = action.trim().to_ascii_lowercase();
    Ok(match a.as_str() {
        "click" | "left_click" => SurfaceAction::Click,
        "invoke" => SurfaceAction::Invoke,
        "double_click" | "dblclick" => SurfaceAction::DoubleClick,
        "right_click" => SurfaceAction::RightClick,
        // Anthropic computer-use 的 middle_click：无独立动作，退化为点击。
        "middle_click" => SurfaceAction::Click,
        "focus" => SurfaceAction::Focus,
        "increment" => SurfaceAction::Increment,
        "decrement" => SurfaceAction::Decrement,
        "show_menu" => SurfaceAction::ShowMenu,
        "raise" => SurfaceAction::Raise,
        "set_value" | "set" => SurfaceAction::SetValue {
            value: ctx.param::<String>("value")?,
        },
        "type" | "type_text" | "fill" => SurfaceAction::TypeText {
            text: ctx.param::<String>("text")?,
            clear: ctx.param_opt::<bool>("clear")?.unwrap_or(false),
        },
        "check" => SurfaceAction::Check {
            checked: ctx.param_opt::<bool>("checked")?.unwrap_or(true),
        },
        "uncheck" => SurfaceAction::Check { checked: false },
        "select" => SurfaceAction::Select {
            value: ctx.param::<String>("value")?,
        },
        "scroll" => SurfaceAction::Scroll {
            dx: ctx.param_opt::<f64>("dx")?.unwrap_or(0.0),
            dy: ctx.param_opt::<f64>("dy")?.unwrap_or(0.0),
        },
        "press_key" | "press" | "key" => SurfaceAction::PressKey {
            key: ctx.param::<String>("key")?,
        },
        other => {
            return Err(EngineError::invalid(format!(
                "unknown surface action '{other}'; valid actions: {}",
                SURFACE_ACTIONS.join(", ")
            )))
        }
    })
}

/// 元素目标参数（供 `ax_act`/`ax_click`/`ax_type` 的 schema 复用）。
fn target_params() -> Value {
    json!({
        "ref": {"type": "string", "required": false, "description": "Element ref: full (web:3:a / desktop:1234:0.2.1) or bare (a / #go / text=Go / role=button)."},
        "selector": {"type": "string", "required": false, "description": "Selector dialect (CSS / text= / role= / xpath= / //...)."},
        "id": {"type": "string", "required": false, "description": "Snapshot letter or element id."},
        "element": {"type": "string", "required": false, "description": "Human-readable hint (Playwright MCP) → text match."},
        "target": {"type": "string", "required": false, "description": "Surface id to bind a bare target to (defaults to active/first)."},
        "x": {"type": "number", "required": false, "description": "Screen x for a coordinate click (with y; ignores element target)."},
        "y": {"type": "number", "required": false, "description": "Screen y for a coordinate click (with x)."}
    })
}

fn ax_list() -> Tool {
    Tool::new(
        "ax_list",
        "Accessibility API: list all operable accessibility targets (surfaces) across providers — browser tabs and native apps/windows. Returns id/kind/title/url/active for each.",
        json!({}),
        r#"{}"#,
        |ctx| {
            let rt = surface(ctx)?;
            let report = rt.list_surfaces_report_blocking();
            let count = report.surfaces.len();
            let mut out = json!({ "surfaces": report.surfaces, "count": count });
            // Report providers that were unavailable (e.g. a native backend whose
            // host service/permission is not enabled) instead of silently
            // omitting them — otherwise the agent assumes the listed surfaces
            // are the full capability set.
            if !report.skipped.is_empty() {
                out["skipped"] = json!(report
                    .skipped
                    .iter()
                    .map(|(p, e)| json!({ "provider": p, "error": e }))
                    .collect::<Vec<_>>());
                out["hint"] = json!(
                    "some surface providers were unavailable; enable the host \
                     accessibility service/permission to expose native app UI"
                );
            }
            Ok(out)
        },
    )
    .requires(&[Capability::Surface])
}

fn ax_snapshot() -> Tool {
    Tool::new(
        "ax_snapshot",
        "Accessibility API (macOS AXUIElement / Windows UIA / Linux AT-SPI): capture a unified accessibility snapshot (UiNode tree) of a surface (browser or native). Target by 'target' (e.g. web:3 or desktop:1234); defaults to the first/active surface. Pruned by default (interestingOnly + node/depth caps) to bound tokens; 'meta.truncated' reports any truncation.",
        json!({
            "target": {"type": "string", "required": false, "description": "Surface id, e.g. web:3 / desktop:1234. Defaults to first surface."},
            "max_nodes": {"type": "integer", "required": false, "description": "Max nodes to return (0 = default)."},
            "max_depth": {"type": "integer", "required": false, "description": "Max tree depth (0 = default)."},
            "interesting_only": {"type": "boolean", "required": false, "description": "Drop pure structural containers (default true)."},
            "max_text_len": {"type": "integer", "required": false, "description": "Max chars per text field."}
        }),
        r#"{"target": "web:1"}"#,
        |ctx| {
            let rt = surface(ctx)?;
            let target = ctx.param_opt::<String>("target")?.unwrap_or_default();
            let snap = rt.snapshot_blocking(&target, snapshot_options(ctx))?;
            Ok(snapshot_json(&snap))
        },
    )
    .requires(&[Capability::Surface])
}

fn ax_act() -> Tool {
    Tool::new(
        "ax_act",
        "Accessibility API (macOS AXUIElement / Windows UIA / Linux AT-SPI): apply an action to an element (or surface) identified by 'ref' (full or bare; also accepts 'selector'/'id'/'element'). Unified across web and native surfaces. Actions: click, double_click, right_click, focus, set_value, type, check/uncheck, select, scroll, press_key, increment, decrement, show_menu, raise.",
        json!({
            "ref": {"type": "string", "required": false, "description": "Element ref: full (web:3:a / desktop:1234:0.2.1) or bare (a / #go / text=Go / role=button)."},
            "selector": {"type": "string", "required": false, "description": "Selector dialect (CSS / text= / role= / xpath= / //...)."},
            "id": {"type": "string", "required": false, "description": "Snapshot letter or element id."},
            "element": {"type": "string", "required": false, "description": "Human-readable hint → text match."},
            "target": {"type": "string", "required": false, "description": "Surface id to bind a bare target to."},
            "action": {"type": "string", "required": true, "description": "Action name (see tool description)."},
            "value": {"type": "string", "required": false, "description": "Value for set_value / select."},
            "text": {"type": "string", "required": false, "description": "Text for type."},
            "clear": {"type": "boolean", "required": false, "description": "Clear before typing."},
            "checked": {"type": "boolean", "required": false, "description": "Target state for check."},
            "key": {"type": "string", "required": false, "description": "Key for press_key."},
            "dx": {"type": "number", "required": false, "description": "Horizontal scroll delta."},
            "dy": {"type": "number", "required": false, "description": "Vertical scroll delta."},
            "x": {"type": "number", "required": false, "description": "Screen x for a coordinate click (with y; ignores element target)."},
            "y": {"type": "number", "required": false, "description": "Screen y for a coordinate click (with x)."}
        }),
        r#"{"ref": "web:1:a", "action": "click"}"#,
        |ctx| {
            let rt = surface(ctx)?;
            let action_name = ctx.param::<String>("action")?;
            let action = action_from_params(ctx, &action_name)?;

            // 坐标动作（Anthropic computer-use 风格）：x/y 优先于元素目标。
            let x = ctx.param_opt::<f64>("x")?;
            let y = ctx.param_opt::<f64>("y")?;
            if let (Some(x), Some(y)) = (x, y) {
                if matches!(
                    action,
                    SurfaceAction::Click
                        | SurfaceAction::Invoke
                        | SurfaceAction::DoubleClick
                        | SurfaceAction::RightClick
                        | SurfaceAction::Focus
                ) {
                    let target = ctx.param_opt::<String>("target")?.unwrap_or_default();
                    let ev = match action {
                        SurfaceAction::DoubleClick => {
                            InputEvent::Mouse(MouseEvent::double_click(x, y))
                        }
                        SurfaceAction::RightClick => {
                            InputEvent::Mouse(MouseEvent::click(x, y, MouseButton::Right))
                        }
                        _ => InputEvent::Mouse(MouseEvent::click(x, y, MouseButton::Left)),
                    };
                    rt.input_blocking(&target, ev)?;
                    return Ok(json!({
                        "ok": true, "action": action_name, "x": x, "y": y, "coordinate": true
                    }));
                }
            }

            // 无元素目标的表面级动作：press_key / scroll / raise 可作用于表面本身。
            let ref_ = match resolve_ref(ctx, rt) {
                Ok(r) => r,
                Err(e) => match &action {
                    SurfaceAction::PressKey { key } => {
                        let ev = crate::engine::InputEvent::Key(crate::engine::KeyEvent::press(
                            key.clone(),
                        ));
                        rt.input_blocking("", ev)?;
                        return Ok(json!({"ok": true, "action": action_name, "surface_level": true}));
                    }
                    SurfaceAction::Scroll { dx, dy } => {
                        let ev = crate::engine::InputEvent::Wheel(crate::engine::WheelEvent {
                            x: 0.0,
                            y: 0.0,
                            delta_x: *dx,
                            delta_y: *dy,
                        });
                        rt.input_blocking("", ev)?;
                        return Ok(json!({"ok": true, "action": action_name, "surface_level": true}));
                    }
                    SurfaceAction::Raise => {
                        let sid = surface_id_for(rt, "")?;
                        rt.act_blocking(&format!("{sid}:root"), SurfaceAction::Raise)?;
                        return Ok(json!({"ok": true, "action": action_name, "surface_level": true}));
                    }
                    // No element target for text/select → operate on the
                    // focused element (common when the user/model focused first).
                    SurfaceAction::TypeText { text, clear } => {
                        let tab = ctx.target_tab()?;
                        if type_focused(ctx, tab, text, *clear) {
                            return Ok(json!({"ok": true, "action": action_name, "focused": true}));
                        }
                        return Err(e);
                    }
                    SurfaceAction::SetValue { value } => {
                        let tab = ctx.target_tab()?;
                        if type_focused(ctx, tab, value, true) {
                            return Ok(json!({"ok": true, "action": action_name, "focused": true}));
                        }
                        return Err(e);
                    }
                    SurfaceAction::Select { value } => {
                        let tab = ctx.target_tab()?;
                        select_focused(ctx, tab, value)?;
                        return Ok(json!({"ok": true, "action": action_name, "focused": true}));
                    }
                    _ => return Err(e),
                },
            };
            rt.act_blocking(&ref_, action)?;
            Ok(json!({ "ok": true, "ref": ref_, "action": action_name }))
        },
    )
    .requires(&[Capability::Surface])
}

fn ax_click() -> Tool {
    Tool::new(
        "ax_click",
        "Accessibility API: click an element on any surface (shortcut for ax_act action=click). Accepts 'ref' (full or bare) or 'selector'/'id'/'element'.",
        target_params(),
        r#"{"ref": "web:1:a"}"#,
        |ctx| {
            let rt = surface(ctx)?;
            let x = ctx.param_opt::<f64>("x")?;
            let y = ctx.param_opt::<f64>("y")?;
            if let (Some(x), Some(y)) = (x, y) {
                let target = ctx.param_opt::<String>("target")?.unwrap_or_default();
                let ev = InputEvent::Mouse(MouseEvent::click(x, y, MouseButton::Left));
                rt.input_blocking(&target, ev)?;
                return Ok(json!({ "ok": true, "x": x, "y": y, "coordinate": true }));
            }
            let ref_ = resolve_ref(ctx, rt)?;
            rt.act_blocking(&ref_, SurfaceAction::Click)?;
            Ok(json!({ "ok": true, "ref": ref_ }))
        },
    )
    .requires(&[Capability::Surface])
}

fn ax_type() -> Tool {
    Tool::new(
        "ax_type",
        "Accessibility API: type text into an element on any surface (shortcut for ax_act action=type). Accepts 'ref' (full or bare) or 'selector'/'id'/'element'.",
        json!({
            "ref": {"type": "string", "required": false, "description": "Element ref: full or bare."},
            "selector": {"type": "string", "required": false, "description": "Selector dialect."},
            "id": {"type": "string", "required": false, "description": "Snapshot letter or element id."},
            "element": {"type": "string", "required": false, "description": "Human-readable hint → text match."},
            "target": {"type": "string", "required": false, "description": "Surface id to bind a bare target to."},
            "text": {"type": "string", "required": true, "description": "Text to type."},
            "clear": {"type": "boolean", "required": false, "description": "Clear before typing."}
        }),
        r#"{"ref": "web:1:e", "text": "hello"}"#,
        |ctx| {
            let rt = surface(ctx)?;
            let text = ctx.param::<String>("text")?;
            let clear = ctx.param_opt::<bool>("clear")?.unwrap_or(false);
            match resolve_ref(ctx, rt) {
                Ok(ref_) => {
                    rt.act_blocking(&ref_, SurfaceAction::TypeText { text, clear })?;
                    Ok(json!({ "ok": true, "ref": ref_ }))
                }
                Err(e) => {
                    // No element target → type into the focused element.
                    let tab = ctx.target_tab()?;
                    if type_focused(ctx, tab, &text, clear) {
                        Ok(json!({ "ok": true, "focused": true }))
                    } else {
                        Err(e)
                    }
                }
            }
        },
    )
    .requires(&[Capability::Surface])
}

fn ax_scroll() -> Tool {
    Tool::new(
        "ax_scroll",
        "Accessibility API: scroll a surface element (by 'ref'/'selector'/'id') or the active surface by (dx, dy).",
        json!({
            "ref": {"type": "string", "required": false, "description": "Element ref (optional)."},
            "selector": {"type": "string", "required": false, "description": "Selector dialect (optional)."},
            "id": {"type": "string", "required": false, "description": "Snapshot letter or element id (optional)."},
            "target": {"type": "string", "required": false, "description": "Surface id to bind a bare target to."},
            "dx": {"type": "number", "required": false, "description": "Horizontal scroll delta."},
            "dy": {"type": "number", "required": false, "description": "Vertical scroll delta."}
        }),
        r#"{"ref": "web:1:a", "dy": 300}"#,
        |ctx| {
            let rt = surface(ctx)?;
            let dx = ctx.param_opt::<f64>("dx")?.unwrap_or(0.0);
            let dy = ctx.param_opt::<f64>("dy")?.unwrap_or(0.0);
            match resolve_ref(ctx, rt) {
                Ok(ref_) => rt.act_blocking(&ref_, SurfaceAction::Scroll { dx, dy })?,
                Err(_) => {
                    // 无元素目标 → 表面级滚动（向坐标输入 provider 发滚轮事件）。
                    let event = crate::engine::InputEvent::Wheel(crate::engine::WheelEvent {
                        x: 0.0,
                        y: 0.0,
                        delta_x: dx,
                        delta_y: dy,
                    });
                    rt.input_blocking("", event)?;
                }
            }
            Ok(json!({ "ok": true, "dx": dx, "dy": dy }))
        },
    )
    .requires(&[Capability::Surface])
}

/// 全部 AX（无障碍）工具。
pub fn tools() -> Vec<Tool> {
    vec![
        ax_list(),
        ax_snapshot(),
        ax_act(),
        ax_click(),
        ax_type(),
        ax_scroll(),
        ax_events(),
    ]
}

fn ax_events() -> Tool {
    Tool::new(
        "ax_events",
        "Accessibility API: drain native accessibility events (focus/window/value/title/selection/structure changes) accumulated since the last call. Native providers (macOS AXObserver) push in the background; returns [] when there are none or the provider has no event source.",
        json!({
            "target": {"type": "string", "required": false, "description": "Surface id (optional; defaults to focused/active)."}
        }),
        r#"{"target": "desktop:1"}"#,
        |ctx| {
            let rt = surface(ctx)?;
            let target = ctx.param_opt::<String>("target")?.unwrap_or_default();
            let events = rt.poll_events_blocking(&target)?;
            Ok(json!({ "events": events, "count": events.len() }))
        },
    )
    .requires(&[Capability::Surface])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::Runtime;
    use crate::config::Config;
    use crate::engines::mock::MockEngine;

    fn runtime_with_surface() -> Runtime {
        let mut cfg = Config::default();
        cfg.surface.provider = "mock".to_string();
        Runtime::new(Box::new(MockEngine::new()), cfg)
    }

    #[test]
    fn list_and_snapshot() {
        let r = runtime_with_surface();
        let list = r.call_tool("ax_list", json!({})).unwrap();
        assert!(list["count"].as_u64().unwrap() >= 1);
        let snap = r
            .call_tool("ax_snapshot", json!({"target": "desktop:1"}))
            .unwrap();
        assert_eq!(snap["surface"]["id"], "desktop:1");
        assert!(snap["tree"].is_object());
        assert!(snap["text"].as_str().unwrap().contains("button \"OK\""));
    }

    #[test]
    fn click_and_type_by_ref() {
        let r = runtime_with_surface();
        r.call_tool("ax_act", json!({"ref": "desktop:1:0.0", "action": "click"}))
            .unwrap();
        r.call_tool(
            "ax_type",
            json!({"ref": "desktop:1:0.1", "text": "Bob", "clear": true}),
        )
        .unwrap();
        let snap = r
            .call_tool("ax_snapshot", json!({"target": "desktop:1"}))
            .unwrap();
        assert_eq!(snap["tree"]["children"][1]["value"], "Bob");
    }

    #[test]
    fn bare_ref_is_qualified_to_active_surface() {
        let r = runtime_with_surface();
        // 裸 ref `0.0` → 绑定到活动/首个表面 `desktop:1`。
        let out = r.call_tool("ax_click", json!({"ref": "0.0"})).unwrap();
        assert_eq!(out["ref"], "desktop:1:0.0");
    }

    #[test]
    fn selector_param_is_accepted() {
        let r = runtime_with_surface();
        // 脚本化表面不支持 CSS，但工具层应完成「方言 → 完整 ref」的绑定并调用 provider。
        let out = r
            .call_tool("ax_act", json!({"selector": "#ok", "action": "click"}))
            .unwrap();
        assert_eq!(out["ref"], "desktop:1:#ok");
    }

    #[test]
    fn id_param_normalization() {
        let r = runtime_with_surface();
        let out = r.call_tool("ax_click", json!({"id": "0.0"})).unwrap();
        assert_eq!(out["ref"], "desktop:1:#0.0");
        let out = r.call_tool("ax_click", json!({"id": "0.1"})).unwrap();
        assert_eq!(out["ref"], "desktop:1:#0.1");
    }

    #[test]
    fn snapshot_caps_nodes() {
        let r = runtime_with_surface();
        let snap = r
            .call_tool(
                "ax_snapshot",
                json!({"target": "desktop:1", "max_nodes": 1}),
            )
            .unwrap();
        assert_eq!(snap["meta"]["truncated"], true);
    }

    #[test]
    fn unknown_action_errors() {
        let r = runtime_with_surface();
        let e = r.call_tool(
            "ax_act",
            json!({"ref": "desktop:1:0.0", "action": "banana"}),
        );
        assert!(e.is_err());
    }

    #[test]
    fn missing_target_errors() {
        let r = runtime_with_surface();
        assert!(r.call_tool("ax_click", json!({})).is_err());
    }

    #[test]
    fn coordinate_click_uses_input() {
        let r = runtime_with_surface();
        let out = r.call_tool("ax_click", json!({"x": 10, "y": 20})).unwrap();
        assert_eq!(out["coordinate"], true);
        let out = r
            .call_tool("ax_act", json!({"action": "left_click", "x": 1, "y": 2}))
            .unwrap();
        assert_eq!(out["coordinate"], true);
    }

    #[test]
    fn anthropic_action_aliases() {
        let r = runtime_with_surface();
        for action in ["left_click", "right_click", "middle_click"] {
            let out = r
                .call_tool("ax_act", json!({"ref": "desktop:1:0.0", "action": action}))
                .unwrap();
            assert_eq!(out["action"], action);
        }
        let out = r
            .call_tool(
                "ax_act",
                json!({"ref": "desktop:1:0.0", "action": "key", "key": "Tab"}),
            )
            .unwrap();
        assert_eq!(out["action"], "key");
    }

    #[test]
    fn surface_events_drains_empty() {
        let r = runtime_with_surface();
        let v = r.call_tool("ax_events", json!({})).unwrap();
        assert_eq!(v["count"], 0);
        assert!(v["events"].as_array().unwrap().is_empty());
    }

    #[test]
    fn object_ref_form_is_supported() {
        let r = runtime_with_surface();
        let out = r
            .call_tool(
                "ax_act",
                json!({"ref": {"kind": "css", "value": "#ok"}, "action": "click"}),
            )
            .unwrap();
        assert_eq!(out["ref"], "desktop:1:css=#ok");
        let out = r
            .call_tool("ax_click", json!({"ref": {"kind": "text", "value": "OK"}}))
            .unwrap();
        assert_eq!(out["ref"], "desktop:1:text=OK");
    }
}
