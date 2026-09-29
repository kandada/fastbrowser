// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Linux 原生无障碍后端（feature `surface-linux`，AT-SPI via zbus）。
//!
//! 通过 AT-SPI2（`org.a11y.atspi.*` D-Bus 接口）感知与操作原生应用：
//! `Accessible`（角色/名称/子节点/状态）、`Component`（几何）、`Action`（点击）、
//! `Value` / `EditableText`（设置值/文本）。
//!
//! 命名空间 `desktop`，ref 形如 `desktop:<appIndex>:<child.path>`（`0.2.1`）。
//! 仅在 Linux 运行；其他平台可编译但不注册。AT-SPI 是系统 D-Bus 服务，需桌面
//! 会话启用无障碍（`gsettings set org.gnome.desktop.interface toolkit-accessibility true`）。

use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;

use atspi::connection::AccessibilityConnection;
use atspi::proxy::accessible::{AccessibleProxy, ObjectRefExt};
use atspi::proxy::proxy_ext::ProxyExt;
use atspi::{CoordType, ObjectRefOwned, State};

use crate::engine::surface::{
    prune, SnapshotOptions, SurfaceAction, SurfaceCapabilities, SurfaceInfo, SurfaceKind,
    SurfaceSnapshot, UiNode,
};
use crate::engine::{EngineError, ErrorKind, InputEvent, Rect, Result};

/// 单次 AT-SPI 遍历的硬上限。
const AX_NODE_CAP: usize = 2000;

/// Linux 原生表面 provider。
pub struct LinuxSurface {
    generation: AtomicU64,
}

impl Default for LinuxSurface {
    fn default() -> Self {
        Self::new()
    }
}

impl LinuxSurface {
    pub fn new() -> Self {
        LinuxSurface {
            generation: AtomicU64::new(0),
        }
    }

    async fn connect() -> Result<AccessibilityConnection> {
        AccessibilityConnection::new().await.map_err(|e| {
            EngineError::new(
                ErrorKind::Plugin,
                format!("AT-SPI connect failed (is the accessibility bus running?): {e}"),
            )
        })
    }

    /// 解析目标表面为应用下标：`desktop:2` / `2` / 空（首个应用）。
    fn parse_app(target: &str) -> Option<usize> {
        let t = target.trim();
        if t.is_empty() || t == "desktop" || t == "focused" {
            return Some(0);
        }
        let rest = t.strip_prefix("desktop:").unwrap_or(t);
        rest.split(':').next()?.parse().ok()
    }

    /// 解析 ref 为（应用下标, 子节点路径）。
    fn parse_ref(ref_: &str) -> Result<(usize, Vec<usize>)> {
        let parts: Vec<&str> = ref_.splitn(3, ':').collect();
        if parts.len() < 2 || parts[0] != "desktop" {
            return Err(EngineError::invalid(format!(
                "bad desktop ref '{ref_}' (expected desktop:<app>:<path>)"
            )));
        }
        let app: usize = parts[1]
            .parse()
            .map_err(|_| EngineError::invalid(format!("bad app index in ref '{ref_}'")))?;
        let path = parts
            .get(2)
            .map(|s| {
                s.split('.')
                    .filter_map(|x| x.parse::<usize>().ok())
                    .collect()
            })
            .unwrap_or_default();
        Ok((app, path))
    }

    /// 应用根 ObjectRef（AT-SPI registry 的子节点即应用）。
    async fn app_ref(conn: &AccessibilityConnection, index: usize) -> Result<ObjectRefOwned> {
        let registry = conn
            .root_accessible_on_registry()
            .await
            .map_err(|e| EngineError::new(ErrorKind::Plugin, format!("AT-SPI registry: {e}")))?;
        let children = registry
            .get_children()
            .await
            .map_err(|e| EngineError::new(ErrorKind::Plugin, format!("AT-SPI apps: {e}")))?;
        children
            .into_iter()
            .nth(index)
            .ok_or_else(|| EngineError::new(ErrorKind::InvalidArgument, "no such application"))
    }
}

