// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! 统一「表面（Surface）」模型 —— 电脑操作内核的唯一 UI 规范。
//!
//! 一个 *surface* 是「可被感知与操作的一块界面」：浏览器标签页、原生 App 窗口、
//! 桌面、甚至命令/文件（与 fastshell 并列）。无论底层是 CDP 无障碍树、macOS
//! AXUIElement、Windows UIA 还是脚本化 mock，最终都产出同一形状的
//! [`UiNode`] 树与 [`SurfaceSnapshot`]。
//!
//! 本模块**零平台依赖**，因此永远参与编译（无需任何 feature）。异步 provider
//! 契约见 `engine::provider`（feature `surface`）。
//!
//! 设计要点：
//! - **单一规范**：浏览器的 `PageSnapshot` 是 `SurfaceSnapshot` 的 web 视图，
//!   不得独立演化出第二套元素/引用模型。
//! - **稳定引用**：`UiNode.ref_` 是跨调用可回退的字符串引用（世代号由 provider
//!   决定），Agent 只用它定位元素。
//! - **token 友好**：`prune` 默认做 interestingOnly + 深度/节点上限 + 字段截断，
//!   并显式报告截断原因（见 [`SnapshotMeta`]）。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::engine::view::Rect;

/// 表面类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SurfaceKind {
    /// 浏览器页面（web / DOM 无障碍树）。
    Web,
    /// 原生应用（其无障碍根）。
    App,
    /// 原生窗口。
    Window,
    /// 桌面（系统级）。
    Desktop,
    /// 未知 / 未分类。
    #[default]
    Unknown,
}

impl SurfaceKind {
    /// 用作 ref 命名空间前缀（`web:` / `desktop:` …）。
    pub fn namespace(&self) -> &'static str {
        match self {
            SurfaceKind::Web => "web",
            SurfaceKind::App => "app",
            SurfaceKind::Window => "window",
            SurfaceKind::Desktop => "desktop",
            SurfaceKind::Unknown => "surface",
        }
    }
}

/// 表面 provider 的能力位（工具层据此收敛；与 `EngineCapabilities` 正交）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SurfaceCapabilities {
    /// 原生无障碍树（AXUIElement / UIA / AT-SPI）。
    pub native_ax: bool,
    /// 坐标级输入注入（CGEvent / SendInput / uinput）。
    pub coordinate_input: bool,
    /// 截图。
    pub screenshot: bool,
    /// 窗口/应用管理（列举、激活、置前）。
    pub window_management: bool,
}

impl SurfaceCapabilities {
    pub fn none() -> Self {
        SurfaceCapabilities::default()
    }
    /// web 表面：有 AX（来自 CDP/JS 注入）、可坐标输入（CDP Input 域）。
    pub fn web() -> Self {
        SurfaceCapabilities {
            native_ax: true,
            coordinate_input: true,
            screenshot: true,
            window_management: true,
        }
    }
    /// 合并两个能力集（注册表汇总用）。
    pub fn merge(self, other: SurfaceCapabilities) -> Self {
        SurfaceCapabilities {
            native_ax: self.native_ax || other.native_ax,
            coordinate_input: self.coordinate_input || other.coordinate_input,
            screenshot: self.screenshot || other.screenshot,
            window_management: self.window_management || other.window_management,
        }
    }
}

/// 一个可操作的表面（浏览器标签 / 原生窗口 / 应用）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SurfaceInfo {
    /// 全局唯一 id，同时是 ref 命名空间的一部分，如 `web:3` / `desktop:1234`。
    pub id: String,
    pub kind: SurfaceKind,
    /// 人类可读标题。
    pub title: String,
    /// 所属应用名（原生）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    /// 进程 id（原生）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<i32>,
    /// 页面 URL（web）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// 屏幕全局边界。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bounds: Option<Rect>,
    /// 是否为当前活动表面。
    #[serde(default)]
    pub active: bool,
}

impl SurfaceInfo {
    /// 该表面下所有 ref 的前缀（含结尾冒号），如 `web:3:`。
    pub fn ref_prefix(&self) -> String {
        format!("{}:", self.id)
    }
}

/// 元素状态（统一自各平台的无障碍状态）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct UiState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expanded: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focused: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub editable: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub busy: Option<bool>,
}

