// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! 表面 provider 抽象 + 注册表 + 同步外壳（feature `surface`）。
//!
//! 架构（见 `FASTBROWSER_ACCESSIBILITY.md` §5）：
//! - **异步核心**：`SurfaceProvider` 全异步；注册表编排亦异步。
//! - **同步外壳**：`SurfaceRuntime` 持有一个 tokio 运行时，向 C ABI / CLI / 同步
//!   SDK 暴露阻塞式 API；FFI 不能异步，因此同步只发生在最外层。
//! - **能力门控**：注册表汇总各 provider 的能力位，工具层据此收敛。
//! - **超时/取消**：每次阻塞调用都套 `tokio::time::timeout`，目标应用无响应
//!   时不会永久挂起。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use crate::engine::surface::{
    ref_namespace, SnapshotOptions, SurfaceAction, SurfaceCapabilities, SurfaceEvent, SurfaceInfo,
    SurfaceKind, SurfaceSnapshot,
};
use crate::engine::{EngineError, ErrorKind, Image, InputEvent, Result};

/// 表面输入事件（复用浏览器输入模型，原生后端按需解释）。
pub type SurfaceInput = InputEvent;

/// 一个可感知/可操作的「表面」提供者。
///
/// 浏览器是第一个实现（web 表面），macOS 原生无障碍是另一个（desktop 表面），
/// 测试用脚本化 mock 是第三个。所有实现都产出统一的 [`SurfaceSnapshot`]。
#[async_trait]
pub trait SurfaceProvider: Send + Sync {
    /// provider 名（"browser" / "macos" / "mock"）。
    fn name(&self) -> &'static str;

    /// 默认表面类型（用于能力/展示）。
    fn kind(&self) -> SurfaceKind;

    /// 能力位。
    fn capabilities(&self) -> SurfaceCapabilities;

    /// 本 provider 拥有的 ref 命名空间（`web` / `desktop` / …）。
    fn namespace(&self) -> &'static str;

    /// 枚举当前可用的表面。
    async fn list_surfaces(&self) -> Result<Vec<SurfaceInfo>>;

    /// 对目标表面生成快照。`target` 为表面 id（如 `web:3`）或 ref 前缀。
    async fn snapshot(&self, target: &str, opts: SnapshotOptions) -> Result<SurfaceSnapshot>;

    /// 对元素 ref 施加动作。
    async fn act(&self, ref_: &str, action: SurfaceAction) -> Result<()>;

    /// 坐标级输入（点击/移动/滚轮/按键）。默认不支持。
    async fn input(&self, target: &str, event: SurfaceInput) -> Result<()> {
        let _ = (target, event);
        Err(EngineError::unsupported(format!(
            "surface provider '{}' does not support coordinate input",
            self.name()
        )))
    }

    /// 截图。默认不支持。
    async fn screenshot(&self, target: &str) -> Result<Image> {
        let _ = target;
        Err(EngineError::unsupported(format!(
            "surface provider '{}' does not support screenshot",
            self.name()
        )))
    }

    /// 拉取自上次调用以来的原生事件（焦点/窗口/值/标题/选择/结构变化）。
    ///
    /// 事件是「推」语义：provider 在后台收集，调用方按需 drain。默认无事件源，
    /// 返回空列表。`target` 为表面 id（空 = 默认/活动表面）。
    async fn poll_events(&self, target: &str) -> Result<Vec<SurfaceEvent>> {
        let _ = target;
        Ok(Vec::new())
    }
}

/// provider 注册表：按 ref 命名空间路由，并汇总能力位。
#[derive(Default)]
pub struct ProviderRegistry {
    providers: Vec<Arc<dyn SurfaceProvider>>,
    generation: AtomicU64,
}

impl ProviderRegistry {
    pub fn new() -> Self {
        ProviderRegistry::default()
    }

    pub fn with_providers(providers: Vec<Arc<dyn SurfaceProvider>>) -> Self {
        ProviderRegistry {
            providers,
            generation: AtomicU64::new(0),
        }
    }

    pub fn register(&mut self, provider: Arc<dyn SurfaceProvider>) {
        self.providers.push(provider);
    }