#[async_trait]
impl crate::engine::SurfaceProvider for LinuxSurface {
    fn name(&self) -> &'static str {
        "linux"
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
        let conn = Self::connect().await?;
        let registry = conn
            .root_accessible_on_registry()
            .await
            .map_err(|e| EngineError::new(ErrorKind::Plugin, format!("AT-SPI registry: {e}")))?;
        let children = registry
            .get_children()
            .await
            .map_err(|e| EngineError::new(ErrorKind::Plugin, format!("AT-SPI apps: {e}")))?;
        let mut out = Vec::new();
        for (i, child) in children.iter().enumerate() {
            if child.is_null() {
                continue;
            }
            let Ok(proxy) = child.as_accessible_proxy(conn.connection()).await else {
                continue;
            };
            let title = proxy.name().await.unwrap_or_default();
            out.push(SurfaceInfo {
                id: format!("desktop:{i}"),
                kind: SurfaceKind::App,
                title: if title.is_empty() {
                    format!("app {i}")
                } else {
                    title
                },
                app: None,
                pid: None,
                url: None,
                bounds: None,
                active: i == 0,
            });
        }
        Ok(out)
    }

    async fn snapshot(&self, target: &str, opts: SnapshotOptions) -> Result<SurfaceSnapshot> {
        let app_index = Self::parse_app(target)
            .ok_or_else(|| EngineError::invalid(format!("bad desktop target '{target}'")))?;
        let conn = Self::connect().await?;
        let app = Self::app_ref(&conn, app_index).await?;
        let prefix = format!("desktop:{app_index}:");

        // 迭代式 DFS：避免递归 async 的生命周期问题；先建 arena，再组装树。
        let mut arena: Vec<UiNode> = Vec::new();
        let mut child_idx: Vec<Vec<usize>> = Vec::new();
        let mut stack: Vec<(ObjectRefOwned, String, Option<usize>)> =
            vec![(app, String::new(), None)];

        while let Some((oref, path, parent)) = stack.pop() {
            if arena.len() >= AX_NODE_CAP {
                break;
            }
            let Ok(proxy) = oref.as_accessible_proxy(conn.connection()).await else {
                continue;
            };
            let node = node_data(&proxy, &path, &prefix).await;
            let my_idx = arena.len();
            arena.push(node);
            child_idx.push(Vec::new());
            if let Some(p) = parent {
                child_idx[p].push(my_idx);
            }
            if let Ok(children) = proxy.get_children().await {
                for (i, child) in children.into_iter().enumerate().rev() {
                    if child.is_null() {
                        continue;
                    }
                    let cpath = if path.is_empty() {
                        i.to_string()
                    } else {
                        format!("{path}.{i}")
                    };
                    stack.push((child, cpath, Some(my_idx)));
                }
            }
            drop(proxy);
        }

        if arena.is_empty() {
            return Err(EngineError::new(
                ErrorKind::Snapshot,
                "failed to build AT-SPI tree",
            ));
        }
        let tree = assemble(&arena, &child_idx, 0);
        let (root, mut meta) = prune(tree, &opts);
        if arena.len() >= AX_NODE_CAP {
            meta.truncated = true;
            if meta.reason.is_none() {
                meta.reason = Some("nodes".into());
            }
        }

        // 应用标题
        let app2 = Self::app_ref(&conn, app_index).await?;
        let title = match app2.as_accessible_proxy(conn.connection()).await {
            Ok(p) => p.name().await.unwrap_or_default(),
            Err(_) => String::new(),
        };
        let surface = SurfaceInfo {
            id: format!("desktop:{app_index}"),
            kind: SurfaceKind::App,
            title: if title.is_empty() {
                format!("app {app_index}")
            } else {
                title
            },
            app: None,
            pid: None,
            url: None,
            bounds: None,
            active: app_index == 0,
        };
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        Ok(SurfaceSnapshot {
            surface,
            root,
            generation,
            meta,
            timestamp_ms: 0,
        })
    }

    async fn act(&self, ref_: &str, action: SurfaceAction) -> Result<()> {
        let (app_index, path) = Self::parse_ref(ref_)?;
        let conn = Self::connect().await?;
        let mut current = Self::app_ref(&conn, app_index).await?;
        for idx in &path {
            let proxy = current
                .as_accessible_proxy(conn.connection())
                .await
                .map_err(|e| EngineError::new(ErrorKind::Dom, format!("AT-SPI proxy: {e}")))?;
            let child = proxy.get_child_at_index(*idx as i32).await.map_err(|e| {
                EngineError::new(ErrorKind::Dom, format!("AT-SPI child {idx}: {e}"))
            })?;
            drop(proxy);
            current = child;
        }
        let proxy = current
            .as_accessible_proxy(conn.connection())
            .await
            .map_err(|e| EngineError::new(ErrorKind::Dom, format!("AT-SPI proxy: {e}")))?;
        let proxies = proxy
            .proxies()
            .await
            .map_err(|e| EngineError::new(ErrorKind::Dom, format!("AT-SPI proxies: {e}")))?;

        match action {
            SurfaceAction::Click
            | SurfaceAction::Invoke
            | SurfaceAction::Focus
            | SurfaceAction::Check { .. } => {
                let act = proxies
                    .action()
                    .await
                    .map_err(|e| EngineError::new(ErrorKind::Dom, format!("AT-SPI action: {e}")))?;
                let actions = act.get_actions().await.unwrap_or_default();
                let index = actions
                    .iter()
                    .position(|a| {
                        let n = a.name.to_ascii_lowercase();
                        n.contains("click") || n.contains("press") || n.contains("activate")
                    })
                    .unwrap_or(0) as i32;
                let ok = act.do_action(index).await.map_err(|e| {
                    EngineError::new(ErrorKind::Dom, format!("AT-SPI do_action: {e}"))
                })?;
                if ok {
                    Ok(())
                } else {
                    Err(EngineError::new(
                        ErrorKind::Dom,
                        "AT-SPI action returned false",
                    ))
                }
            }
            SurfaceAction::SetValue { value } => {
                if let Ok(v) = proxies.value().await {
                    if let Ok(n) = value.parse::<f64>() {
                        return v.set_current_value(n).await.map_err(|e| {
                            EngineError::new(ErrorKind::Dom, format!("AT-SPI set value: {e}"))
                        });
                    }
                }
                let et = proxies.editable_text().await.map_err(|e| {
                    EngineError::new(ErrorKind::Dom, format!("AT-SPI editable text: {e}"))
                })?;
                et.set_text_contents(&value).await.map_err(|e| {
                    EngineError::new(ErrorKind::Dom, format!("AT-SPI set text: {e}"))
                })?;
                Ok(())
            }
            other => Err(EngineError::unsupported(format!(
                "action '{}' not supported by AT-SPI surface",
                other.name()
            ))),
        }
    }

    async fn input(&self, _target: &str, _event: InputEvent) -> Result<()> {
        Err(EngineError::unsupported(
            "AT-SPI surface does not support coordinate input",
        ))
    }
}