impl UiState {
    pub fn is_empty(&self) -> bool {
        self.checked.is_none()
            && self.disabled.is_none()
            && self.selected.is_none()
            && self.expanded.is_none()
            && self.focused.is_none()
            && self.editable.is_none()
            && self.required.is_none()
            && self.busy.is_none()
    }
}

/// 统一 UI 节点（web 与原生共用）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiNode {
    /// provider 局部 id（用于在单次快照内定位；不保证跨快照稳定）。
    pub node_id: String,
    /// **稳定引用**：Agent 用它操作元素。形如 `web:3:a` / `desktop:1234:0.2.1`。
    #[serde(rename = "ref")]
    pub ref_: String,
    /// 统一角色词表（button / textbox / window / menuitem …）。
    pub role: String,
    /// 无障碍名称（不是 innerText）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// 当前值（输入框文本、滑块值等）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// 描述 / help 文本。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "UiState::is_empty")]
    pub states: UiState,
    /// 屏幕全局坐标。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub geometry: Option<Rect>,
    /// 本节点支持的动作（invoke / press / set_value / focus …）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<String>,
    /// 来源表面类型。
    #[serde(default)]
    pub source: SurfaceKind,
    /// 后端私有句柄（CSS / DOM backend id / AX path），供 provider 解析。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<String>,
    /// 额外属性（平台相关，谨慎使用以免 token 膨胀）。
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attrs: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<UiNode>,
}

impl UiNode {
    pub fn new(
        node_id: impl Into<String>,
        ref_: impl Into<String>,
        role: impl Into<String>,
    ) -> Self {
        UiNode {
            node_id: node_id.into(),
            ref_: ref_.into(),
            role: role.into(),
            name: None,
            value: None,
            description: None,
            states: UiState::default(),
            geometry: None,
            actions: Vec::new(),
            source: SurfaceKind::Unknown,
            backend: None,
            attrs: BTreeMap::new(),
            children: Vec::new(),
        }
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        let s = name.into();
        if !s.is_empty() {
            self.name = Some(s);
        }
        self
    }

    pub fn with_value(mut self, value: impl Into<String>) -> Self {
        let s = value.into();
        if !s.is_empty() {
            self.value = Some(s);
        }
        self
    }

    pub fn with_geometry(mut self, rect: Rect) -> Self {
        self.geometry = Some(rect);
        self
    }

    pub fn with_source(mut self, kind: SurfaceKind) -> Self {
        self.source = kind;
        self
    }

    /// 深度优先展开为扁平列表（含自身）。
    pub fn flatten(&self) -> Vec<&UiNode> {
        let mut out = Vec::new();
        self.collect(&mut out);
        out
    }

    fn collect<'a>(&'a self, out: &mut Vec<&'a UiNode>) {
        out.push(self);
        for c in &self.children {
            c.collect(out);
        }
    }

    /// 节点总数（含自身）。
    pub fn node_count(&self) -> usize {
        1 + self.children.iter().map(UiNode::node_count).sum::<usize>()
    }
}

/// 快照生成参数（token 预算的核心旋钮）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotOptions {
    /// 只保留「有意义」的节点（丢弃无 name/value/action 的纯容器）。
    pub interesting_only: bool,
    /// 最大树深。
    pub max_depth: usize,
    /// 最大返回节点数。
    pub max_nodes: usize,
    /// 单个文本字段（name/value/description）最大字符数（0 = 不限）。
    pub max_text_len: usize,
}

impl Default for SnapshotOptions {
    fn default() -> Self {
        SnapshotOptions {
            interesting_only: true,
            max_depth: 12,
            max_nodes: 300,
            max_text_len: 200,
        }
    }
}

/// 快照元信息（截断/过期必须显式可见）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SnapshotMeta {
    /// 遍历到的节点总数（含被裁剪掉的）。
    pub total_nodes: usize,
    /// 实际返回的节点数。
    pub returned_nodes: usize,
    /// 实际到达的最大深度。
    pub max_depth_reached: usize,
    /// 是否发生截断。
    pub truncated: bool,
    /// 截断原因："nodes" | "depth" | "timeout" | "stale"。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// 快照是否可能过期（页面忙/导航中）。
    #[serde(default)]
    pub stale: bool,
}

