// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! 脚本化表面 provider：测试与「无平台后端」环境用。
//!
//! 命名空间 `desktop`，能力位接近原生（有 AX、可坐标输入），用于验证统一
//! `surface_*` 工具、注册表路由与同步外壳，无需真实操作系统权限。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use async_trait::async_trait;

use crate::engine::surface::{
    prune, SnapshotOptions, SurfaceAction, SurfaceCapabilities, SurfaceEvent, SurfaceEventKind,
    SurfaceInfo, SurfaceKind, SurfaceSnapshot, UiNode,
};
use crate::engine::{EngineError, ErrorKind, Image, InputEvent, Rect, Result};

/// 记录到的动作。
#[derive(Debug, Clone, PartialEq)]
pub struct RecordedAction {
    pub ref_: String,
    pub action: String,
}

/// 脚本化表面。
pub struct MockSurface {
    surfaces: Mutex<Vec<SurfaceInfo>>,
    tree: Mutex<UiNode>,
    acted: Mutex<Vec<RecordedAction>>,
    inputs: Mutex<usize>,
    events: Mutex<Vec<SurfaceEvent>>,
    generation: AtomicU64,
}

impl Default for MockSurface {
    fn default() -> Self {
        Self::new()
    }
}

impl MockSurface {
    pub fn new() -> Self {
        let surface = SurfaceInfo {
            id: "desktop:1".into(),
            kind: SurfaceKind::Window,
            title: "Mock Window".into(),
            app: Some("MockApp".into()),
            pid: Some(4242),
            url: None,
            bounds: Some(Rect::new(0.0, 0.0, 800.0, 600.0)),
            active: true,
        };
        let mut root = UiNode::new("0", "desktop:1:0", "window")
            .with_name("Mock Window")
            .with_source(SurfaceKind::Window)
            .with_geometry(Rect::new(0.0, 0.0, 800.0, 600.0));
        let mut ok = UiNode::new("0.0", "desktop:1:0.0", "button")
            .with_name("OK")
            .with_source(SurfaceKind::Window)
            .with_geometry(Rect::new(10.0, 10.0, 60.0, 30.0));
        ok.actions.push("click".into());
        let mut name = UiNode::new("0.1", "desktop:1:0.1", "textbox")
            .with_name("Name")
            .with_value("")
            .with_source(SurfaceKind::Window)
            .with_geometry(Rect::new(10.0, 50.0, 200.0, 24.0));
        name.actions.push("set_value".into());
        name.states.editable = Some(true);
        root.children.push(ok);
        root.children.push(name);

        MockSurface {
            surfaces: Mutex::new(vec![surface]),
            tree: Mutex::new(root),
            acted: Mutex::new(Vec::new()),
            inputs: Mutex::new(0),
            events: Mutex::new(Vec::new()),
            generation: AtomicU64::new(0),
        }
    }

    /// 用自定义表面与树构造（测试用）。
    pub fn with_content(surfaces: Vec<SurfaceInfo>, root: UiNode) -> Self {
        let s = MockSurface::new();
        *s.surfaces.lock().unwrap() = surfaces;
        *s.tree.lock().unwrap() = root;
        s
    }

    pub fn actions(&self) -> Vec<RecordedAction> {
        self.acted.lock().unwrap().clone()
    }

    pub fn input_count(&self) -> usize {
        *self.inputs.lock().unwrap()
    }

    /// 注入一条事件（测试用：模拟原生通知）。
    pub fn push_event(&self, kind: SurfaceEventKind) {
        self.events
            .lock()
            .unwrap()
            .push(SurfaceEvent::new(kind).with_surface("desktop:1"));
    }

    pub fn set_value_of(&self, node_ref: &str, value: &str) {
        let mut tree = self.tree.lock().unwrap();
        if let Some(n) = find_mut(&mut tree, node_ref) {
            n.value = Some(value.to_string());
        }
    }
}

fn find_mut<'a>(node: &'a mut UiNode, ref_: &str) -> Option<&'a mut UiNode> {
    if node.ref_ == ref_ {
        return Some(node);
    }
    for c in &mut node.children {
        if let Some(found) = find_mut(c, ref_) {
            return Some(found);
        }
    }
    None
}