/// 读取单个 AT-SPI 节点的属性（不含子节点）。
async fn node_data(proxy: &AccessibleProxy<'_>, path: &str, prefix: &str) -> UiNode {
    let role = proxy
        .get_role()
        .await
        .map(|r| r.to_string().to_ascii_lowercase())
        .unwrap_or_default();
    let name = proxy.name().await.unwrap_or_default();
    let node_id = if path.is_empty() {
        "root".to_string()
    } else {
        path.to_string()
    };
    let mut node =
        UiNode::new(node_id, format!("{prefix}{path}"), role).with_source(SurfaceKind::App);
    if !name.is_empty() {
        node.name = Some(name);
    }
    if let Ok(states) = proxy.get_state().await {
        node.states.disabled = Some(!states.contains(State::Enabled));
        node.states.focused = Some(states.contains(State::Focused));
        node.states.checked = Some(states.contains(State::Checked));
        node.states.selected = Some(states.contains(State::Selected));
        node.states.expanded = Some(states.contains(State::Expanded));
        node.states.editable = Some(states.contains(State::Editable));
        node.states.required = Some(states.contains(State::Required));
        node.states.busy = Some(states.contains(State::Busy));
    }
    if let Ok(proxies) = proxy.proxies().await {
        if let Ok(component) = proxies.component().await {
            if let Ok((x, y, w, h)) = component.get_extents(CoordType::Screen).await {
                if w > 0 && h > 0 {
                    node.geometry = Some(Rect::new(x as f64, y as f64, w as f64, h as f64));
                }
            }
        }
        if let Ok(act) = proxies.action().await {
            if let Ok(actions) = act.get_actions().await {
                node.actions = actions
                    .iter()
                    .map(|a| a.name.to_ascii_lowercase())
                    .collect();
            }
        }
    }
    node
}

/// 由 arena + 子索引组装 owned 树（快照有上限，克隆可接受）。
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
        assert_eq!(LinuxSurface::parse_app(""), Some(0));
        assert_eq!(LinuxSurface::parse_app("desktop:3"), Some(3));
        assert_eq!(LinuxSurface::parse_app("3"), Some(3));
        assert_eq!(
            LinuxSurface::parse_ref("desktop:1:0.2.1").unwrap(),
            (1, vec![0, 2, 1])
        );
        assert_eq!(LinuxSurface::parse_ref("desktop:0").unwrap(), (0, vec![]));
        assert!(LinuxSurface::parse_ref("web:1:a").is_err());
    }

    #[test]
    fn assemble_builds_tree() {
        let arena = vec![
            UiNode::new("root", "desktop:0:root", "application"),
            UiNode::new("0", "desktop:0:0", "button"),
            UiNode::new("0.0", "desktop:0:0.0", "text"),
        ];
        let child_idx = vec![vec![1], vec![2], vec![]];
        let tree = assemble(&arena, &child_idx, 0);
        assert_eq!(tree.node_count(), 3);
        assert_eq!(tree.children[0].children[0].role, "text");
    }
}