/// 一次表面快照。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SurfaceSnapshot {
    pub surface: SurfaceInfo,
    /// 树根（可能因裁剪为空）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<UiNode>,
    /// 快照世代号（provider 维护；ref 在下次同表面快照前有效）。
    pub generation: u64,
    #[serde(default)]
    pub meta: SnapshotMeta,
    /// 快照时间戳（毫秒）。
    #[serde(default)]
    pub timestamp_ms: u64,
}

impl SurfaceSnapshot {
    /// 按 ref 查节点。
    pub fn node_by_ref(&self, ref_: &str) -> Option<&UiNode> {
        self.root
            .as_ref()
            .and_then(|r| r.flatten().into_iter().find(|n| n.ref_ == ref_))
    }

    /// 扁平节点列表。
    pub fn nodes(&self) -> Vec<&UiNode> {
        self.root.as_ref().map(UiNode::flatten).unwrap_or_default()
    }

    /// 生成给 LLM 的紧凑文本（YAML 风格，类 Playwright aria snapshot）。
    pub fn to_llm_text(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "surface: {} ({})\ntitle: {}\n",
            self.surface.id,
            self.surface.kind.namespace(),
            self.surface.title
        ));
        if let Some(url) = &self.surface.url {
            out.push_str(&format!("url: {url}\n"));
        }
        if self.meta.truncated {
            out.push_str(&format!(
                "truncated: true (reason={:?}, total={})\n",
                self.meta.reason, self.meta.total_nodes
            ));
        }
        if let Some(root) = &self.root {
            render_node(root, 0, &mut out);
        }
        out
    }
}

fn render_node(node: &UiNode, indent: usize, out: &mut String) {
    let pad = "  ".repeat(indent);
    let name = node.name.as_deref().unwrap_or("");
    let mut line = format!("{pad}- {}", node.role);
    if !name.is_empty() {
        line.push_str(&format!(" \"{name}\""));
    }
    if let Some(v) = &node.value {
        if !v.is_empty() {
            line.push_str(&format!(" = \"{v}\""));
        }
    }
    let mut flags: Vec<String> = Vec::new();
    if node.states.disabled == Some(true) {
        flags.push("disabled".into());
    }
    if node.states.checked == Some(true) {
        flags.push("checked".into());
    }
    if node.states.selected == Some(true) {
        flags.push("selected".into());
    }
    if node.states.expanded == Some(true) {
        flags.push("expanded".into());
    }
    if node.states.focused == Some(true) {
        flags.push("focused".into());
    }
    if !flags.is_empty() {
        line.push_str(&format!(" [{}]", flags.join(",")));
    }
    line.push_str(&format!(" @{}", node.ref_));
    out.push_str(&line);
    out.push('\n');
    for c in &node.children {
        render_node(c, indent + 1, out);
    }
}

/// 一个可施加于元素或表面的动作（统一自各平台）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SurfaceAction {
    /// 单击 / 按下（AXPress、CDP click）。
    Click,
    DoubleClick,
    RightClick,
    /// 聚焦。
    Focus,
    /// 设置值（输入框、滑块）。
    SetValue {
        value: String,
    },
    /// 键入文本（真实输入通道）。
    TypeText {
        text: String,
        clear: bool,
    },
    /// 勾选/取消勾选。
    Check {
        checked: bool,
    },
    /// 下拉选择。
    Select {
        value: String,
    },
    /// 滚动（作用于元素或表面）。
    Scroll {
        dx: f64,
        dy: f64,
    },
    /// 按键（Enter、Tab、Escape…）。
    PressKey {
        key: String,
    },
    /// 触发默认动作（AXPress 的通用化）。
    Invoke,
    Increment,
    Decrement,
    /// 展开菜单。
    ShowMenu,
    /// 置前 / 激活窗口。
    Raise,
}

