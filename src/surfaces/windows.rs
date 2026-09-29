// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Windows 原生无障碍后端（feature `surface-windows`，UI Automation）。
//!
//! 通过 `uiautomation`（MS UI Automation）感知与操作原生窗口/控件：
//! 角色（ControlType）、名称、几何（BoundingRectangle）、启用/焦点状态；
//! 动作走 UI Automation 模式（Invoke/Value/Toggle）或元素级 click/focus。
//!
//! 命名空间 `desktop`，ref 形如 `desktop:<topIndex>:<child.path>`（`0.2.1`）。
//! 仅在 Windows 运行；其他平台可交叉 `cargo check` 但不注册。COM 在每次阻塞
//! 调用内初始化（`UIAutomation::new`），避免跨线程公寓问题。

use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;

use uiautomation::patterns::{UIInvokePattern, UITogglePattern, UIValuePattern};
use uiautomation::types::ControlType;
use uiautomation::{UIAutomation, UIElement, UITreeWalker};

use crate::engine::surface::{
    prune, SnapshotOptions, SurfaceAction, SurfaceCapabilities, SurfaceInfo, SurfaceKind,
    SurfaceSnapshot, UiNode,
};
use crate::engine::{EngineError, ErrorKind, InputEvent, Rect, Result};

/// 单次遍历的硬上限。
const AX_NODE_CAP: usize = 2000;

/// Windows 原生表面 provider。
pub struct WindowsSurface {
    generation: AtomicU64,
}

impl Default for WindowsSurface {
    fn default() -> Self {
        Self::new()
    }
}

impl WindowsSurface {
    pub fn new() -> Self {
        WindowsSurface {
            generation: AtomicU64::new(0),
        }
    }

    /// 在阻塞池上执行 UIA 操作（COM 线程内初始化）。
    async fn blocking<T, F>(f: F) -> Result<T>
    where
        F: FnOnce() -> Result<T> + Send + 'static,
        T: Send + 'static,
    {
        tokio::task::spawn_blocking(f).await.map_err(|e| {
            EngineError::new(ErrorKind::Internal, format!("windows surface join: {e}"))
        })?
    }

    /// 解析目标为顶层窗口下标：`desktop:2` / `2` / 空（首个）。
    fn parse_top(target: &str) -> Option<usize> {
        let t = target.trim();
        if t.is_empty() || t == "desktop" || t == "focused" {
            return Some(0);
        }
        let rest = t.strip_prefix("desktop:").unwrap_or(t);
        rest.split(':').next()?.parse().ok()
    }

    /// 解析 ref 为（顶层下标, 子路径）。
    fn parse_ref(ref_: &str) -> Result<(usize, Vec<usize>)> {
        let parts: Vec<&str> = ref_.splitn(3, ':').collect();
        if parts.len() < 2 || parts[0] != "desktop" {
            return Err(EngineError::invalid(format!(
                "bad desktop ref '{ref_}' (expected desktop:<top>:<path>)"
            )));
        }
        let top: usize = parts[1]
            .parse()
            .map_err(|_| EngineError::invalid(format!("bad top index in ref '{ref_}'")))?;
        let path = parts
            .get(2)
            .map(|s| {
                s.split('.')
                    .filter_map(|x| x.parse::<usize>().ok())
                    .collect()
            })
            .unwrap_or_default();
        Ok((top, path))
    }
}

fn automation() -> Result<UIAutomation> {
    UIAutomation::new()
        .map_err(|e| EngineError::new(ErrorKind::Plugin, format!("UIAutomation init: {e}")))
}

fn walker(auto: &UIAutomation) -> Result<UITreeWalker> {
    auto.get_control_view_walker()
        .map_err(|e| EngineError::new(ErrorKind::Plugin, format!("UIAutomation walker: {e}")))
}

fn top_levels(auto: &UIAutomation, walker: &UITreeWalker) -> Result<Vec<UIElement>> {
    let root = auto
        .get_root_element()
        .map_err(|e| EngineError::new(ErrorKind::Plugin, format!("UIA root: {e}")))?;
    Ok(walker.get_children(&root).unwrap_or_default())
}

fn control_role(ct: ControlType) -> String {
    match format!("{ct:?}").to_ascii_lowercase().as_str() {
        "edit" => "textbox",
        "hyperlink" => "link",
        "radiobutton" => "radio",
        "combobox" => "combobox",
        "tabitem" => "tab",
        "spinner" => "spinbutton",
        "datagrid" => "table",
        "dataitem" => "cell",
        "custom" => "generic",
        other => other,
    }
    .to_string()
}

