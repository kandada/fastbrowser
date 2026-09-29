// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! 移动端原生无障碍后端（feature `surface-android`）。
//!
//! Android 的 `AccessibilityService` 由 **App（Kotlin）实现**，经 JNI / C ABI 以
//! [`SurfaceOps`](crate::engine::host::SurfaceOps)（JSON 协议）注入；本 provider 只做
//! 协议适配与世代号管理，因此可跨平台编译（未注册宿主 ops 时调用会返回明确错误）。
//!
//! 命名空间 `desktop`，ref 形如 `desktop:<windowId>:<path>`，与 macOS/Linux/Windows
//! 后端一致，`ax_*` 工具无需改动即可复用。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use async_trait::async_trait;

use crate::engine::host::SurfaceOps;
use crate::engine::surface::{
    prune, SnapshotOptions, SurfaceAction, SurfaceCapabilities, SurfaceInfo, SurfaceKind,
    SurfaceSnapshot,
};
use crate::engine::{EngineError, ErrorKind, Image, InputEvent, Result, SurfaceEvent};
use crate::sdk::plugin::take_surface_ops;

/// Android 原生表面 provider。
pub struct AndroidSurface {
    generation: AtomicU64,
}

impl Default for AndroidSurface {
    fn default() -> Self {
        Self::new()
    }
}

impl AndroidSurface {
    pub fn new() -> Self {
        AndroidSurface {
            generation: AtomicU64::new(0),
        }
    }

    fn ops() -> Result<Arc<dyn SurfaceOps>> {
        take_surface_ops().ok_or_else(|| {
            EngineError::new(
                ErrorKind::Plugin,
                "no SurfaceOps host registered; the Android host must call \
                 fastbrowser_register_surface_ops before init (and enable its \
                 AccessibilityService in system settings)",
            )
        })
    }

    async fn blocking<T, F>(f: F) -> Result<T>
    where
        F: FnOnce() -> Result<T> + Send + 'static,
        T: Send + 'static,
    {
        tokio::task::spawn_blocking(f).await.map_err(|e| {
            EngineError::new(ErrorKind::Internal, format!("android surface join: {e}"))
        })?
    }
}

#[async_trait]
impl crate::engine::SurfaceProvider for AndroidSurface {
    fn name(&self) -> &'static str {
        "android"
    }

    fn kind(&self) -> SurfaceKind {
        SurfaceKind::App
    }

    fn capabilities(&self) -> SurfaceCapabilities {
        Self::ops()
            .and_then(|o| o.capabilities())
            .ok()
            .and_then(|json| serde_json::from_str::<SurfaceCapabilities>(&json).ok())
            .unwrap_or_else(SurfaceCapabilities::none)
    }

    fn namespace(&self) -> &'static str {
        "desktop"
    }

    async fn list_surfaces(&self) -> Result<Vec<SurfaceInfo>> {
        let ops = Self::ops()?;
        Self::blocking(move || {
            let json = ops.list()?;
            serde_json::from_str::<Vec<SurfaceInfo>>(&json).map_err(|e| {
                EngineError::new(ErrorKind::Dom, format!("android list_surfaces: {e}"))
            })
        })
        .await
    }

    async fn snapshot(&self, target: &str, opts: SnapshotOptions) -> Result<SurfaceSnapshot> {
        let ops = Self::ops()?;
        let target = target.to_string();
        let opts_json = serde_json::to_string(&opts)
            .map_err(|e| EngineError::new(ErrorKind::Internal, format!("opts json: {e}")))?;
        let mut snapshot = Self::blocking(move || {
            let json = ops.snapshot(&target, &opts_json)?;
            serde_json::from_str::<SurfaceSnapshot>(&json).map_err(|e| {
                EngineError::new(ErrorKind::Snapshot, format!("android snapshot: {e}"))
            })
        })
        .await?;
        // 宿主返回受限原始树；裁剪与 token 预算在内核侧统一处理（与 macOS 一致）。
        if let Some(root) = snapshot.root.take() {
            let (root, meta) = prune(root, &opts);
            snapshot.root = root;
            snapshot.meta = meta;
        }
        snapshot.generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        Ok(snapshot)
    }

    async fn act(&self, ref_: &str, action: SurfaceAction) -> Result<()> {
        let ops = Self::ops()?;
        let ref_ = ref_.to_string();
        let action_json = serde_json::to_string(&action)
            .map_err(|e| EngineError::new(ErrorKind::Internal, format!("action json: {e}")))?;
        Self::blocking(move || {
            ops.act(&ref_, &action_json)?;
            Ok(())
        })
        .await
    }

    async fn input(&self, target: &str, event: InputEvent) -> Result<()> {
        let ops = Self::ops()?;
        let target = target.to_string();
        let event_json = serde_json::to_string(&event)
            .map_err(|e| EngineError::new(ErrorKind::Internal, format!("event json: {e}")))?;
        Self::blocking(move || ops.input(&target, &event_json)).await
    }

    async fn screenshot(&self, target: &str) -> Result<Image> {
        let ops = Self::ops()?;
        let target = target.to_string();
        Self::blocking(move || {
            let json = ops.screenshot(&target)?;
            let v: serde_json::Value = serde_json::from_str(&json).map_err(|e| {
                EngineError::new(ErrorKind::Snapshot, format!("android screenshot: {e}"))
            })?;
            let width = v.get("width").and_then(|x| x.as_u64()).unwrap_or(0) as u32;
            let height = v.get("height").and_then(|x| x.as_u64()).unwrap_or(0) as u32;
            let b64 = v.get("base64").and_then(|x| x.as_str()).unwrap_or("");
            use base64::Engine as _;
            let rgba = base64::engine::general_purpose::STANDARD
                .decode(b64)
                .map_err(|e| EngineError::new(ErrorKind::Snapshot, format!("base64: {e}")))?;
            Ok(Image {
                width,
                height,
                rgba,
            })
        })
        .await
    }

    async fn poll_events(&self, target: &str) -> Result<Vec<SurfaceEvent>> {
        let ops = match take_surface_ops() {
            Some(o) => o,
            None => return Ok(Vec::new()),
        };
        let target = target.to_string();
        Self::blocking(move || {
            let json = ops.poll_events(&target)?;
            serde_json::from_str::<Vec<SurfaceEvent>>(&json).map_err(|e| {
                EngineError::new(ErrorKind::Internal, format!("android poll_events: {e}"))
            })
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::SurfaceProvider;

    #[test]
    fn identity_and_no_op_capabilities() {
        let s = AndroidSurface::new();
        assert_eq!(s.name(), "android");
        assert_eq!(s.namespace(), "desktop");
        assert_eq!(s.kind(), SurfaceKind::App);
        // No host SurfaceOps registered in unit tests → capabilities fall back.
        assert_eq!(s.capabilities(), SurfaceCapabilities::none());
    }

    #[tokio::test]
    async fn calls_error_clearly_without_host_ops() {
        let s = AndroidSurface::new();
        let e = s.list_surfaces().await.unwrap_err();
        assert!(e.to_string().contains("SurfaceOps"), "{e}");
        let e = s
            .snapshot("desktop:1", SnapshotOptions::default())
            .await
            .unwrap_err();
        assert!(e.to_string().contains("SurfaceOps"), "{e}");
        // poll_events degrades to empty rather than erroring (host optional).
        assert!(s.poll_events("desktop:1").await.unwrap().is_empty());
    }
}
