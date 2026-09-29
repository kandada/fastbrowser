// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! 浏览器表面：把同步 `BrowserEngine` 适配为异步 `SurfaceProvider`（命名空间 `web`）。
//!
//! 这是「浏览器作为第一个表面」的落地：Agent 既可用原有浏览器工具，也可用统一的
//! `surface_*` 工具，后者对 web 与原生一视同仁。
//!
//! 同步引擎调用一律经 `spawn_blocking`，避免占用异步反应堆。

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::Value;

use crate::engine::surface::{
    prune, SnapshotOptions, SurfaceAction, SurfaceCapabilities, SurfaceInfo, SurfaceKind,
    SurfaceSnapshot, UiNode,
};
use crate::engine::{
    BrowserEngine, ElementRef, EngineError, ErrorKind, Image, InputEvent, MouseButton, MouseEvent,
    PageSnapshot, Rect, RefKind, Result, TabId,
};

/// web 表面 provider。
pub struct BrowserSurface {
    engine: Arc<dyn BrowserEngine>,
    generation: AtomicU64,
    /// 是否优先使用引擎的原生 AX 树（CDP `Accessibility.getFullAXTree`）。
    prefer_ax: bool,
    /// AX 节点 ref → 屏幕/页面几何（世代内有效；新快照时按 tab 刷新）。
    geo_cache: Mutex<HashMap<String, Rect>>,
}

impl BrowserSurface {
    pub fn new(engine: Arc<dyn BrowserEngine>) -> Self {
        Self::with_options(engine, true)
    }

    /// `prefer_ax=false` 时始终走注入 JS 的交互元素快照（旧行为）。
    pub fn with_options(engine: Arc<dyn BrowserEngine>, prefer_ax: bool) -> Self {
        BrowserSurface {
            engine,
            generation: AtomicU64::new(0),
            prefer_ax,
            geo_cache: Mutex::new(HashMap::new()),
        }
    }