    pub fn providers(&self) -> &[Arc<dyn SurfaceProvider>] {
        &self.providers
    }

    pub fn is_empty(&self) -> bool {
        self.providers.is_empty()
    }

    pub fn len(&self) -> usize {
        self.providers.len()
    }

    /// 汇总所有 provider 的能力位。
    pub fn capabilities(&self) -> SurfaceCapabilities {
        self.providers
            .iter()
            .fold(SurfaceCapabilities::none(), |acc, p| {
                acc.merge(p.capabilities())
            })
    }

    /// 自增并返回快照世代号。
    pub fn next_generation(&self) -> u64 {
        self.generation.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// 按 ref 命名空间找 provider。
    pub fn provider_for_ref(&self, ref_: &str) -> Option<&Arc<dyn SurfaceProvider>> {
        let ns = ref_namespace(ref_);
        if ns.is_empty() {
            return None;
        }
        self.provider_by_namespace(ns)
    }

    /// 按命名空间找 provider。
    pub fn provider_by_namespace(&self, ns: &str) -> Option<&Arc<dyn SurfaceProvider>> {
        self.providers.iter().find(|p| p.namespace() == ns)
    }

    /// 首个具备坐标输入能力的 provider（`input` 未指定目标时用）。
    pub fn input_provider(&self) -> Option<&Arc<dyn SurfaceProvider>> {
        self.providers
            .iter()
            .find(|p| p.capabilities().coordinate_input)
            .or_else(|| self.providers.first())
    }

    /// 聚合所有 provider 的表面列表（单个 provider 失败不阻断其余）。
    pub async fn list_surfaces(&self) -> Result<Vec<SurfaceInfo>> {
        let report = self.list_surfaces_report().await;
        if report.surfaces.is_empty() {
            if let Some((_, msg)) = report.skipped.first() {
                return Err(EngineError::new(ErrorKind::Plugin, msg.clone()));
            }
        }
        Ok(report.surfaces)
    }

    /// 同 [`list_surfaces`](Self::list_surfaces)，但额外返回**被跳过的 provider**
    /// （名称 + 错误）。让 `ax_list` 能告诉 agent「原生 provider 因未授权/服务未启用
    /// 而不可用」，而不是静默省略（否则 agent 会误以为只有网页 surface）。
    pub async fn list_surfaces_report(&self) -> SurfaceListReport {
        let mut report = SurfaceListReport::default();
        for p in &self.providers {
            match p.list_surfaces().await {
                Ok(mut v) => report.surfaces.append(&mut v),
                Err(e) => report.skipped.push((p.name().to_string(), e.to_string())),
            }
        }
        report
    }

    /// 对目标表面生成快照。
    pub async fn snapshot(&self, target: &str, opts: SnapshotOptions) -> Result<SurfaceSnapshot> {
        let provider = self
            .resolve_target_provider(target)
            .ok_or_else(|| unknown_target(target))?;
        provider.snapshot(target, opts).await
    }

    /// 对 ref 施加动作。
    pub async fn act(&self, ref_: &str, action: SurfaceAction) -> Result<()> {
        let provider = self
            .provider_for_ref(ref_)
            .ok_or_else(|| unknown_ref(ref_))?;
        provider.act(ref_, action).await
    }

    /// 坐标级输入。
    pub async fn input(&self, target: &str, event: SurfaceInput) -> Result<()> {
        let provider = if target.trim().is_empty() {
            self.input_provider()
        } else {
            self.resolve_target_provider(target)
        }
        .ok_or_else(|| unknown_target(target))?;
        provider.input(target, event).await
    }

    /// 聚合所有 provider 的原生事件（单个 provider 失败不阻断其余）。
    pub async fn poll_events(&self, target: &str) -> Result<Vec<SurfaceEvent>> {
        let mut out = Vec::new();
        for p in &self.providers {
            if let Ok(mut ev) = p.poll_events(target).await {
                out.append(&mut ev);
            }
        }
        Ok(out)
    }

    fn resolve_target_provider(&self, target: &str) -> Option<&Arc<dyn SurfaceProvider>> {
        if target.trim().is_empty() {
            return self.providers.first();
        }
        let ns = ref_namespace(target);
        if !ns.is_empty() {
            if let Some(p) = self.provider_by_namespace(ns) {
                return Some(p);
            }
        }
        // target 可能是裸 id（无命名空间）：唯一 provider 时直接用它。
        if self.providers.len() == 1 {
            return self.providers.first();
        }
        None
    }
}

fn unknown_ref(ref_: &str) -> EngineError {
    EngineError::new(
        ErrorKind::InvalidArgument,
        format!("no surface provider owns ref '{ref_}' (expected '<namespace>:<surface>:<id>')"),
    )
}

fn unknown_target(target: &str) -> EngineError {
    EngineError::new(
        ErrorKind::InvalidArgument,
        format!("no surface provider for target '{target}' (expected '<namespace>:<surface>')"),
    )
}

/// 表面列表聚合结果：成功列出的表面 + 被跳过（失败）的 provider 及原因。
#[derive(Debug, Clone, Default)]
pub struct SurfaceListReport {
    /// 所有成功列出的表面。
    pub surfaces: Vec<SurfaceInfo>,
    /// `(provider 名, 错误信息)`，表示该 provider 本次未能提供表面
    /// （如权限未授予、无障碍服务未启用）。
    pub skipped: Vec<(String, String)>,
}

/// 同步外壳：持有唯一 tokio 运行时，向同步调用方暴露阻塞 API。
///
/// 同步只在最外层发生：C ABI / CLI / 同步工具调用 `*_blocking`，内部 `block_on`
/// 异步 provider。**不要**在异步上下文中调用这些方法（会 panic）；异步调用方
/// 应直接用 [`ProviderRegistry`] 的异步方法。
pub struct SurfaceRuntime {
    rt: tokio::runtime::Runtime,
    registry: Arc<ProviderRegistry>,
    timeout: Duration,
}

impl SurfaceRuntime {
    /// 默认单次表面操作超时（秒）。目标应用无响应时不永久挂起。
    pub const DEFAULT_TIMEOUT_SECS: u64 = 20;