#[async_trait]
impl crate::engine::SurfaceProvider for MockSurface {
    fn name(&self) -> &'static str {
        "mock"
    }

    fn kind(&self) -> SurfaceKind {
        SurfaceKind::Window
    }

    fn capabilities(&self) -> SurfaceCapabilities {
        SurfaceCapabilities {
            native_ax: true,
            coordinate_input: true,
            screenshot: false,
            window_management: true,
        }
    }

    fn namespace(&self) -> &'static str {
        "desktop"
    }

    async fn list_surfaces(&self) -> Result<Vec<SurfaceInfo>> {
        Ok(self.surfaces.lock().unwrap().clone())
    }

    async fn snapshot(&self, target: &str, opts: SnapshotOptions) -> Result<SurfaceSnapshot> {
        let surface = self
            .surfaces
            .lock()
            .unwrap()
            .iter()
            .find(|s| s.id == target)
            .cloned()
            .or_else(|| self.surfaces.lock().unwrap().first().cloned())
            .ok_or_else(|| EngineError::new(ErrorKind::InvalidArgument, "no mock surface"))?;
        let tree = self.tree.lock().unwrap().clone();
        let (root, mut meta) = prune(tree, &opts);
        meta.total_nodes = self.tree.lock().unwrap().node_count();
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
        if !ref_.starts_with("desktop:") {
            return Err(EngineError::invalid(format!("bad mock ref '{ref_}'")));
        }
        // set_value / type 反映到内存树，便于断言。
        let mut tree = self.tree.lock().unwrap();
        match &action {
            SurfaceAction::SetValue { value } => {
                if let Some(n) = find_mut(&mut tree, ref_) {
                    n.value = Some(value.clone());
                } else {
                    return Err(EngineError::new(ErrorKind::Dom, "node not found"));
                }
            }
            SurfaceAction::TypeText { text, clear } => {
                if let Some(n) = find_mut(&mut tree, ref_) {
                    let existing = if *clear {
                        String::new()
                    } else {
                        n.value.clone().unwrap_or_default()
                    };
                    n.value = Some(format!("{existing}{text}"));
                } else {
                    return Err(EngineError::new(ErrorKind::Dom, "node not found"));
                }
            }
            SurfaceAction::Check { checked } => {
                if let Some(n) = find_mut(&mut tree, ref_) {
                    n.states.checked = Some(*checked);
                }
            }
            _ => {}
        }
        drop(tree);
        self.acted.lock().unwrap().push(RecordedAction {
            ref_: ref_.to_string(),
            action: action.name().to_string(),
        });
        Ok(())
    }

    async fn input(&self, _target: &str, _event: InputEvent) -> Result<()> {
        *self.inputs.lock().unwrap() += 1;
        Ok(())
    }

    async fn screenshot(&self, _target: &str) -> Result<Image> {
        // 1x1 透明像素，便于测试截图路径。
        Ok(Image::new(1, 1, vec![0, 0, 0, 0]))
    }

    async fn poll_events(&self, _target: &str) -> Result<Vec<SurfaceEvent>> {
        let mut q = self.events.lock().unwrap();
        Ok(std::mem::take(&mut *q))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::SurfaceProvider;

    #[tokio::test]
    async fn snapshot_and_act() {
        let s = MockSurface::new();
        let snap = s
            .snapshot("desktop:1", SnapshotOptions::default())
            .await
            .unwrap();
        assert_eq!(snap.surface.id, "desktop:1");
        assert!(snap.node_by_ref("desktop:1:0.0").is_some());
        s.act("desktop:1:0.0", SurfaceAction::Click).await.unwrap();
        s.act(
            "desktop:1:0.1",
            SurfaceAction::SetValue {
                value: "Alice".into(),
            },
        )
        .await
        .unwrap();
        let acts = s.actions();
        assert_eq!(acts.len(), 2);
        assert_eq!(acts[0].action, "click");
        // 值写入内存树
        let snap2 = s
            .snapshot("desktop:1", SnapshotOptions::default())
            .await
            .unwrap();
        assert_eq!(
            snap2.node_by_ref("desktop:1:0.1").unwrap().value.as_deref(),
            Some("Alice")
        );
    }

    #[tokio::test]
    async fn input_counted_and_screenshot() {
        let s = MockSurface::new();
        s.input(
            "desktop:1",
            InputEvent::Key(crate::engine::KeyEvent::press("Enter")),
        )
        .await
        .unwrap();
        assert_eq!(s.input_count(), 1);
        let img = s.screenshot("desktop:1").await.unwrap();
        assert!(img.is_valid());
    }

    #[tokio::test]
    async fn bad_ref_rejected() {
        let s = MockSurface::new();
        assert!(s.act("web:1:a", SurfaceAction::Click).await.is_err());
    }

    #[tokio::test]
    async fn events_push_and_drain() {
        let s = MockSurface::new();
        s.push_event(SurfaceEventKind::FocusChanged);
        s.push_event(SurfaceEventKind::ValueChanged);
        let ev = s.poll_events("").await.unwrap();
        assert_eq!(ev.len(), 2);
        assert_eq!(ev[0].kind, SurfaceEventKind::FocusChanged);
        assert_eq!(ev[0].surface.as_deref(), Some("desktop:1"));
        // drain 后为空
        assert!(s.poll_events("").await.unwrap().is_empty());
    }
}