fn actions_for_role(role: &str) -> Vec<String> {
    match role {
        "button" | "link" | "menuitem" | "tab" | "listitem" | "treeitem" => vec!["click".into()],
        "checkbox" | "radio" => vec!["click".into(), "check".into()],
        "textbox" | "combobox" | "spinner" => vec!["click".into(), "set_value".into()],
        "slider" => vec!["set_value".into()],
        _ => Vec::new(),
    }
}

/// 读取单个元素的统一节点（不含子节点）。
fn node_data(el: &UIElement, path: &str, prefix: &str) -> UiNode {
    let role = el
        .get_control_type()
        .map(control_role)
        .unwrap_or_else(|_| "generic".to_string());
    let name = el.get_name().unwrap_or_default();
    let node_id = if path.is_empty() {
        "root".to_string()
    } else {
        path.to_string()
    };
    let mut node =
        UiNode::new(node_id, format!("{prefix}{path}"), role.clone()).with_source(SurfaceKind::App);
    if !name.is_empty() {
        node.name = Some(name);
    }
    if let Ok(b) = el.get_bounding_rectangle() {
        let (l, t, r, bo) = (b.get_left(), b.get_top(), b.get_right(), b.get_bottom());
        if r > l && bo > t {
            node.geometry = Some(Rect::new(
                l as f64,
                t as f64,
                (r - l) as f64,
                (bo - t) as f64,
            ));
        }
    }
    if let Ok(enabled) = el.is_enabled() {
        node.states.disabled = Some(!enabled);
    }
    if let Ok(focused) = el.has_keyboard_focus() {
        node.states.focused = Some(focused);
    }
    if let Ok(required) = el.is_required_for_form() {
        node.states.required = Some(required);
    }
    if let Ok(v) = el.get_pattern::<UIValuePattern>() {
        if let Ok(val) = v.get_value() {
            if !val.is_empty() {
                node.value = Some(val);
            }
        }
    }
    node.actions = actions_for_role(&role);
    node
}