impl SurfaceAction {
    /// 动作名（用于审计与能力判断）。
    pub fn name(&self) -> &'static str {
        match self {
            SurfaceAction::Click => "click",
            SurfaceAction::DoubleClick => "double_click",
            SurfaceAction::RightClick => "right_click",
            SurfaceAction::Focus => "focus",
            SurfaceAction::SetValue { .. } => "set_value",
            SurfaceAction::TypeText { .. } => "type_text",
            SurfaceAction::Check { .. } => "check",
            SurfaceAction::Select { .. } => "select",
            SurfaceAction::Scroll { .. } => "scroll",
            SurfaceAction::PressKey { .. } => "press_key",
            SurfaceAction::Invoke => "invoke",
            SurfaceAction::Increment => "increment",
            SurfaceAction::Decrement => "decrement",
            SurfaceAction::ShowMenu => "show_menu",
            SurfaceAction::Raise => "raise",
        }
    }
}

// ─────────────────────────── 裁剪（token 预算）───────────────────────────

/// 裁剪一个节点树，返回（保留的树, 元信息）。
///
/// 规则：
/// 1. 先做字段截断（`max_text_len`）；
/// 2. 后序裁剪：`interesting_only` 时丢弃「无 name/value/action/状态且角色为
///    纯容器」的节点，但若其子树保留了节点则保留它作为结构；
/// 3. 深度超过 `max_depth` 的子树整体丢弃，标记 `reason=depth`；
/// 4. 返回节点数达到 `max_nodes` 后停止，标记 `reason=nodes`。
pub fn prune(root: UiNode, opts: &SnapshotOptions) -> (Option<UiNode>, SnapshotMeta) {
    let mut state = PruneState {
        opts,
        total: 0,
        returned: 0,
        max_depth: 0,
        truncated: false,
        reason: None,
    };
    let out = state.prune_node(root, 0);
    let meta = SnapshotMeta {
        total_nodes: state.total,
        returned_nodes: state.returned,
        max_depth_reached: state.max_depth,
        truncated: state.truncated,
        reason: state.reason,
        stale: false,
    };
    (out, meta)
}

struct PruneState<'a> {
    opts: &'a SnapshotOptions,
    total: usize,
    returned: usize,
    max_depth: usize,
    truncated: bool,
    reason: Option<String>,
}

impl<'a> PruneState<'a> {
    fn mark(&mut self, reason: &str) {
        self.truncated = true;
        if self.reason.is_none() {
            self.reason = Some(reason.to_string());
        }
    }

    fn prune_node(&mut self, mut node: UiNode, depth: usize) -> Option<UiNode> {
        self.total += 1;
        self.max_depth = self.max_depth.max(depth);
        truncate_field(&mut node.name, self.opts.max_text_len);
        truncate_field(&mut node.value, self.opts.max_text_len);
        truncate_field(&mut node.description, self.opts.max_text_len);

        if self.returned >= self.opts.max_nodes {
            self.mark("nodes");
            return None;
        }

        let keep_self = !self.opts.interesting_only || is_interesting(&node);
        // 先为自己预留一个名额（后序裁剪下，父节点必须先于子节点计数，
        // 否则会在恰好达到上限时多返回一个节点）。
        self.returned += 1;

        let mut kept_children = Vec::new();
        if depth < self.opts.max_depth {
            let children = std::mem::take(&mut node.children);
            for child in children {
                if let Some(k) = self.prune_node(child, depth + 1) {
                    kept_children.push(k);
                }
            }
        } else if !node.children.is_empty() {
            self.mark("depth");
        }
        node.children = kept_children;

        if keep_self || !node.children.is_empty() {
            Some(node)
        } else {
            // 自己与子树都被裁掉，释放预留名额。
            self.returned -= 1;
            None
        }
    }
}

fn truncate_field(field: &mut Option<String>, max: usize) {
    if max == 0 {
        return;
    }
    if let Some(s) = field {
        if s.chars().count() > max {
            let truncated: String = s.chars().take(max).collect();
            *s = format!("{truncated}…");
        }
    }
}

/// 节点是否「有意义」（interestingOnly 的判据）。
pub fn is_interesting(node: &UiNode) -> bool {
    if node.name.as_deref().map(|s| !s.is_empty()).unwrap_or(false) {
        return true;
    }
    if node
        .value
        .as_deref()
        .map(|s| !s.is_empty())
        .unwrap_or(false)
    {
        return true;
    }
    if !node.actions.is_empty() {
        return true;
    }
    if node.states.focused == Some(true) {
        return true;
    }
    !matches!(
        node.role.as_str(),
        "" | "generic" | "none" | "presentation" | "unknown" | "group" | "ignored"
    )
}