    pub fn new(registry: ProviderRegistry) -> Result<Self> {
        Self::with_timeout(registry, Duration::from_secs(Self::DEFAULT_TIMEOUT_SECS))
    }

    pub fn with_timeout(registry: ProviderRegistry, timeout: Duration) -> Result<Self> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("fastbrowser-surface")
            // Enable the I/O driver too: the Linux AT-SPI backend (zbus) opens a
            // D-Bus socket, so a time-only runtime panics ("IO driver disabled")
            // when it is used — which killed the daemon on Linux CI. `enable_all`
            // turns on I/O + time (both features are compiled in).
            .enable_all()
            .build()
            .map_err(|e| EngineError::new(ErrorKind::Internal, format!("surface runtime: {e}")))?;
        Ok(SurfaceRuntime {
            rt,
            registry: Arc::new(registry),
            timeout,
        })
    }

    pub fn registry(&self) -> &Arc<ProviderRegistry> {
        &self.registry
    }

    pub fn is_available(&self) -> bool {
        !self.registry.is_empty()
    }

    pub fn capabilities(&self) -> SurfaceCapabilities {
        self.registry.capabilities()
    }

    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    fn block_on<T, F>(&self, fut: F) -> Result<T>
    where
        F: std::future::Future<Output = Result<T>>,
    {
        let timeout = self.timeout;
        self.rt.block_on(async move {
            match tokio::time::timeout(timeout, fut).await {
                Ok(r) => r,
                Err(_) => Err(EngineError::new(
                    ErrorKind::Timeout,
                    format!("surface operation timed out ({}s)", timeout.as_secs()),
                )),
            }
        })
    }

    pub fn list_surfaces_blocking(&self) -> Result<Vec<SurfaceInfo>> {
        let registry = self.registry.clone();
        self.block_on(async move { registry.list_surfaces().await })
    }

    /// 同 [`list_surfaces_blocking`](Self::list_surfaces_blocking)，但返回包含
    /// “被跳过 provider”的诊断信息。超时/整体失败时也以 `skipped` 表达。
    pub fn list_surfaces_report_blocking(&self) -> SurfaceListReport {
        let registry = self.registry.clone();
        let timeout = self.timeout;
        self.rt.block_on(async move {
            match tokio::time::timeout(timeout, registry.list_surfaces_report()).await {
                Ok(r) => r,
                Err(_) => SurfaceListReport {
                    surfaces: Vec::new(),
                    skipped: vec![(
                        "(all)".to_string(),
                        format!("surface list timed out ({}s)", timeout.as_secs()),
                    )],
                },
            }
        })
    }