#[async_trait]
impl crate::engine::SurfaceProvider for WindowsSurface {
    fn name(&self) -> &'static str {
        "windows"
    }

    fn kind(&self) -> SurfaceKind {
        SurfaceKind::App
    }

    fn capabilities(&self) -> SurfaceCapabilities {
        SurfaceCapabilities {
            native_ax: true,
            coordinate_input: false,
            screenshot: false,
            window_management: true,
        }
    }

    fn namespace(&self) -> &'static str {
        "desktop"
    }

    async fn list_surfaces(&self) -> Result<Vec<SurfaceInfo>> {
        Self::blocking(|| {
            let auto = automation()?;
            let walker = walker(&auto)?;
            let tops = top_levels(&auto, &walker)?;
            let mut out = Vec::new();
            for (i, el) in tops.iter().enumerate() {
                let title = el.get_name().unwrap_or_default();
                out.push(SurfaceInfo {
                    id: format!("desktop:{i}"),
                    kind: SurfaceKind::Window,
                    title: if title.is_empty() {
                        format!("window {i}")
                    } else {
                        title
                    },
                    app: el.get_classname().ok(),
                    pid: el.get_process_id().ok().map(|p| p as i32),
                    url: None,
                    bounds: None,
                    active: i == 0,
                });
            }
            Ok(out)
        })
        .await
    }

    async fn snapshot(&self, target: &str, opts: SnapshotOptions) -> Result<SurfaceSnapshot> {
        let top = Self::parse_top(target)
            .ok_or_else(|| EngineError::invalid(format!("bad desktop target '{target}'")))?;
        let prefix = format!("desktop:{top}:");
        let mut snap = Self::blocking(move || {
            let auto = automation()?;
            let walker = walker(&auto)?;
            let tops = top_levels(&auto, &walker)?;
            let root_el = tops
                .into_iter()
                .nth(top)
                .ok_or_else(|| EngineError::new(ErrorKind::InvalidArgument, "no such window"))?;

            let mut arena: Vec<UiNode> = Vec::new();
            let mut child_idx: Vec<Vec<usize>> = Vec::new();
            let mut stack: Vec<(UIElement, String, Option<usize>)> =
                vec![(root_el, String::new(), None)];
            while let Some((el, path, parent)) = stack.pop() {
                if arena.len() >= AX_NODE_CAP {
                    break;
                }
                let node = node_data(&el, &path, &prefix);
                let my = arena.len();
                arena.push(node);
                child_idx.push(Vec::new());
                if let Some(p) = parent {
                    child_idx[p].push(my);
                }
                if let Some(children) = walker.get_children(&el) {
                    for (i, c) in children.into_iter().enumerate().rev() {
                        let cpath = if path.is_empty() {
                            i.to_string()
                        } else {
                            format!("{path}.{i}")
                        };
                        stack.push((c, cpath, Some(my)));
                    }
                }
            }
            if arena.is_empty() {
                return Err(EngineError::new(ErrorKind::Snapshot, "empty UIA tree"));
            }
            let tree = assemble(&arena, &child_idx, 0);
            let (root, mut meta) = prune(tree, &opts);
            if arena.len() >= AX_NODE_CAP {
                meta.truncated = true;
                if meta.reason.is_none() {
                    meta.reason = Some("nodes".into());
                }
            }
            let title = arena[0].name.clone().unwrap_or_default();
            Ok(SurfaceSnapshot {
                surface: SurfaceInfo {
                    id: format!("desktop:{top}"),
                    kind: SurfaceKind::Window,
                    title: if title.is_empty() {
                        format!("window {top}")
                    } else {
                        title
                    },
                    app: None,
                    pid: None,
                    url: None,
                    bounds: None,
                    active: top == 0,
                },
                root,
                generation: 0,
                meta,
                timestamp_ms: 0,
            })
        })
        .await?;
        snap.generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        Ok(snap)
    }

    async fn act(&self, ref_: &str, action: SurfaceAction) -> Result<()> {
        let (top, path) = Self::parse_ref(ref_)?;
        let ref_owned = ref_.to_string();
        Self::blocking(move || {
            let auto = automation()?;
            let walker = walker(&auto)?;
            let mut current = top_levels(&auto, &walker)?
                .into_iter()
                .nth(top)
                .ok_or_else(|| EngineError::new(ErrorKind::InvalidArgument, "no such window"))?;
            for idx in &path {
                let children = walker.get_children(&current).unwrap_or_default();
                current = children.into_iter().nth(*idx).ok_or_else(|| {
                    EngineError::new(ErrorKind::Dom, format!("child {idx} not found"))
                })?;
            }
            match action {
                SurfaceAction::Click | SurfaceAction::Invoke => {
                    if let Ok(p) = current.get_pattern::<UIInvokePattern>() {
                        let _ = p.invoke();
                        return Ok(());
                    }
                    current.click().map_err(uia_err)
                }
                SurfaceAction::DoubleClick => current.double_click().map_err(uia_err),
                SurfaceAction::RightClick => current.right_click().map_err(uia_err),
                SurfaceAction::Focus => current.set_focus().map_err(uia_err),
                SurfaceAction::Check { .. } => match current.get_pattern::<UITogglePattern>() {
                    Ok(p) => p.toggle().map_err(uia_err),
                    Err(_) => current.click().map_err(uia_err),
                },
                SurfaceAction::SetValue { value } => {
                    let p = current.get_pattern::<UIValuePattern>().map_err(uia_err)?;
                    p.set_value(&value).map_err(uia_err)
                }
                other => Err(EngineError::unsupported(format!(
                    "action '{}' not supported by UIA surface (ref {ref_owned})",
                    other.name()
                ))),
            }
        })
        .await
    }

    async fn input(&self, _target: &str, _event: InputEvent) -> Result<()> {
        Err(EngineError::unsupported(
            "UIA surface does not support coordinate input",
        ))
    }
}

fn uia_err(e: uiautomation::Error) -> EngineError {
    EngineError::new(ErrorKind::Dom, format!("UIA: {e}"))
}

/// 由 arena + 子索引组装 owned 树。
fn assemble(arena: &[UiNode], child_idx: &[Vec<usize>], idx: usize) -> UiNode {
    let mut node = arena[idx].clone();
    for &c in &child_idx[idx] {
        node.children.push(assemble(arena, child_idx, c));
    }
    node
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_target_and_ref() {
        assert_eq!(WindowsSurface::parse_top(""), Some(0));
        assert_eq!(WindowsSurface::parse_top("desktop:3"), Some(3));
        assert_eq!(
            WindowsSurface::parse_ref("desktop:1:0.2.1").unwrap(),
            (1, vec![0, 2, 1])
        );
        assert!(WindowsSurface::parse_ref("web:1:a").is_err());
    }

    #[test]
    fn control_role_mapping() {
        assert_eq!(control_role(ControlType::Button), "button");
        assert_eq!(control_role(ControlType::Edit), "textbox");
        assert_eq!(control_role(ControlType::Hyperlink), "link");
        assert_eq!(control_role(ControlType::CheckBox), "checkbox");
        assert_eq!(control_role(ControlType::Window), "window");
    }

    #[test]
    fn assemble_builds_tree() {
        let arena = vec![
            UiNode::new("root", "desktop:0:root", "window"),
            UiNode::new("0", "desktop:0:0", "button"),
        ];
        let child_idx = vec![vec![1], vec![]];
        let tree = assemble(&arena, &child_idx, 0);
        assert_eq!(tree.node_count(), 2);
    }
}