    /// 在阻塞池上执行同步引擎调用。
    async fn blocking<T, F>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&dyn BrowserEngine) -> Result<T> + Send + 'static,
        T: Send + 'static,
    {
        let engine = self.engine.clone();
        tokio::task::spawn_blocking(move || f(engine.as_ref()))
            .await
            .map_err(|e| EngineError::new(ErrorKind::Internal, format!("surface join: {e}")))?
    }

    /// 对 AX 引用施加动作：用快照缓存的几何做坐标级操作（点击/滚轮/聚焦输入）。
    async fn act_ax_ref(&self, tab: TabId, local: &str, action: SurfaceAction) -> Result<()> {
        let ref_key = format!("web:{}:{}", tab.as_u32(), local);
        let rect = {
            let guard = self.geo_cache.lock().unwrap_or_else(|e| e.into_inner());
            guard.get(&ref_key).copied()
        };
        let rect = rect.ok_or_else(|| {
            EngineError::new(
                ErrorKind::Dom,
                format!("stale ax ref '{ref_key}'; take a fresh ax_snapshot"),
            )
        })?;
        let (cx, cy) = rect.center();

        match action {
            SurfaceAction::Click | SurfaceAction::Invoke => {
                let ev = InputEvent::Mouse(MouseEvent::click(cx, cy, MouseButton::Left));
                self.blocking(move |e| e.inject_event(tab, ev)).await
            }
            SurfaceAction::Focus => {
                // Synthetic clicks don't move focus → focus the element at the
                // point explicitly; only fall back to a click if that fails.
                let js = crate::engine::inject::focus_at_point_js(cx, cy);
                if self.eval_str(tab, js).await?.as_deref() == Some("ok") {
                    return Ok(());
                }
                let ev = InputEvent::Mouse(MouseEvent::click(cx, cy, MouseButton::Left));
                self.blocking(move |e| e.inject_event(tab, ev)).await
            }
            SurfaceAction::Check { checked } => {
                let js = crate::engine::inject::check_at_point_js(cx, cy, checked);
                if self.eval_str(tab, js).await?.as_deref() == Some("ok") {
                    return Ok(());
                }
                let ev = InputEvent::Mouse(MouseEvent::click(cx, cy, MouseButton::Left));
                self.blocking(move |e| e.inject_event(tab, ev)).await
            }
            SurfaceAction::DoubleClick => {
                let ev = InputEvent::Mouse(MouseEvent::double_click(cx, cy));
                self.blocking(move |e| e.inject_event(tab, ev)).await
            }
            SurfaceAction::RightClick => {
                let ev = InputEvent::Mouse(MouseEvent::click(cx, cy, MouseButton::Right));
                self.blocking(move |e| e.inject_event(tab, ev)).await
            }
            SurfaceAction::Scroll { dx, dy } => {
                let ev = InputEvent::Wheel(crate::engine::WheelEvent {
                    x: cx,
                    y: cy,
                    delta_x: dx,
                    delta_y: dy,
                });
                self.blocking(move |e| e.inject_event(tab, ev)).await
            }
            SurfaceAction::PressKey { key } => {
                let ev = InputEvent::Key(crate::engine::KeyEvent::press(key));
                self.blocking(move |e| e.inject_event(tab, ev)).await
            }
            SurfaceAction::SetValue { value } => {
                // Prefer the real keyboard channel; else set the value at the
                // element's own position (works without relying on focus).
                let v2 = value.clone();
                if self
                    .blocking(move |e| e.type_text_focused(tab, &v2))
                    .await
                    .is_ok()
                {
                    return Ok(());
                }
                self.text_at_point(tab, cx, cy, &value, true).await
            }
            SurfaceAction::TypeText { text, clear } => {
                let t2 = text.clone();
                if self
                    .blocking(move |e| e.type_text_focused(tab, &t2))
                    .await
                    .is_ok()
                {
                    return Ok(());
                }
                self.text_at_point(tab, cx, cy, &text, clear).await
            }
            SurfaceAction::Select { value } => {
                let js = crate::engine::inject::select_at_point_js(cx, cy, &value);
                match self.eval_str(tab, js).await?.as_deref() {
                    Some("ok") => Ok(()),
                    Some("nosel") => Err(EngineError::new(
                        ErrorKind::Dom,
                        "select: no <select> at the element's position",
                    )),
                    Some("nomatch") => Err(EngineError::invalid(format!(
                        "select: no <option> matches {value:?}"
                    ))),
                    _ => Err(EngineError::new(
                        ErrorKind::Dom,
                        "select: element not found",
                    )),
                }
            }
            other => Err(EngineError::unsupported(format!(
                "action '{}' not supported on AX ref",
                other.name()
            ))),
        }
    }

    /// Evaluate a small JS snippet and return its string result (if any).
    async fn eval_str(&self, tab: TabId, js: String) -> Result<Option<String>> {
        self.blocking(move |e| {
            let v = e.evaluate(tab, &js)?;
            Ok(v.as_str().map(|s| s.to_string()))
        })
        .await
    }

    /// Insert `text` into the editable element at `(cx,cy)` (no reliance on the
    /// current focus). Errors clearly when there is nothing editable there.
    async fn text_at_point(
        &self,
        tab: TabId,
        cx: f64,
        cy: f64,
        text: &str,
        clear: bool,
    ) -> Result<()> {
        let js = crate::engine::inject::set_text_at_point_js(cx, cy, text, clear);
        match self.eval_str(tab, js).await?.as_deref() {
            Some("ok") => Ok(()),
            _ => Err(EngineError::new(
                ErrorKind::Dom,
                "no editable element at the target position (or focused) for text input",
            )),
        }
    }

    /// 解析目标表面为标签页：`web:3` / `web:3:a` / `3` / 空（活动标签）。
    fn parse_tab(&self, target: &str) -> Result<TabId> {
        let t = target.trim();
        if t.is_empty() {
            return self
                .engine
                .active_tab()
                .ok_or_else(|| EngineError::new(ErrorKind::TabNotFound, "no active tab"));
        }
        let second = if let Some(rest) = t.strip_prefix("web:") {
            rest.split(':').next().unwrap_or("")
        } else {
            t.split(':').next().unwrap_or("")
        };
        if let Ok(n) = second.parse::<u32>() {
            return Ok(TabId(n));
        }
        self.engine
            .active_tab()
            .ok_or_else(|| EngineError::new(ErrorKind::TabNotFound, "no active tab"))
    }

    /// 从 ref 中解析（tab, 局部 id）。
    fn parse_ref(&self, ref_: &str) -> Result<(TabId, String)> {
        let parts: Vec<&str> = ref_.splitn(3, ':').collect();
        if parts.len() < 3 || parts[0] != "web" {
            return Err(EngineError::invalid(format!(
                "bad web ref '{ref_}' (expected web:<tab>:<id>)"
            )));
        }
        let tab: u32 = parts[1]
            .parse()
            .map_err(|_| EngineError::invalid(format!("bad tab in ref '{ref_}'")))?;
        Ok((TabId(tab), parts[2].to_string()))
    }

    fn snapshot_to_tree(tab: TabId, snap: &PageSnapshot) -> UiNode {
        let prefix = format!("web:{tab}:");
        let mut root = UiNode::new("root", format!("{prefix}root"), "webarea")
            .with_name(snap.title.clone())
            .with_source(SurfaceKind::Web);
        if let Some(url) = Some(snap.url.clone()) {
            root.attrs.insert("url".into(), url);
        }
        for e in &snap.interactive {
            root.children.push(element_to_node(&prefix, e));
        }
        for (i, frame) in snap.frames.iter().enumerate() {
            let mut fnode = UiNode::new(format!("frame{i}"), format!("{prefix}frame{i}"), "iframe")
                .with_name(frame.url.clone())
                .with_source(SurfaceKind::Web);
            fnode
                .attrs
                .insert("cross_origin".into(), frame.cross_origin.to_string());
            if frame.truncated {
                fnode.attrs.insert("truncated".into(), "true".into());
            }
            for e in &frame.interactive {
                fnode.children.push(element_to_node(&prefix, e));
            }
            root.children.push(fnode);
        }
        root
    }
}

