// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! 表面后端（feature `surface`）。
//!
//! - [`browser`]：把 `BrowserEngine` 适配为 `SurfaceProvider`（web 表面）。
//! - [`mock`]：脚本化表面，测试与无平台环境用。
//! - `macos`：macOS 原生无障碍后端（feature `surface-macos`）。

pub mod browser;
pub mod mock;

#[cfg(feature = "surface-macos")]
pub mod macos;

#[cfg(feature = "surface-linux")]
pub mod linux;

#[cfg(feature = "surface-windows")]
pub mod windows;

#[cfg(feature = "surface-android")]
pub mod android;

use std::sync::Arc;

use crate::config::Config;
use crate::engine::{BrowserEngine, ProviderRegistry, SurfaceProvider};

/// 按配置装配表面 provider 注册表。
///
/// - `auto`（默认）：浏览器（web 表面）+ 平台原生（desktop 表面，若已编译）。
/// - `browser`：仅浏览器。
/// - `macos`：仅 macOS 原生（需 feature `surface-macos`）。
/// - `android`：仅 Android 原生（需 feature `surface-android` + 宿主注册 `SurfaceOps`）。
/// - `mock`：仅脚本化 provider（测试/无平台）。
/// - `none`：不装配。
pub fn create_registry(engine: Arc<dyn BrowserEngine>, config: &Config) -> ProviderRegistry {
    let mut registry = ProviderRegistry::new();
    let provider = config.surface.provider.as_str();

    let register_browser = || {
        Arc::new(browser::BrowserSurface::with_options(
            engine.clone(),
            config.surface.prefer_ax_tree,
        )) as Arc<dyn SurfaceProvider>
    };

    match provider {
        "none" => {}
        "mock" => {
            registry.register(Arc::new(mock::MockSurface::new()) as Arc<dyn SurfaceProvider>);
        }
        "browser" => {
            registry.register(register_browser());
        }
        "macos" => {
            #[cfg(feature = "surface-macos")]
            registry.register(Arc::new(macos::MacSurface::new()) as Arc<dyn SurfaceProvider>);
        }
        "linux" => {
            #[cfg(all(feature = "surface-linux", target_os = "linux"))]
            registry.register(Arc::new(linux::LinuxSurface::new()) as Arc<dyn SurfaceProvider>);
        }
        "windows" => {
            #[cfg(all(feature = "surface-windows", target_os = "windows"))]
            registry.register(Arc::new(windows::WindowsSurface::new()) as Arc<dyn SurfaceProvider>);
        }
        "android" => {
            #[cfg(feature = "surface-android")]
            registry.register(Arc::new(android::AndroidSurface::new()) as Arc<dyn SurfaceProvider>);
        }
        // auto（或未知值）→ 浏览器 + 平台原生
        _ => {
            registry.register(register_browser());
            #[cfg(feature = "surface-macos")]
            registry.register(Arc::new(macos::MacSurface::new()) as Arc<dyn SurfaceProvider>);
            #[cfg(all(feature = "surface-linux", target_os = "linux"))]
            registry.register(Arc::new(linux::LinuxSurface::new()) as Arc<dyn SurfaceProvider>);
            #[cfg(all(feature = "surface-windows", target_os = "windows"))]
            registry.register(Arc::new(windows::WindowsSurface::new()) as Arc<dyn SurfaceProvider>);
            #[cfg(feature = "surface-android")]
            registry.register(Arc::new(android::AndroidSurface::new()) as Arc<dyn SurfaceProvider>);
        }
    }
    registry
}