/// 原生事件类型（焦点/窗口/值/标题/选择/结构变化）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SurfaceEventKind {
    FocusChanged,
    WindowCreated,
    WindowClosed,
    ValueChanged,
    TitleChanged,
    SelectionChanged,
    StructureChanged,
    Unknown,
}

/// 一条原生表面事件。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SurfaceEvent {
    pub kind: SurfaceEventKind,
    /// 事件来源表面 id（如 `desktop:1234`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surface: Option<String>,
    /// 相关元素 ref（若可得）。
    #[serde(rename = "ref", default, skip_serializing_if = "Option::is_none")]
    pub ref_: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// 事件时间戳（毫秒，Unix epoch）。
    #[serde(default)]
    pub timestamp_ms: u64,
}

impl SurfaceEvent {
    pub fn new(kind: SurfaceEventKind) -> Self {
        SurfaceEvent {
            kind,
            surface: None,
            ref_: None,
            role: None,
            name: None,
            value: None,
            timestamp_ms: now_ms(),
        }
    }

    pub fn with_surface(mut self, surface: impl Into<String>) -> Self {
        self.surface = Some(surface.into());
        self
    }

    pub fn with_element(mut self, role: Option<String>, name: Option<String>) -> Self {
        self.role = role.filter(|s| !s.is_empty());
        self.name = name.filter(|s| !s.is_empty());
        self
    }
}

/// 当前 Unix 毫秒时间戳。
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 从 ref 中解析命名空间（首个 `:` 之前），无则返回空串。
pub fn ref_namespace(ref_: &str) -> &str {
    match ref_.find(':') {
        Some(i) => &ref_[..i],
        None => "",
    }
}