fn element_to_node(prefix: &str, e: &crate::engine::InteractiveElement) -> UiNode {
    let id = e.id.to_string();
    let role = e
        .role
        .clone()
        .unwrap_or_else(|| role_for_tag(&e.tag, e.input_type.as_deref()));
    let mut n = UiNode::new(id.clone(), format!("{prefix}{id}"), role)
        .with_name(e.text.clone().unwrap_or_default())
        .with_source(SurfaceKind::Web);
    if let Some(v) = &e.value {
        n = n.with_value(v.clone());
    }
    n.geometry = Some(e.rect);
    n.states.checked = e.checked;
    n.states.disabled = e.attrs.get("disabled").map(|v| v == "true");
    n.states.editable = matches!(e.tag.as_str(), "input" | "textarea" | "select").then_some(true);
    // 动作：所有可交互元素可 click；输入类可 set_value。
    n.actions.push("click".into());
    if matches!(e.tag.as_str(), "input" | "textarea") {
        n.actions.push("set_value".into());
    }
    if e.tag == "select" {
        n.actions.push("select".into());
    }
    n.backend = Some(format!("data-fb={id}"));
    n
}

/// 把引擎 AX 树（`{role,name,value,description,props,backend_id,geometry,children}`）
/// 映射为统一 [`UiNode`]，并把带几何的节点写入 ref→Rect 缓存（供坐标动作）。
fn ax_node_to_ui(
    prefix: &str,
    node: &Value,
    path: &str,
    cache: &mut HashMap<String, Rect>,
) -> Option<UiNode> {
    let role = node.get("role").and_then(Value::as_str).unwrap_or("");
    let backend = node.get("backend_id").and_then(Value::as_i64);
    let has_children = node
        .get("children")
        .and_then(Value::as_array)
        .map(|a| !a.is_empty())
        .unwrap_or(false);
    if role.is_empty() && backend.is_none() && !has_children {
        return None;
    }
    let local = match backend {
        Some(b) => format!("ax:{b}"),
        None if path.is_empty() => "axp:root".to_string(),
        None => format!("axp:{path}"),
    };
    let ref_ = format!("{prefix}{local}");
    let mut n = UiNode::new(
        local,
        ref_.clone(),
        if role.is_empty() { "generic" } else { role },
    )
    .with_source(SurfaceKind::Web);
    n.name = non_empty_str(node, "name");
    n.value = non_empty_str(node, "value");
    n.description = non_empty_str(node, "description");
    if let Some(props) = node.get("props") {
        let b = |k: &str| props.get(k).and_then(Value::as_bool);
        n.states.checked = b("checked");
        n.states.disabled = b("disabled");
        n.states.selected = b("selected");
        n.states.expanded = b("expanded");
        n.states.focused = b("focused");
        n.states.required = b("required");
        n.states.busy = b("busy");
    }
    if let Some(g) = node.get("geometry").filter(|g| !g.is_null()) {
        let rect = Rect::new(
            g.get("x").and_then(Value::as_f64).unwrap_or(0.0),
            g.get("y").and_then(Value::as_f64).unwrap_or(0.0),
            g.get("width").and_then(Value::as_f64).unwrap_or(0.0),
            g.get("height").and_then(Value::as_f64).unwrap_or(0.0),
        );
        n.geometry = Some(rect);
        cache.insert(ref_.clone(), rect);
    }
    n.actions = ax_actions_for_role(role);
    n.backend = Some(ref_.clone());
    if let Some(children) = node.get("children").and_then(Value::as_array) {
        for (i, c) in children.iter().enumerate() {
            let cpath = if path.is_empty() {
                i.to_string()
            } else {
                format!("{path}.{i}")
            };
            if let Some(child) = ax_node_to_ui(prefix, c, &cpath, cache) {
                n.children.push(child);
            }
        }
    }
    Some(n)
}