    pub fn snapshot_blocking(
        &self,
        target: &str,
        opts: SnapshotOptions,
    ) -> Result<SurfaceSnapshot> {
        let registry = self.registry.clone();
        let target = target.to_string();
        self.block_on(async move { registry.snapshot(&target, opts).await })
    }

    pub fn act_blocking(&self, ref_: &str, action: SurfaceAction) -> Result<()> {
        let registry = self.registry.clone();
        let ref_ = ref_.to_string();
        self.block_on(async move { registry.act(&ref_, action).await })
    }

    pub fn input_blocking(&self, target: &str, event: SurfaceInput) -> Result<()> {
        let registry = self.registry.clone();
        let target = target.to_string();
        self.block_on(async move { registry.input(&target, event).await })
    }

    pub fn poll_events_blocking(&self, target: &str) -> Result<Vec<SurfaceEvent>> {
        let registry = self.registry.clone();
        let target = target.to_string();
        self.block_on(async move { registry.poll_events(&target).await })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::surface::{SurfaceKind, UiNode};
    use std::sync::Mutex;

    /// 脚本化 provider：内存树 + 记录动作，用于验证注册表路由与同步外壳。
    struct ScriptProvider {
        namespace: &'static str,
        surfaces: Vec<SurfaceInfo>,
        tree: Mutex<Option<UiNode>>,
        acted: Mutex<Vec<(String, String)>>,
        fail: bool,
    }

    impl ScriptProvider {
        fn new(namespace: &'static str) -> Self {
            let surface = SurfaceInfo {
                id: format!("{namespace}:1"),
                kind: if namespace == "web" {
                    SurfaceKind::Web
                } else {
                    SurfaceKind::Desktop
                },
                title: format!("{namespace} surface"),
                app: None,
                pid: None,
                url: None,
                bounds: None,
                active: true,
            };
            let root = UiNode::new("r", format!("{namespace}:1:r"), "root").with_name("Root");
            ScriptProvider {
                namespace,
                surfaces: vec![surface],
                tree: Mutex::new(Some(root)),
                acted: Mutex::new(Vec::new()),
                fail: false,
            }
        }
    }

    #[async_trait]
    impl SurfaceProvider for ScriptProvider {
        fn name(&self) -> &'static str {
            "script"
        }
        fn kind(&self) -> SurfaceKind {
            SurfaceKind::Unknown
        }
        fn capabilities(&self) -> SurfaceCapabilities {
            SurfaceCapabilities {
                native_ax: true,
                coordinate_input: true,
                screenshot: false,
                window_management: false,
            }
        }
        fn namespace(&self) -> &'static str {
            self.namespace
        }
        async fn list_surfaces(&self) -> Result<Vec<SurfaceInfo>> {
            if self.fail {
                return Err(EngineError::new(ErrorKind::Internal, "boom"));
            }
            Ok(self.surfaces.clone())
        }
        async fn snapshot(&self, target: &str, _opts: SnapshotOptions) -> Result<SurfaceSnapshot> {
            let root = self.tree.lock().unwrap_or_else(|e| e.into_inner()).clone();
            let surface = self
                .surfaces
                .iter()
                .find(|s| s.id == target)
                .cloned()
                .unwrap_or_else(|| self.surfaces[0].clone());
            Ok(SurfaceSnapshot {
                surface,
                root,
                generation: 1,
                meta: Default::default(),
                timestamp_ms: 0,
            })
        }
        async fn act(&self, ref_: &str, action: SurfaceAction) -> Result<()> {
            self.acted
                .lock()
                .unwrap()
                .push((ref_.to_string(), action.name().to_string()));
            Ok(())
        }
        async fn input(&self, _target: &str, _event: SurfaceInput) -> Result<()> {
            Ok(())
        }
        async fn poll_events(&self, _target: &str) -> Result<Vec<SurfaceEvent>> {
            Ok(vec![SurfaceEvent::new(
                crate::engine::surface::SurfaceEventKind::FocusChanged,
            )])
        }
    }

    fn registry() -> ProviderRegistry {
        let mut r = ProviderRegistry::new();
        r.register(Arc::new(ScriptProvider::new("web")));
        r.register(Arc::new(ScriptProvider::new("desktop")));
        r
    }

    #[test]
    fn registry_routes_by_namespace() {
        let r = registry();
        assert_eq!(r.len(), 2);
        assert_eq!(r.provider_for_ref("web:1:a").unwrap().namespace(), "web");
        assert_eq!(
            r.provider_for_ref("desktop:1:0.1").unwrap().namespace(),
            "desktop"
        );
        assert!(r.provider_for_ref("bogus:1:a").is_none());
        assert!(r.provider_for_ref("bare").is_none());
    }

    #[test]
    fn registry_capabilities_union() {
        let r = registry();
        let c = r.capabilities();
        assert!(c.native_ax && c.coordinate_input);
        assert!(!c.screenshot);
    }

    #[test]
    fn registry_generation_increments() {
        let r = registry();
        assert_eq!(r.next_generation(), 1);
        assert_eq!(r.next_generation(), 2);
    }

    #[tokio::test]
    async fn async_list_and_snapshot() {
        let r = registry();
        let surfaces = r.list_surfaces().await.unwrap();
        assert_eq!(surfaces.len(), 2);
        let snap = r
            .snapshot("web:1", SnapshotOptions::default())
            .await
            .unwrap();
        assert_eq!(snap.surface.id, "web:1");
        assert!(snap.node_by_ref("web:1:r").is_some());
    }

    #[tokio::test]
    async fn async_act_routes() {
        let r = registry();
        r.act("desktop:1:0.1", SurfaceAction::Click).await.unwrap();
        // 断言记录：desktop provider 收到了动作（通过再次 act 验证路由不 panic）
        r.act("web:1:a", SurfaceAction::Invoke).await.unwrap();
    }

    #[test]
    fn blocking_facade_works() {
        let rt = SurfaceRuntime::new(registry()).unwrap();
        assert!(rt.is_available());
        let surfaces = rt.list_surfaces_blocking().unwrap();
        assert_eq!(surfaces.len(), 2);
        let snap = rt
            .snapshot_blocking("desktop:1", SnapshotOptions::default())
            .unwrap();
        assert_eq!(snap.surface.id, "desktop:1");
        rt.act_blocking("web:1:a", SurfaceAction::Click).unwrap();
    }

    #[test]
    fn blocking_unknown_ref_errors() {
        let rt = SurfaceRuntime::new(registry()).unwrap();
        let err = rt.act_blocking("nope", SurfaceAction::Click).unwrap_err();
        assert_eq!(err.kind, ErrorKind::InvalidArgument);
    }

    #[test]
    fn empty_registry_unavailable() {
        let rt = SurfaceRuntime::new(ProviderRegistry::new()).unwrap();
        assert!(!rt.is_available());
        assert!(rt
            .snapshot_blocking("web:1", SnapshotOptions::default())
            .is_err());
    }

    // ── 超时 / 部分失败 / 输入路由 ──────────────────────────────────────

    struct SlowProvider;

    #[async_trait]
    impl SurfaceProvider for SlowProvider {
        fn name(&self) -> &'static str {
            "slow"
        }
        fn kind(&self) -> SurfaceKind {
            SurfaceKind::Unknown
        }
        fn capabilities(&self) -> SurfaceCapabilities {
            SurfaceCapabilities::default()
        }
        fn namespace(&self) -> &'static str {
            "slow"
        }
        async fn list_surfaces(&self) -> Result<Vec<SurfaceInfo>> {
            Ok(Vec::new())
        }
        async fn snapshot(&self, _target: &str, _opts: SnapshotOptions) -> Result<SurfaceSnapshot> {
            tokio::time::sleep(Duration::from_millis(500)).await;
            Err(EngineError::new(
                ErrorKind::Internal,
                "should have timed out",
            ))
        }
        async fn act(&self, _ref_: &str, _action: SurfaceAction) -> Result<()> {
            Ok(())
        }
    }

    struct FailingProvider;

    #[async_trait]
    impl SurfaceProvider for FailingProvider {
        fn name(&self) -> &'static str {
            "failing"
        }
        fn kind(&self) -> SurfaceKind {
            SurfaceKind::Unknown
        }
        fn capabilities(&self) -> SurfaceCapabilities {
            SurfaceCapabilities::default()
        }
        fn namespace(&self) -> &'static str {
            "failing"
        }
        async fn list_surfaces(&self) -> Result<Vec<SurfaceInfo>> {
            Err(EngineError::new(ErrorKind::Internal, "boom"))
        }
        async fn snapshot(&self, _target: &str, _opts: SnapshotOptions) -> Result<SurfaceSnapshot> {
            Err(EngineError::new(ErrorKind::Internal, "boom"))
        }
        async fn act(&self, _ref_: &str, _action: SurfaceAction) -> Result<()> {
            Err(EngineError::new(ErrorKind::Internal, "boom"))
        }
    }

    #[test]
    fn timeout_returns_timeout_error() {
        let mut r = ProviderRegistry::new();
        r.register(Arc::new(SlowProvider));
        let rt = SurfaceRuntime::with_timeout(r, Duration::from_millis(20)).unwrap();
        let err = rt
            .snapshot_blocking("slow:1", SnapshotOptions::default())
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::Timeout);
    }

    #[tokio::test]
    async fn list_surfaces_tolerates_partial_failure() {
        let mut r = ProviderRegistry::new();
        r.register(Arc::new(ScriptProvider::new("web")));
        r.register(Arc::new(FailingProvider));
        let surfaces = r.list_surfaces().await.unwrap();
        assert_eq!(
            surfaces.len(),
            1,
            "healthy provider's surfaces should survive"
        );
        assert!(surfaces[0].id.starts_with("web:"));
    }

    #[tokio::test]
    async fn all_providers_failing_returns_error() {
        let mut r = ProviderRegistry::new();
        r.register(Arc::new(FailingProvider));
        assert!(r.list_surfaces().await.is_err());
    }

    #[tokio::test]
    async fn list_surfaces_report_surfaces_and_skipped() {
        let mut r = ProviderRegistry::new();
        r.register(Arc::new(ScriptProvider::new("web")));
        r.register(Arc::new(FailingProvider));
        let report = r.list_surfaces_report().await;
        assert_eq!(report.surfaces.len(), 1);
        assert!(report.surfaces[0].id.starts_with("web:"));
        // The failing provider is reported (name + message) instead of silently
        // omitted — the agent can then tell native AX is unavailable.
        assert_eq!(report.skipped.len(), 1);
        assert_eq!(report.skipped[0].0, "failing");
        assert!(report.skipped[0].1.contains("boom"), "{:?}", report.skipped);
    }

    #[test]
    fn list_surfaces_report_blocking_no_providers_is_empty() {
        let rt = SurfaceRuntime::new(ProviderRegistry::new()).unwrap();
        let report = rt.list_surfaces_report_blocking();
        assert!(report.surfaces.is_empty() && report.skipped.is_empty());
    }

    #[test]
    fn input_routes_to_coordinate_provider() {
        let rt = SurfaceRuntime::new(registry()).unwrap();
        // 空 target → 选首个具备 coordinate_input 的 provider（script）。
        rt.input_blocking(
            "",
            crate::engine::InputEvent::Key(crate::engine::KeyEvent::press("Tab")),
        )
        .unwrap();
    }

    #[test]
    fn provider_by_namespace_lookup() {
        let r = registry();
        assert!(r.provider_by_namespace("web").is_some());
        assert!(r.provider_by_namespace("desktop").is_some());
        assert!(r.provider_by_namespace("nope").is_none());
        assert!(r.input_provider().is_some());
    }

    #[tokio::test]
    async fn poll_events_aggregates_providers() {
        let r = registry(); // web + desktop script providers
        let events = r.poll_events("").await.unwrap();
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn blocking_poll_events() {
        let rt = SurfaceRuntime::new(registry()).unwrap();
        assert_eq!(rt.poll_events_blocking("").unwrap().len(), 2);
    }
}