/// 从 ref 中解析表面 id（`web:3:a` → `web:3`），无则返回整个串。
pub fn ref_surface_id(ref_: &str) -> &str {
    // 命名空间 + 冒号 + 表面号 → 取前两段。
    let mut it = ref_.splitn(3, ':');
    match (it.next(), it.next()) {
        (Some(ns), Some(id)) if !ns.is_empty() && !id.is_empty() => {
            let ns_len = ns.len();
            let id_len = id.len();
            &ref_[..ns_len + 1 + id_len]
        }
        _ => ref_,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf(id: &str, role: &str, name: &str) -> UiNode {
        UiNode::new(id, format!("web:1:{id}"), role)
            .with_name(name)
            .with_source(SurfaceKind::Web)
    }

    fn sample_tree() -> UiNode {
        let mut root = UiNode::new("r", "web:1:r", "webarea")
            .with_name("Example")
            .with_source(SurfaceKind::Web);
        let mut container = UiNode::new("c", "web:1:c", "group"); // 无 name → 容器
        container.children.push(leaf("a", "button", "Submit"));
        container
            .children
            .push(UiNode::new("b", "web:1:b", "button")); // 无 name
        root.children.push(container);
        root
    }

    #[test]
    fn surface_kind_namespace() {
        assert_eq!(SurfaceKind::Web.namespace(), "web");
        assert_eq!(SurfaceKind::Desktop.namespace(), "desktop");
    }

    #[test]
    fn prune_drops_empty_container_but_keeps_structure() {
        let (tree, meta) = prune(sample_tree(), &SnapshotOptions::default());
        let tree = tree.unwrap();
        // root + container(有子树) + a(button 有 name)；b 无 name 且是 button 角色 → 保留？
        // button 不是纯容器，is_interesting=true，因此 b 也保留。
        let refs: Vec<&str> = tree.flatten().iter().map(|n| n.ref_.as_str()).collect();
        assert!(refs.contains(&"web:1:r"));
        assert!(refs.contains(&"web:1:a"));
        assert!(refs.contains(&"web:1:b"));
        assert!(!meta.truncated);
    }

    #[test]
    fn prune_drops_pure_empty_group() {
        let mut root = UiNode::new("r", "web:1:r", "webarea").with_name("X");
        root.children.push(UiNode::new("g", "web:1:g", "group")); // 空 group
        let (tree, _) = prune(root, &SnapshotOptions::default());
        let tree = tree.unwrap();
        let refs: Vec<&str> = tree.flatten().iter().map(|n| n.ref_.as_str()).collect();
        assert!(!refs.contains(&"web:1:g"), "empty group should be pruned");
    }

    #[test]
    fn prune_respects_max_nodes() {
        let mut root = UiNode::new("r", "web:1:r", "webarea").with_name("X");
        for i in 0..50 {
            root.children
                .push(leaf(&format!("n{i}"), "button", &format!("B{i}")));
        }
        let opts = SnapshotOptions {
            max_nodes: 10,
            ..SnapshotOptions::default()
        };
        let (tree, meta) = prune(root, &opts);
        assert!(tree.unwrap().node_count() <= 10);
        assert!(meta.truncated);
        assert_eq!(meta.reason.as_deref(), Some("nodes"));
    }

    #[test]
    fn prune_respects_max_depth() {
        let mut root = UiNode::new("r", "web:1:r", "webarea").with_name("X");
        let mut cur = &mut root;
        for i in 0..30 {
            let child = UiNode::new(format!("d{i}"), format!("web:1:d{i}"), "group");
            cur.children.push(child);
            cur = cur.children.last_mut().unwrap();
        }
        let opts = SnapshotOptions {
            max_depth: 3,
            interesting_only: false,
            ..SnapshotOptions::default()
        };
        let (tree, meta) = prune(root, &opts);
        let max_depth = tree.unwrap().flatten().len();
        assert!(max_depth <= 4, "got {max_depth} nodes for depth 3");
        assert!(meta.truncated);
        assert_eq!(meta.reason.as_deref(), Some("depth"));
    }

    #[test]
    fn prune_truncates_long_fields() {
        let long = "x".repeat(500);
        let root = UiNode::new("r", "web:1:r", "button").with_name(long);
        let opts = SnapshotOptions {
            max_text_len: 20,
            ..SnapshotOptions::default()
        };
        let (tree, _) = prune(root, &opts);
        let name = tree.unwrap().name.unwrap();
        assert!(name.chars().count() <= 21); // 20 + '…'
                                             // 纯容器（group）且无 name/value/action → 不 interesting。
        let boring = UiNode::new("g", "web:1:g", "group");
        assert!(!is_interesting(&boring));
        // 有 name 的容器仍然 interesting。
        let named = UiNode::new("b", "web:1:b", "group").with_name("X");
        assert!(is_interesting(&named));
        // button 角色即使无 name 也 interesting（可交互）。
        let button = UiNode::new("c", "web:1:c", "button");
        assert!(is_interesting(&button));
    }

    #[test]
    fn ref_helpers() {
        assert_eq!(ref_namespace("web:3:a"), "web");
        assert_eq!(ref_namespace("desktop:1234:0.2.1"), "desktop");
        assert_eq!(ref_namespace("bare"), "");
        assert_eq!(ref_surface_id("web:3:a"), "web:3");
        assert_eq!(ref_surface_id("desktop:1234:0.2.1"), "desktop:1234");
        assert_eq!(ref_surface_id("bare"), "bare");
    }

    #[test]
    fn llm_text_renders_tree() {
        let (tree, meta) = prune(sample_tree(), &SnapshotOptions::default());
        let snap = SurfaceSnapshot {
            surface: SurfaceInfo {
                id: "web:1".into(),
                kind: SurfaceKind::Web,
                title: "Example".into(),
                app: None,
                pid: None,
                url: Some("https://example.com".into()),
                bounds: None,
                active: true,
            },
            root: tree,
            generation: 1,
            meta,
            timestamp_ms: 0,
        };
        let text = snap.to_llm_text();
        assert!(text.contains("surface: web:1 (web)"));
        assert!(text.contains("button \"Submit\""));
        assert!(snap.node_by_ref("web:1:a").is_some());
        assert_eq!(snap.nodes().len(), 4);
    }

    #[test]
    fn capabilities_merge() {
        let a = SurfaceCapabilities {
            native_ax: true,
            ..Default::default()
        };
        let b = SurfaceCapabilities {
            coordinate_input: true,
            ..Default::default()
        };
        let m = a.merge(b);
        assert!(m.native_ax && m.coordinate_input);
        assert!(!m.screenshot);
    }

    #[test]
    fn action_names() {
        assert_eq!(SurfaceAction::Click.name(), "click");
        assert_eq!(
            SurfaceAction::SetValue { value: "x".into() }.name(),
            "set_value"
        );
    }

    #[test]
    fn ui_node_serde_omits_empty_collections() {
        let n = UiNode::new("a", "web:1:a", "button").with_name("Go");
        let v = serde_json::to_value(&n).unwrap();
        assert_eq!(v["ref"], "web:1:a");
        assert_eq!(v["role"], "button");
        assert_eq!(v["name"], "Go");
        assert!(v.get("states").is_none(), "empty states must be omitted");
        assert!(v.get("attrs").is_none(), "empty attrs must be omitted");
        assert!(
            v.get("children").is_none(),
            "empty children must be omitted"
        );
    }

    #[test]
    fn surface_action_serde_roundtrip() {
        let cases = [
            SurfaceAction::Click,
            SurfaceAction::DoubleClick,
            SurfaceAction::Focus,
            SurfaceAction::SetValue { value: "x".into() },
            SurfaceAction::TypeText {
                text: "y".into(),
                clear: true,
            },
            SurfaceAction::Check { checked: false },
            SurfaceAction::Select { value: "s".into() },
            SurfaceAction::Scroll { dx: 1.0, dy: 2.0 },
            SurfaceAction::PressKey { key: "Tab".into() },
            SurfaceAction::Increment,
            SurfaceAction::Raise,
        ];
        for a in cases {
            let j = serde_json::to_value(&a).unwrap();
            let back: SurfaceAction = serde_json::from_value(j).unwrap();
            assert_eq!(a, back);
        }
        assert_eq!(
            serde_json::to_value(SurfaceAction::Click).unwrap()["kind"],
            "click"
        );
        let j = serde_json::to_value(SurfaceAction::SetValue { value: "x".into() }).unwrap();
        assert_eq!(j["kind"], "set_value");
        assert_eq!(j["value"], "x");
    }

    #[test]
    fn prune_no_text_cap_when_zero() {
        let long = "z".repeat(500);
        let root = UiNode::new("r", "web:1:r", "button").with_name(long.clone());
        let opts = SnapshotOptions {
            max_text_len: 0,
            ..SnapshotOptions::default()
        };
        let (tree, _) = prune(root, &opts);
        assert_eq!(tree.unwrap().name.unwrap(), long);
    }

    #[test]
    fn node_count_matches_flatten() {
        let (tree, _) = prune(sample_tree(), &SnapshotOptions::default());
        let tree = tree.unwrap();
        assert_eq!(tree.node_count(), tree.flatten().len());
    }

    #[test]
    fn interesting_only_false_keeps_containers() {
        let mut root = UiNode::new("r", "web:1:r", "webarea").with_name("X");
        root.children.push(UiNode::new("g", "web:1:g", "group"));
        let opts = SnapshotOptions {
            interesting_only: false,
            ..SnapshotOptions::default()
        };
        let (tree, _) = prune(root, &opts);
        let tree = tree.unwrap();
        let refs: Vec<&str> = tree.flatten().iter().map(|n| n.ref_.as_str()).collect();
        assert!(refs.contains(&"web:1:g"));
    }

    #[test]
    fn ref_helpers_edge_cases() {
        assert_eq!(ref_namespace("web"), "");
        assert_eq!(ref_namespace(""), "");
        assert_eq!(ref_surface_id("web"), "web");
        assert_eq!(ref_surface_id("web:"), "web:");
    }

    #[test]
    fn surface_event_serde_and_helpers() {
        let e = SurfaceEvent::new(SurfaceEventKind::ValueChanged)
            .with_surface("desktop:1")
            .with_element(Some("textbox".into()), Some("Name".into()));
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["kind"], "value_changed");
        assert_eq!(v["surface"], "desktop:1");
        assert_eq!(v["role"], "textbox");
        assert_eq!(v["name"], "Name");
        assert!(v["timestamp_ms"].as_u64().unwrap() > 0);
        let back: SurfaceEvent = serde_json::from_value(v).unwrap();
        assert_eq!(back, e);
        // 空字段被省略
        let bare = serde_json::to_value(SurfaceEvent::new(SurfaceEventKind::Unknown)).unwrap();
        assert!(bare.get("surface").is_none());
        assert!(bare.get("ref").is_none());
    }
}