fn non_empty_str(node: &Value, key: &str) -> Option<String> {
    node.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(String::from)
}

/// AX 角色 → 可用动作（供 Agent 选择动作）。
fn ax_actions_for_role(role: &str) -> Vec<String> {
    match role {
        "button" | "link" | "menuitem" | "menuitemcheckbox" | "menuitemradio" | "tab"
        | "option" | "treeitem" | "switch" => vec!["click".into()],
        "checkbox" | "radio" => vec!["click".into(), "check".into()],
        "textbox" | "searchbox" | "spinbutton" => vec!["click".into(), "set_value".into()],
        "combobox" => vec!["click".into(), "select".into()],
        "slider" => vec!["set_value".into(), "increment".into(), "decrement".into()],
        _ => Vec::new(),
    }
}

/// 把 web ref 的局部部分（`web:<tab>:<local>` 的 `<local>`）解析为 [`ElementRef`]：
/// 单字母 `a..z` / `eN` → 快照 id；否则按选择器方言（`css=`/`text=`/`role=`/`xpath=`/
/// `id=`/`data-testid=`/`nth=`/`//...`/`:has-text(...)`）解析，与浏览器专有工具完全一致。
fn local_to_element_ref(local: &str) -> Result<ElementRef> {
    let l = local.trim();
    if l.is_empty() {
        return Err(EngineError::invalid("empty element id"));
    }
    if let Some(c) = crate::engine::snapshot::parse_snapshot_ref(l) {
        return Ok(ElementRef::snapshot(c));
    }
    Ok(crate::engine::snapshot::parse_selector_dialect(l))
}

/// 解析元素为具体快照 id：先查缓存快照，未命中再用实时 DOM 查询（与工具层一致）。
fn snapshot_id_of(engine: &dyn BrowserEngine, tab: TabId, r: &ElementRef) -> Result<char> {
    let snap = engine.snapshot(tab)?;
    match r.kind {
        RefKind::Snapshot => {
            let c = r
                .value
                .chars()
                .next()
                .ok_or_else(|| EngineError::invalid("bad snapshot id"))?;
            if snap.element_by_id(c).is_none() {
                return Err(EngineError::new(
                    ErrorKind::Dom,
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
            let v = engine.evaluate(tab, &crate::engine::inject::live_resolve_js(r))?;
            v.as_str()
                .and_then(|s| s.chars().next())
                .ok_or_else(|| EngineError::new(ErrorKind::Dom, format!("element {r} not found")))
        }
    }
}

fn role_for_tag(tag: &str, input_type: Option<&str>) -> String {
    match tag {
        "a" => "link",
        "button" => "button",
        "select" => "combobox",
        "textarea" => "textbox",
        "img" => "image",
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => "heading",
        "ul" | "ol" => "list",
        "li" => "listitem",
        "nav" => "navigation",
        "form" => "form",
        "input" => match input_type {
            Some("checkbox") => "checkbox",
            Some("radio") => "radio",
            Some("submit") | Some("button") => "button",
            _ => "textbox",
        },
        other => other,
    }
    .to_string()
}

#[async_trait]
impl crate::engine::SurfaceProvider for BrowserSurface {
    fn name(&self) -> &'static str {
        "browser"
    }

    fn kind(&self) -> SurfaceKind {
        SurfaceKind::Web
    }

    fn capabilities(&self) -> SurfaceCapabilities {
        SurfaceCapabilities::web()
    }

    fn namespace(&self) -> &'static str {
        "web"
    }

    async fn list_surfaces(&self) -> Result<Vec<SurfaceInfo>> {
        self.blocking(|engine| {
            let active = engine.active_tab();
            Ok(engine
                .list_tabs()
                .into_iter()
                .map(|t| SurfaceInfo {
                    id: format!("web:{}", t.id.as_u32()),
                    kind: SurfaceKind::Web,
                    title: t.title.clone(),
                    app: Some("browser".into()),
                    pid: None,
                    url: Some(t.url.clone()),
                    bounds: None,
                    active: Some(t.id) == active,
                })
                .collect())
        })
        .await
    }

    async fn snapshot(&self, target: &str, opts: SnapshotOptions) -> Result<SurfaceSnapshot> {
        let tab = self.parse_tab(target)?;

        // 1) 优先：引擎原生 AX 树（CDP `Accessibility.getFullAXTree` + 几何关联）。
        if self.prefer_ax {
            if let Ok(ax) = self
                .blocking(move |engine| engine.accessibility_tree(tab))
                .await
            {
                if let Some(root_json) = ax.get("root").filter(|r| !r.is_null()) {
                    let prefix = format!("web:{}:", tab.as_u32());
                    let mut cache: HashMap<String, Rect> = HashMap::new();
                    let ui = ax_node_to_ui(&prefix, root_json, "", &mut cache);
                    if let Some(ui) = ui {
                        // 刷新该 tab 的几何缓存（按 tab 前缀替换）。
                        {
                            let mut guard =
                                self.geo_cache.lock().unwrap_or_else(|e| e.into_inner());
                            guard.retain(|k, _| !k.starts_with(&prefix));
                            guard.extend(cache);
                        }
                        let (root, mut meta) = prune(ui, &opts);
                        meta.truncated |= ax
                            .get("truncated")
                            .and_then(Value::as_bool)
                            .unwrap_or(false);
                        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
                        let title = root_json
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        let url = self
                            .blocking(move |e| e.page_url(tab))
                            .await
                            .unwrap_or_default();
                        let surface = SurfaceInfo {
                            id: format!("web:{}", tab.as_u32()),
                            kind: SurfaceKind::Web,
                            title,
                            app: Some("browser".into()),
                            pid: None,
                            url: Some(url),
                            bounds: None,
                            active: self.engine.active_tab() == Some(tab),
                        };
                        return Ok(SurfaceSnapshot {
                            surface,
                            root,
                            generation,
                            meta,
                            timestamp_ms: 0,
                        });
                    }
                }
            }
        }

        // 2) 回退：注入 JS 的交互元素快照。
        let snap = self.blocking(move |engine| engine.snapshot(tab)).await?;
        let tree = Self::snapshot_to_tree(tab, &snap);
        let (root, mut meta) = prune(tree, &opts);
        meta.stale = snap.meta.stale;
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let surface = SurfaceInfo {
            id: format!("web:{}", tab.as_u32()),
            kind: SurfaceKind::Web,
            title: snap.title.clone(),
            app: Some("browser".into()),
            pid: None,
            url: Some(snap.url.clone()),
            bounds: None,
            active: self.engine.active_tab() == Some(tab),
        };
        Ok(SurfaceSnapshot {
            surface,
            root,
            generation,
            meta,
            timestamp_ms: snap.timestamp_ms,
        })
    }

    async fn act(&self, ref_: &str, action: SurfaceAction) -> Result<()> {
        let (tab, local) = self.parse_ref(ref_)?;

        // 表面级动作（无元素）：Raise → 切换/激活标签页。
        if local == "root" || local.is_empty() {
            return match action {
                SurfaceAction::Raise => self.blocking(move |e| e.switch_tab(tab)).await,
                other => Err(EngineError::unsupported(format!(
                    "action '{}' not supported on browser surface root",
                    other.name()
                ))),
            };
        }

        // AX 引用（`ax:<backend_id>` / `axp:<path>`）：按几何做坐标动作。
        if local.starts_with("ax:") || local.starts_with("axp:") {
            return self.act_ax_ref(tab, &local, action).await;
        }

        let element = local_to_element_ref(&local)?;

        match action {
            SurfaceAction::Click | SurfaceAction::Invoke => {
                self.blocking(move |e| e.click_element(tab, &element)).await
            }
            SurfaceAction::DoubleClick => {
                self.blocking(move |e| e.double_click_element(tab, &element))
                    .await
            }
            SurfaceAction::RightClick => {
                self.blocking(move |e| e.right_click_element(tab, &element))
                    .await
            }
            SurfaceAction::SetValue { value } => {
                self.blocking(move |e| e.set_element_value(tab, &element, &value))
                    .await
            }
            SurfaceAction::TypeText { text, clear } => {
                self.blocking(move |e| e.type_text(tab, &element, &text, clear))
                    .await
            }
            SurfaceAction::Check { checked } => {
                self.blocking(move |e| e.check_element(tab, &element, checked))
                    .await
            }
            SurfaceAction::Select { value } => {
                self.blocking(move |e| e.select_option(tab, &element, &value))
                    .await
            }
            SurfaceAction::Focus => {
                // 解析出具体元素后页面内聚焦；失败退化为点击（保证可用）。
                let target = element.clone();
                let r = self
                    .blocking(move |e| {
                        let id = snapshot_id_of(e, tab, &target)?;
                        let v: Value =
                            e.evaluate(tab, &crate::engine::inject::action_js(id, "el.focus()"))?;
                        if v.as_str() == Some("notfound") {
                            Err(EngineError::new(ErrorKind::Dom, "element not found"))
                        } else {
                            Ok(())
                        }
                    })
                    .await;
                match r {
                    Ok(()) => Ok(()),
                    Err(_) => self.blocking(move |e| e.click_element(tab, &element)).await,
                }
            }
            SurfaceAction::Scroll { dx, dy } => {
                // 以元素中心为滚动点；无法解析元素时退化为视口中心（滚动本身不依赖元素）。
                let target = element.clone();
                let (cx, cy) = self
                    .blocking(move |e| {
                        if let Ok(id) = snapshot_id_of(e, tab, &target) {
                            if let Ok(snap) = e.snapshot(tab) {
                                if let Some(el) = snap.element_by_id(id) {
                                    return Ok(el.rect.center());
                                }
                            }
                        }
                        let vp = e
                            .snapshot(tab)
                            .map(|s| s.viewport)
                            .unwrap_or(crate::engine::Viewport::new(0, 0));
                        Ok((vp.width as f64 / 2.0, vp.height as f64 / 2.0))
                    })
                    .await?;
                let ev = InputEvent::Wheel(crate::engine::WheelEvent {
                    x: cx,
                    y: cy,
                    delta_x: dx,
                    delta_y: dy,
                });
                self.blocking(move |e| e.inject_event(tab, ev)).await
            }
            SurfaceAction::PressKey { key } => {
                let ev = InputEvent::Key(crate::engine::KeyEvent::press(key));
                self.blocking(move |e| e.inject_event(tab, ev)).await
            }
            other => Err(EngineError::unsupported(format!(
                "action '{}' not supported by browser surface",
                other.name()
            ))),
        }
    }

    async fn input(&self, target: &str, event: InputEvent) -> Result<()> {
        let tab = self.parse_tab(target)?;
        // Surface-level wheel (coords 0,0): scroll the document directly. A
        // synthesized wheel event often does not move the page on WebView hosts
        // (scrollY stays put), whereas `window.scrollBy` always does.
        if let InputEvent::Wheel(w) = &event {
            if w.x <= 0.0 && w.y <= 0.0 {
                let (dx, dy) = (w.delta_x, w.delta_y);
                let js = format!(
                    "(function(){{window.scrollBy({dx},{dy});return {{x:window.scrollX||0,y:window.scrollY||0}};}})()"
                );
                // Best-effort: fall back to wheel injection when the engine can't
                // evaluate JS (e.g. mock), instead of erroring the whole call.
                if self
                    .blocking(move |e| e.evaluate(tab, &js).map(|_| ()))
                    .await
                    .is_ok()
                {
                    return Ok(());
                }
            }
        }
        self.blocking(move |e| e.inject_event(tab, event)).await
    }

    async fn screenshot(&self, target: &str) -> Result<Image> {
        let tab = self.parse_tab(target)?;
        self.blocking(move |e| e.screenshot(tab)).await
    }
}

/// 便于测试：从引擎构造。
pub fn browser_surface(engine: Arc<dyn BrowserEngine>) -> BrowserSurface {
    BrowserSurface::new(engine)
}

#[allow(dead_code)]
fn _assert_rect(_r: Rect) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::SurfaceProvider;
    use crate::engine::TabOptions;
    use crate::engines::mock::MockEngine;

    fn provider() -> BrowserSurface {
        BrowserSurface::new(Arc::new(MockEngine::new()))
    }

    #[tokio::test]
    async fn list_and_snapshot_mock_engine() {
        let p = provider();
        p.engine
            .create_tab("https://example.com", &TabOptions::default())
            .unwrap();
        let surfaces = p.list_surfaces().await.unwrap();
        assert!(!surfaces.is_empty());
        assert!(surfaces[0].id.starts_with("web:"));
        let snap = p.snapshot("", SnapshotOptions::default()).await.unwrap();
        assert_eq!(snap.surface.kind, SurfaceKind::Web);
        assert!(snap.root.is_some());
        // mock 页面有可交互元素
        assert!(!snap.nodes().is_empty());
    }

    #[tokio::test]
    async fn click_by_ref_works() {
        let p = provider();
        p.engine
            .create_tab("https://example.com/login", &TabOptions::default())
            .unwrap();
        let snap = p.snapshot("", SnapshotOptions::default()).await.unwrap();
        let button = snap
            .nodes()
            .into_iter()
            .find(|n| n.role == "button")
            .map(|n| n.ref_.clone());
        if let Some(ref_) = button {
            p.act(&ref_, SurfaceAction::Click).await.unwrap();
        }
    }

    #[tokio::test]
    async fn set_value_by_ref() {
        let p = provider();
        p.engine
            .create_tab("https://example.com/login", &TabOptions::default())
            .unwrap();
        let snap = p.snapshot("", SnapshotOptions::default()).await.unwrap();
        let input = snap
            .nodes()
            .into_iter()
            .find(|n| n.role == "textbox")
            .map(|n| n.ref_.clone());
        if let Some(ref_) = input {
            p.act(
                &ref_,
                SurfaceAction::SetValue {
                    value: "hello".into(),
                },
            )
            .await
            .unwrap();
        }
    }

    #[tokio::test]
    async fn bad_ref_errors() {
        let p = provider();
        let err = p.act("nope", SurfaceAction::Click).await.unwrap_err();
        assert_eq!(err.kind, ErrorKind::InvalidArgument);
    }

    #[test]
    fn parse_ref_shapes() {
        let p = provider();
        assert_eq!(p.parse_ref("web:3:a").unwrap(), (TabId(3), "a".into()));
        assert_eq!(
            p.parse_ref("web:3:frame0:b").unwrap(),
            (TabId(3), "frame0:b".into())
        );
        assert!(p.parse_ref("desktop:1:a").is_err());
    }

    #[test]
    fn role_mapping() {
        assert_eq!(role_for_tag("input", Some("checkbox")), "checkbox");
        assert_eq!(role_for_tag("a", None), "link");
        assert_eq!(role_for_tag("select", None), "combobox");
    }

    #[test]
    fn local_dialect_to_element_ref() {
        assert_eq!(
            local_to_element_ref("a").unwrap(),
            ElementRef::snapshot('a')
        );
        // Playwright MCP 风格 eN（1-based）
        assert_eq!(
            local_to_element_ref("e1").unwrap(),
            ElementRef::snapshot('a')
        );
        assert_eq!(
            local_to_element_ref("e5").unwrap(),
            ElementRef::snapshot('e')
        );
        assert_eq!(
            local_to_element_ref("css=#go").unwrap(),
            ElementRef::css("#go")
        );
        assert_eq!(
            local_to_element_ref("text=Go").unwrap(),
            ElementRef::text("Go")
        );
        assert_eq!(
            local_to_element_ref("role=button").unwrap(),
            ElementRef::role("button")
        );
        assert_eq!(
            local_to_element_ref("//button").unwrap(),
            ElementRef::xpath("//button")
        );
        assert_eq!(local_to_element_ref("#go").unwrap(), ElementRef::css("#go"));
        assert_eq!(
            local_to_element_ref("id=go").unwrap(),
            ElementRef::css("#go")
        );
        assert_eq!(
            local_to_element_ref("data-testid=go").unwrap(),
            ElementRef::css("[data-testid=\"go\"]")
        );
        assert_eq!(
            local_to_element_ref("nth=2").unwrap(),
            ElementRef::snapshot('c')
        );
        assert!(local_to_element_ref("").is_err());
    }

    #[tokio::test]
    async fn dialect_click_and_input_on_mock() {
        let p = provider();
        let tab = p
            .engine
            .create_tab("https://example.com/login", &TabOptions::default())
            .unwrap();
        // role 方言
        p.act(
            &format!("web:{}:role=button", tab.as_u32()),
            SurfaceAction::Click,
        )
        .await
        .unwrap();
        // 文本方言（登录页有 "Submit"）
        p.act(
            &format!("web:{}:text=Submit", tab.as_u32()),
            SurfaceAction::Click,
        )
        .await
        .unwrap();
        // 坐标输入
        p.input(
            &format!("web:{}", tab.as_u32()),
            InputEvent::Key(crate::engine::KeyEvent::press("Tab")),
        )
        .await
        .unwrap();
        // 截图
        let img = p
            .screenshot(&format!("web:{}", tab.as_u32()))
            .await
            .unwrap();
        assert!(img.is_valid());
    }

    #[tokio::test]
    async fn surface_level_raise_and_scroll() {
        let p = provider();
        let tab = p
            .engine
            .create_tab("https://example.com", &TabOptions::default())
            .unwrap();
        p.act(&format!("web:{}:root", tab.as_u32()), SurfaceAction::Raise)
            .await
            .unwrap();
        // 方言目标滚动（mock 可解析 role=button → 元素中心）
        p.act(
            &format!("web:{}:role=button", tab.as_u32()),
            SurfaceAction::Scroll { dx: 0.0, dy: 100.0 },
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn unsupported_action_errors() {
        let p = provider();
        let tab = p
            .engine
            .create_tab("https://example.com", &TabOptions::default())
            .unwrap();
        let err = p
            .act(&format!("web:{}:a", tab.as_u32()), SurfaceAction::Increment)
            .await
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::Unsupported);
    }
}
