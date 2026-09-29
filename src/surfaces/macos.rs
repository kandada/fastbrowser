// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! macOS 原生无障碍后端（feature `surface-macos`）。
//!
//! 通过 `AXUIElement`（系统 ApplicationServices 框架）感知与操作原生应用，
//! 通过 `CGEvent`（CoreGraphics）做坐标级输入。命名空间 `desktop`。
//!
//! 设计要点：
//! - **异步外壳 + 阻塞内核**：AX/CG 是同步 C API，统一经 `spawn_blocking` 执行，
//!   外层由 `SurfaceRuntime` 施加超时；对无响应应用先设 AX messaging timeout。
//! - **稳定引用**：ref = `desktop:<pid>:<path>`，`path` 为从应用根出发的子节点
//!   下标（`0.2.1`）。每次动作按 path 重新走树解析，避免缓存 AXUIElement 失效。
//! - **权限**：未授予「辅助功能」权限时返回明确错误，绝不静默失败。

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use accessibility::{AXAttribute, AXUIElement, AXUIElementActions};
use accessibility_sys::{
    kAXValueTypeCGPoint, kAXValueTypeCGSize, AXIsProcessTrusted, AXObserverAddNotification,
    AXObserverCreate, AXObserverGetRunLoopSource, AXObserverRef, AXUIElementCopyAttributeValue,
    AXUIElementCreateApplication, AXUIElementGetPid, AXUIElementRef, AXUIElementSetAttributeValue,
    AXValueGetValue, AXValueRef,
};
use core_foundation::base::{CFRelease, CFType, CFTypeRef, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::number::CFNumber;
use core_foundation::runloop::{kCFRunLoopCommonModes, CFRunLoop, CFRunLoopSource};
use core_foundation::string::{CFString, CFStringRef};
use core_graphics::event::{CGEvent, CGEventTapLocation, CGEventType, CGMouseButton};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use core_graphics::geometry::{CGPoint, CGSize};

use crate::engine::surface::{
    prune, SnapshotOptions, SurfaceAction, SurfaceCapabilities, SurfaceEvent, SurfaceEventKind,
    SurfaceInfo, SurfaceKind, SurfaceSnapshot, UiNode, UiState,
};
use crate::engine::{EngineError, ErrorKind, Image, InputEvent, KeyKind, MouseKind, Rect, Result};

/// 单次 AX 遍历的硬上限（在 prune 之前，防止构建出巨大树）。
const HARD_NODE_CAP: usize = 4000;
/// 每个元素的 AX messaging 超时（秒）：避免无响应应用卡死遍历。
const AX_MESSAGING_TIMEOUT_SECS: f32 = 2.0;
/// 事件队列上限（溢出丢弃最旧，避免无界增长）。
const EVENT_QUEUE_CAP: usize = 1024;

/// 当前进程是否已获「辅助功能」权限。
pub fn is_trusted() -> bool {
    unsafe { AXIsProcessTrusted() }
}

/// 监听的通知（聚焦应用内）。
const OBSERVED_NOTIFICATIONS: &[&str] = &[
    "AXFocusedUIElementChanged",
    "AXFocusedWindowChanged",
    "AXMainWindowChanged",
    "AXWindowCreated",
    "AXUIElementDestroyed",
    "AXValueChanged",
    "AXTitleChanged",
    "AXSelectedTextChanged",
    "AXSelectedChildrenChanged",
    "AXLayoutChanged",
];

/// 通知名 → 统一事件类型（纯函数，可测）。
pub fn event_kind_for(notification: &str) -> SurfaceEventKind {
    match notification {
        "AXFocusedUIElementChanged"
        | "AXFocusedWindowChanged"
        | "AXMainWindowChanged"
        | "AXApplicationActivated" => SurfaceEventKind::FocusChanged,
        "AXWindowCreated" => SurfaceEventKind::WindowCreated,
        "AXValueChanged" => SurfaceEventKind::ValueChanged,
        "AXTitleChanged" => SurfaceEventKind::TitleChanged,
        "AXSelectedTextChanged"
        | "AXSelectedChildrenChanged"
        | "AXSelectedRowsChanged"
        | "AXSelectedColumnsChanged"
        | "AXSelectedCellsChanged" => SurfaceEventKind::SelectionChanged,
        "AXUIElementDestroyed" | "AXLayoutChanged" | "AXCreated" => {
            SurfaceEventKind::StructureChanged
        }
        _ => SurfaceEventKind::Unknown,
    }
}

/// AXObserver 事件收集器：后台线程跑 CFRunLoop，回调入队；`poll` 时 drain。
struct EventCollector {
    queue: Arc<Mutex<Vec<SurfaceEvent>>>,
    handle: Mutex<Option<CollectorHandle>>,
    started: AtomicBool,
}

struct CollectorHandle {
    runloop: CFRunLoop,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl EventCollector {
    fn new() -> Self {
        EventCollector {
            queue: Arc::new(Mutex::new(Vec::new())),
            handle: Mutex::new(None),
            started: AtomicBool::new(false),
        }
    }

    /// 为指定 pid 启动观察者（幂等；失败则重置以便重试）。
    fn ensure_started(&self, pid: i32) {
        if self.started.swap(true, Ordering::SeqCst) {
            return;
        }
        let queue = self.queue.clone();
        let (tx, rx) = std::sync::mpsc::channel::<Option<CFRunLoop>>();
        let thread = std::thread::Builder::new()
            .name("fastbrowser-ax-events".into())
            .spawn(move || {
                unsafe { run_observer(pid, queue, tx) };
            });
        let thread = match thread {
            Ok(t) => t,
            Err(_) => {
                self.started.store(false, Ordering::SeqCst);
                return;
            }
        };
        match rx.recv_timeout(std::time::Duration::from_secs(3)) {
            Ok(Some(runloop)) => {
                *self.handle.lock().unwrap_or_else(|e| e.into_inner()) = Some(CollectorHandle {
                    runloop,
                    thread: Some(thread),
                });
            }
            _ => {
                // 观察者创建失败（无权限/无前台应用）：允许下次重试。
                self.started.store(false, Ordering::SeqCst);
            }
        }
    }

    fn drain(&self) -> Vec<SurfaceEvent> {
        let mut q = self.queue.lock().unwrap_or_else(|e| e.into_inner());
        std::mem::take(&mut *q)
    }
}

impl Drop for EventCollector {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.lock().unwrap_or_else(|e| e.into_inner()).take() {
            handle.runloop.stop();
            if let Some(t) = handle.thread {
                let _ = t.join();
            }
        }
    }
}

/// 在专用线程上创建观察者、注册通知、跑 CFRunLoop。
unsafe fn run_observer(
    pid: i32,
    queue: Arc<Mutex<Vec<SurfaceEvent>>>,
    tx: std::sync::mpsc::Sender<Option<CFRunLoop>>,
) {
    let mut observer: AXObserverRef = std::ptr::null_mut();
    let err = AXObserverCreate(pid, ax_event_callback, &mut observer);
    if err != 0 || observer.is_null() {
        let _ = tx.send(None);
        return;
    }
    let app = AXUIElementCreateApplication(pid);
    let refcon = Arc::as_ptr(&queue) as *mut c_void;
    for notif in OBSERVED_NOTIFICATIONS {
        let name = CFString::new(notif);
        let _ = AXObserverAddNotification(observer, app, name.as_concrete_TypeRef(), refcon);
    }
    let source_ref = AXObserverGetRunLoopSource(observer);
    if source_ref.is_null() {
        let _ = tx.send(None);
        return;
    }
    let source = CFRunLoopSource::wrap_under_get_rule(source_ref);
    let runloop = CFRunLoop::get_current();
    runloop.add_source(&source, kCFRunLoopCommonModes);
    let _ = tx.send(Some(runloop.clone()));

    CFRunLoop::run_current();

    // 线程退出前释放（refcon 指向的 Arc 由本线程的 `queue` 持有，此处仍有效）。
    CFRelease(observer as CFTypeRef);
    CFRelease(app as CFTypeRef);
}

/// AXObserver 回调：把通知转成 `SurfaceEvent` 入队。
unsafe extern "C" fn ax_event_callback(
    _observer: AXObserverRef,
    element: AXUIElementRef,
    notification: CFStringRef,
    refcon: *mut c_void,
) {
    if refcon.is_null() {
        return;
    }
    let queue = &*(refcon as *const Mutex<Vec<SurfaceEvent>>);
    let notif = CFString::wrap_under_get_rule(notification).to_string();
    let kind = event_kind_for(&notif);
    let (role, name) = ax_element_role_name(element);
    let ev = SurfaceEvent::new(kind)
        .with_surface("desktop:focus")
        .with_element(role, name);
    if let Ok(mut q) = queue.lock() {
        if q.len() >= EVENT_QUEUE_CAP {
            let excess = q.len() + 1 - EVENT_QUEUE_CAP;
            q.drain(0..excess);
        }
        q.push(ev);
    }
}

/// 读取元素的 role/title（用于事件标注）。
unsafe fn ax_element_role_name(el: AXUIElementRef) -> (Option<String>, Option<String>) {
    let role = ax_copy_string(el, "AXRole");
    let name = ax_copy_string(el, "AXTitle").or_else(|| ax_copy_string(el, "AXDescription"));
    (role, name)
}

unsafe fn ax_copy_string(el: AXUIElementRef, attr: &str) -> Option<String> {
    let attr_cf = CFString::new(attr);
    let mut val: CFTypeRef = std::ptr::null();
    let err = AXUIElementCopyAttributeValue(el, attr_cf.as_concrete_TypeRef(), &mut val);
    if err != 0 || val.is_null() {
        return None;
    }
    let cf = CFType::wrap_under_create_rule(val);
    cf.downcast::<CFString>().map(|s| s.to_string())
}

/// macOS 原生表面 provider。
pub struct MacSurface {
    generation: AtomicU64,
    events: Arc<EventCollector>,
}

impl Default for MacSurface {
    fn default() -> Self {
        Self::new()
    }
}

impl MacSurface {
    pub fn new() -> Self {
        MacSurface {
            generation: AtomicU64::new(0),
            events: Arc::new(EventCollector::new()),
        }
    }

    async fn blocking<T, F>(f: F) -> Result<T>
    where
        F: FnOnce() -> Result<T> + Send + 'static,
        T: Send + 'static,
    {
        tokio::task::spawn_blocking(f).await.map_err(|e| {
            EngineError::new(ErrorKind::Internal, format!("macos surface join: {e}"))
        })?
    }

    fn ensure_trusted() -> Result<()> {
        if is_trusted() {
            Ok(())
        } else {
            Err(EngineError::new(
                ErrorKind::Unsupported,
                "macOS accessibility permission not granted; enable it in \
                 System Settings → Privacy & Security → Accessibility",
            ))
        }
    }

    /// 解析目标为应用元素：`desktop:<pid>` / `desktop:<pid>:<path>` / 空或 "focused"。
    fn app_for_target(target: &str) -> Result<(AXUIElement, i32, String)> {
        let t = target.trim();
        if t.is_empty() || t == "focused" || t == "desktop" {
            let app = focused_application().ok_or_else(|| {
                EngineError::new(
                    ErrorKind::Unsupported,
                    "no focused application (check accessibility permission)",
                )
            })?;
            let pid = element_pid(&app).unwrap_or(0);
            return Ok((app, pid, String::new()));
        }
        let rest = t.strip_prefix("desktop:").unwrap_or(t);
        let mut it = rest.splitn(2, ':');
        let pid: i32 = it
            .next()
            .unwrap_or("")
            .parse()
            .map_err(|_| EngineError::invalid(format!("bad pid in target '{target}'")))?;
        let path = it.next().unwrap_or("").to_string();
        Ok((AXUIElement::application(pid), pid, path))
    }
}

#[async_trait]
impl crate::engine::SurfaceProvider for MacSurface {
    fn name(&self) -> &'static str {
        "macos"
    }

    fn kind(&self) -> SurfaceKind {
        SurfaceKind::App
    }

    fn capabilities(&self) -> SurfaceCapabilities {
        SurfaceCapabilities {
            native_ax: true,
            coordinate_input: true,
            screenshot: true,
            window_management: true,
        }
    }

    fn namespace(&self) -> &'static str {
        "desktop"
    }

    async fn list_surfaces(&self) -> Result<Vec<SurfaceInfo>> {
        Self::blocking(|| {
            Self::ensure_trusted()?;
            let Some(app) = focused_application() else {
                return Ok(Vec::new());
            };
            let pid = element_pid(&app).unwrap_or(0);
            let title = element_name(&app).unwrap_or_else(|| format!("pid {pid}"));
            let bounds = element_bounds(&app);
            Ok(vec![SurfaceInfo {
                id: format!("desktop:{pid}"),
                kind: SurfaceKind::App,
                title,
                app: None,
                pid: Some(pid),
                url: None,
                bounds,
                active: true,
            }])
        })
        .await
    }

    async fn snapshot(&self, target: &str, opts: SnapshotOptions) -> Result<SurfaceSnapshot> {
        let target = target.to_string();
        let snapshot = Self::blocking(move || {
            Self::ensure_trusted()?;
            let (app, pid, _path) = Self::app_for_target(&target)?;
            let _ = app.set_messaging_timeout(AX_MESSAGING_TIMEOUT_SECS);
            let prefix = format!("desktop:{pid}:");
            let mut counter = 0usize;
            let mut hit_cap = false;
            let root = build_node(&app, "", &prefix, &mut counter, &mut hit_cap);
            let root = root.ok_or_else(|| {
                EngineError::new(ErrorKind::Snapshot, "failed to build accessibility tree")
            })?;
            let (root, mut meta) = prune(root, &opts);
            if hit_cap {
                meta.truncated = true;
                if meta.reason.is_none() {
                    meta.reason = Some("nodes".into());
                }
            }
            let title = element_name(&app).unwrap_or_else(|| format!("pid {pid}"));
            let surface = SurfaceInfo {
                id: format!("desktop:{pid}"),
                kind: SurfaceKind::App,
                title,
                app: None,
                pid: Some(pid),
                url: None,
                bounds: element_bounds(&app),
                active: true,
            };
            Ok(SurfaceSnapshot {
                surface,
                root,
                generation: 0, // 由下方填充
                meta,
                timestamp_ms: 0,
            })
        })
        .await?;
        let mut snapshot = snapshot;
        snapshot.generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        Ok(snapshot)
    }

    async fn act(&self, ref_: &str, action: SurfaceAction) -> Result<()> {
        let (pid, path) = parse_desktop_ref(ref_)?;
        let ref_owned = ref_.to_string();
        Self::blocking(move || {
            Self::ensure_trusted()?;
            let app = AXUIElement::application(pid);
            let _ = app.set_messaging_timeout(AX_MESSAGING_TIMEOUT_SECS);
            let element = resolve_path(&app, &path).ok_or_else(|| {
                EngineError::new(
                    ErrorKind::Dom,
                    format!("element not found for ref '{ref_owned}'"),
                )
            })?;

            match action {
                SurfaceAction::Click | SurfaceAction::Invoke => element.press().map_err(ax_err)?,
                SurfaceAction::Focus => set_ax_bool(&element, "AXFocused", true)?,
                SurfaceAction::SetValue { value } => set_ax_value(&element, &value)?,
                SurfaceAction::TypeText { text, clear } => {
                    let final_text = if clear {
                        text
                    } else {
                        let existing = element
                            .attribute(&AXAttribute::value())
                            .ok()
                            .and_then(|v| cftype_to_string(&v))
                            .unwrap_or_default();
                        format!("{existing}{text}")
                    };
                    set_ax_value(&element, &final_text)?;
                }
                SurfaceAction::Check { checked } => {
                    set_ax_value(&element, if checked { "1" } else { "0" })?
                }
                SurfaceAction::Select { value } => set_ax_value(&element, &value)?,
                SurfaceAction::Increment => element.increment().map_err(ax_err)?,
                SurfaceAction::Decrement => element.decrement().map_err(ax_err)?,
                SurfaceAction::ShowMenu => element.show_menu().map_err(ax_err)?,
                SurfaceAction::Raise => element.raise().map_err(ax_err)?,
                SurfaceAction::DoubleClick
                | SurfaceAction::RightClick
                | SurfaceAction::Scroll { .. } => {
                    let rect = element_bounds(&element).ok_or_else(|| {
                        EngineError::new(
                            ErrorKind::Dom,
                            "element has no geometry for pointer action",
                        )
                    })?;
                    let (cx, cy) = rect.center();
                    match action {
                        SurfaceAction::DoubleClick => post_mouse(
                            cx,
                            cy,
                            CGEventType::LeftMouseDown,
                            CGEventType::LeftMouseUp,
                            CGMouseButton::Left,
                            2,
                        )?,
                        SurfaceAction::RightClick => post_mouse(
                            cx,
                            cy,
                            CGEventType::RightMouseDown,
                            CGEventType::RightMouseUp,
                            CGMouseButton::Right,
                            1,
                        )?,
                        SurfaceAction::Scroll { dx, dy } => post_scroll(cx, cy, dx, dy)?,
                        _ => unreachable!(),
                    }
                }
                SurfaceAction::PressKey { key } => post_key(&key, true)?,
            }
            Ok(())
        })
        .await
    }

    async fn input(&self, _target: &str, event: InputEvent) -> Result<()> {
        Self::blocking(move || {
            Self::ensure_trusted()?;
            match event {
                InputEvent::Mouse(m) => {
                    let (down, up, button) = match m.button {
                        crate::engine::MouseButton::Right => (
                            CGEventType::RightMouseDown,
                            CGEventType::RightMouseUp,
                            CGMouseButton::Right,
                        ),
                        crate::engine::MouseButton::Middle => (
                            CGEventType::OtherMouseDown,
                            CGEventType::OtherMouseUp,
                            CGMouseButton::Center,
                        ),
                        _ => (
                            CGEventType::LeftMouseDown,
                            CGEventType::LeftMouseUp,
                            CGMouseButton::Left,
                        ),
                    };
                    match m.kind {
                        MouseKind::Move => post_mouse_move(m.x, m.y),
                        MouseKind::DoubleClick => post_mouse(m.x, m.y, down, up, button, 2),
                        MouseKind::Down => post_single(m.x, m.y, down),
                        MouseKind::Up => post_single(m.x, m.y, up),
                        MouseKind::Click => post_mouse(m.x, m.y, down, up, button, 1),
                    }
                }
                InputEvent::Wheel(w) => post_scroll(w.x, w.y, w.delta_x, w.delta_y),
                InputEvent::Key(k) => {
                    if k.kind == KeyKind::Press && k.text.chars().count() > 1 {
                        post_unicode(&k.text)
                    } else if k.kind == KeyKind::Down {
                        post_key(&k.key, true)
                    } else if k.kind == KeyKind::Up {
                        post_key(&k.key, false)
                    } else {
                        post_key(&k.key, true)
                    }
                }
                InputEvent::Touch(_) => Err(EngineError::unsupported(
                    "macOS surface does not support touch injection",
                )),
            }
        })
        .await
    }

    /// 全屏截图（`screencapture -x -t png` → 复用内核 PNG 解码）。
    ///
    /// 需要「屏幕录制」权限；未授予时返回明确错误。用独立临时文件 + 清理，
    /// 避免污染工作目录。
    async fn screenshot(&self, _target: &str) -> Result<Image> {
        Self::blocking(|| {
            let tmp = std::env::temp_dir().join(format!(
                "fastbrowser-shot-{}-{}.png",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis())
                    .unwrap_or(0)
            ));
            let output = std::process::Command::new("screencapture")
                .arg("-x") // 静音
                .arg("-t")
                .arg("png")
                .arg(&tmp)
                .output()
                .map_err(|e| {
                    EngineError::new(ErrorKind::Io, format!("screencapture spawn failed: {e}"))
                })?;
            if !output.status.success() {
                let _ = std::fs::remove_file(&tmp);
                return Err(EngineError::new(
                    ErrorKind::Io,
                    "screencapture failed (grant Screen Recording permission to the host app)",
                ));
            }
            let bytes = std::fs::read(&tmp).map_err(EngineError::from)?;
            let _ = std::fs::remove_file(&tmp);
            Image::from_png(&bytes)
        })
        .await
    }

    /// 拉取自上次调用以来的原生事件（AXObserver）。
    ///
    /// 首次调用时为当前聚焦应用启动观察者（后台 CFRunLoop 线程）；无权限/无前台
    /// 应用时返回空列表。事件是「推」语义，调用方按需 drain。
    async fn poll_events(&self, _target: &str) -> Result<Vec<SurfaceEvent>> {
        let events = self.events.clone();
        Self::blocking(move || {
            if is_trusted() {
                if let Some(app) = focused_application() {
                    if let Some(pid) = element_pid(&app) {
                        events.ensure_started(pid);
                    }
                }
            }
            Ok(events.drain())
        })
        .await
    }
}

// ─────────────────────────── AX 树构建 ───────────────────────────

fn build_node(
    el: &AXUIElement,
    path: &str,
    prefix: &str,
    counter: &mut usize,
    hit_cap: &mut bool,
) -> Option<UiNode> {
    if *counter >= HARD_NODE_CAP {
        *hit_cap = true;
        return None;
    }
    *counter += 1;

    let ax_role = attr_string(el, &AXAttribute::role()).unwrap_or_default();
    let role = map_role(&ax_role);
    let title = attr_string(el, &AXAttribute::title());
    let value = el
        .attribute(&AXAttribute::value())
        .ok()
        .and_then(|v| cftype_to_string(&v));
    let description = attr_string(el, &AXAttribute::description());
    let identifier = attr_string(el, &AXAttribute::identifier());

    let name = title
        .clone()
        .filter(|s| !s.is_empty())
        .or_else(|| description.clone().filter(|s| !s.is_empty()))
        .or_else(|| {
            if matches!(role.as_str(), "text" | "statictext" | "textbox") {
                value.clone().filter(|s| !s.is_empty())
            } else {
                None
            }
        });

    let node_id = if path.is_empty() {
        "root".to_string()
    } else {
        path.to_string()
    };
    let ref_ = format!("{prefix}{path}");
    let mut node = UiNode::new(node_id, ref_, role.clone()).with_source(SurfaceKind::App);
    node.name = name;
    node.value = value.clone();
    node.description = description;
    if let Some(id) = identifier.filter(|s| !s.is_empty()) {
        node.attrs.insert("identifier".into(), id);
    }
    if let Some(b) = element_bounds(el) {
        node.geometry = Some(b);
    }
    node.states = element_states(el, &role, value.as_deref());
    node.actions = element_actions(el);
    node.backend = Some(path.to_string());

    // 子节点：应用根用 AXWindows，其余用 AXChildren（确定性顺序，供 path 解析）。
    let children = child_elements(el, &ax_role);
    let max_children = children.len().min(HARD_NODE_CAP.saturating_sub(*counter));
    for (i, child) in children.iter().take(max_children).enumerate() {
        let child_path = if path.is_empty() {
            i.to_string()
        } else {
            format!("{path}.{i}")
        };
        if let Some(c) = build_node(child, &child_path, prefix, counter, hit_cap) {
            node.children.push(c);
        }
    }
    if children.len() > max_children {
        *hit_cap = true;
    }
    Some(node)
}

/// 确定性子元素列表。
fn child_elements(el: &AXUIElement, ax_role: &str) -> Vec<AXUIElement> {
    if ax_role == "AXApplication" {
        if let Ok(windows) = el.attribute(&AXAttribute::windows()) {
            let v: Vec<AXUIElement> = windows.iter().map(|c| c.clone()).collect();
            if !v.is_empty() {
                return v;
            }
        }
    }
    match el.attribute(&AXAttribute::children()) {
        Ok(children) => children.iter().map(|c| c.clone()).collect(),
        Err(_) => Vec::new(),
    }
}

/// 按 path（`0.2.1`）从根解析元素。
fn resolve_path(root: &AXUIElement, path: &str) -> Option<AXUIElement> {
    if path.is_empty() {
        return Some(root.clone());
    }
    let mut current = root.clone();
    for seg in path.split('.') {
        let idx: usize = seg.parse().ok()?;
        let role = attr_string(&current, &AXAttribute::role()).unwrap_or_default();
        let children = child_elements(&current, &role);
        current = children.into_iter().nth(idx)?;
    }
    Some(current)
}

// ─────────────────────────── AX 属性读取 ───────────────────────────

fn focused_application() -> Option<AXUIElement> {
    let system = AXUIElement::system_wide();
    let attr = AXAttribute::<CFType>::new(&CFString::from_static_string("AXFocusedApplication"));
    let cf = system.attribute(&attr).ok()?;
    cf.downcast::<AXUIElement>()
}

fn element_pid(el: &AXUIElement) -> Option<i32> {
    let mut pid: i32 = 0;
    let rc = unsafe { AXUIElementGetPid(el.as_concrete_TypeRef(), &mut pid) };
    if rc == 0 {
        Some(pid)
    } else {
        None
    }
}

fn attr_string(el: &AXUIElement, attr: &AXAttribute<CFString>) -> Option<String> {
    el.attribute(attr).ok().map(|s| s.to_string())
}

fn element_name(el: &AXUIElement) -> Option<String> {
    attr_string(el, &AXAttribute::title())
        .filter(|s| !s.is_empty())
        .or_else(|| attr_string(el, &AXAttribute::description()).filter(|s| !s.is_empty()))
}

fn element_states(el: &AXUIElement, role: &str, value: Option<&str>) -> UiState {
    let mut s = UiState::default();
    if let Ok(b) = el.attribute(&AXAttribute::enabled()) {
        s.disabled = Some(!bool::from(b));
    }
    if let Ok(b) = el.attribute(&AXAttribute::focused()) {
        s.focused = Some(bool::from(b));
    }
    if matches!(role, "checkbox" | "radio" | "switch") {
        s.checked = value.map(|v| v == "1" || v.eq_ignore_ascii_case("true"));
    }
    if matches!(role, "textbox" | "combobox" | "searchbox") {
        s.editable = Some(true);
    }
    s
}

fn element_actions(el: &AXUIElement) -> Vec<String> {
    match el.action_names() {
        Ok(names) => names.iter().map(|n| map_action(&n.to_string())).collect(),
        Err(_) => Vec::new(),
    }
}

fn element_bounds(el: &AXUIElement) -> Option<Rect> {
    let pos_attr = AXAttribute::<CFType>::new(&CFString::from_static_string("AXPosition"));
    let size_attr = AXAttribute::<CFType>::new(&CFString::from_static_string("AXSize"));
    let pos = el
        .attribute(&pos_attr)
        .ok()
        .and_then(|cf| decode_point(&cf))?;
    let size = el
        .attribute(&size_attr)
        .ok()
        .and_then(|cf| decode_size(&cf))?;
    Some(Rect::new(pos.x, pos.y, size.width, size.height))
}

fn decode_point(cf: &CFType) -> Option<CGPoint> {
    let raw = cf.as_CFTypeRef() as AXValueRef;
    let mut p = CGPoint::new(0.0, 0.0);
    let ok = unsafe {
        AXValueGetValue(
            raw,
            kAXValueTypeCGPoint,
            &mut p as *mut CGPoint as *mut c_void,
        )
    };
    ok.then_some(p)
}

fn decode_size(cf: &CFType) -> Option<CGSize> {
    let raw = cf.as_CFTypeRef() as AXValueRef;
    let mut s = CGSize::new(0.0, 0.0);
    let ok = unsafe {
        AXValueGetValue(
            raw,
            kAXValueTypeCGSize,
            &mut s as *mut CGSize as *mut c_void,
        )
    };
    ok.then_some(s)
}

fn cftype_to_string(cf: &CFType) -> Option<String> {
    if let Some(s) = cf.downcast::<CFString>() {
        return Some(s.to_string());
    }
    if let Some(n) = cf.downcast::<CFNumber>() {
        return n.to_f64().map(|f| {
            if f.fract() == 0.0 {
                format!("{}", f as i64)
            } else {
                format!("{f}")
            }
        });
    }
    if let Some(b) = cf.downcast::<CFBoolean>() {
        return Some(bool::from(b).to_string());
    }
    None
}

fn set_ax_value(el: &AXUIElement, value: &str) -> Result<()> {
    let attr = CFString::from_static_string("AXValue");
    let val = CFString::new(value);
    let rc = unsafe {
        AXUIElementSetAttributeValue(
            el.as_concrete_TypeRef(),
            attr.as_concrete_TypeRef(),
            val.as_CFTypeRef(),
        )
    };
    if rc == 0 {
        Ok(())
    } else {
        Err(EngineError::new(
            ErrorKind::Dom,
            format!("AXUIElementSetAttributeValue failed (AXError {rc})"),
        ))
    }
}

fn set_ax_bool(el: &AXUIElement, attr_name: &str, value: bool) -> Result<()> {
    let attr = CFString::new(attr_name);
    let val = CFBoolean::from(value);
    let rc = unsafe {
        AXUIElementSetAttributeValue(
            el.as_concrete_TypeRef(),
            attr.as_concrete_TypeRef(),
            val.as_CFTypeRef(),
        )
    };
    if rc == 0 {
        Ok(())
    } else {
        Err(EngineError::new(
            ErrorKind::Dom,
            format!("AXUIElementSetAttributeValue({attr_name}) failed (AXError {rc})"),
        ))
    }
}

fn ax_err(e: accessibility::Error) -> EngineError {
    EngineError::new(ErrorKind::Dom, format!("accessibility: {e}"))
}

// ─────────────────────────── 坐标输入（CGEvent）───────────────────────────

fn event_source() -> Result<CGEventSource> {
    CGEventSource::new(CGEventSourceStateID::HIDSystemState)
        .map_err(|_| EngineError::new(ErrorKind::Input, "failed to create CGEventSource"))
}

fn post_single(x: f64, y: f64, event_type: CGEventType) -> Result<()> {
    let source = event_source()?;
    let ev = CGEvent::new_mouse_event(source, event_type, CGPoint::new(x, y), CGMouseButton::Left)
        .map_err(|_| EngineError::new(ErrorKind::Input, "CGEvent mouse creation failed"))?;
    ev.post(CGEventTapLocation::HID);
    Ok(())
}

fn post_mouse_move(x: f64, y: f64) -> Result<()> {
    post_single(x, y, CGEventType::MouseMoved)
}

fn post_mouse(
    x: f64,
    y: f64,
    down: CGEventType,
    up: CGEventType,
    button: CGMouseButton,
    click_count: u64,
) -> Result<()> {
    let source = event_source()?;
    for (ev_type, is_down) in [(down, true), (up, false)] {
        let ev = CGEvent::new_mouse_event(source.clone(), ev_type, CGPoint::new(x, y), button)
            .map_err(|_| EngineError::new(ErrorKind::Input, "CGEvent mouse creation failed"))?;
        let _ = is_down;
        if click_count > 1 {
            ev.set_integer_value_field(
                core_graphics::event::EventField::MOUSE_EVENT_CLICK_STATE,
                click_count as i64,
            );
        }
        ev.post(CGEventTapLocation::HID);
    }
    Ok(())
}

fn post_scroll(x: f64, y: f64, dx: f64, dy: f64) -> Result<()> {
    let source = event_source()?;
    // 先把指针移到目标点，再发送滚轮事件。
    post_mouse_move(x, y)?;
    let ev = CGEvent::new_scroll_event(
        source,
        core_graphics::event::ScrollEventUnit::PIXEL,
        2,
        dy as i32,
        dx as i32,
        0,
    )
    .map_err(|_| EngineError::new(ErrorKind::Input, "CGEvent scroll creation failed"))?;
    ev.post(CGEventTapLocation::HID);
    Ok(())
}

fn post_key(key: &str, down: bool) -> Result<()> {
    let code = keycode_for(key)
        .ok_or_else(|| EngineError::new(ErrorKind::Input, format!("unsupported key '{key}'")))?;
    let source = event_source()?;
    let ev = CGEvent::new_keyboard_event(source, code, down)
        .map_err(|_| EngineError::new(ErrorKind::Input, "CGEvent key creation failed"))?;
    ev.post(CGEventTapLocation::HID);
    Ok(())
}

fn post_unicode(text: &str) -> Result<()> {
    let source = event_source()?;
    for ch in text.chars() {
        let mut buf: Vec<u16> = vec![0; 2];
        let encoded = ch.encode_utf16(&mut buf);
        let ev = CGEvent::new_keyboard_event(source.clone(), 0, true)
            .map_err(|_| EngineError::new(ErrorKind::Input, "CGEvent key creation failed"))?;
        ev.set_string_from_utf16_unchecked(encoded);
        ev.post(CGEventTapLocation::HID);
        let ev_up = CGEvent::new_keyboard_event(source.clone(), 0, false)
            .map_err(|_| EngineError::new(ErrorKind::Input, "CGEvent key creation failed"))?;
        ev_up.post(CGEventTapLocation::HID);
    }
    Ok(())
}

fn keycode_for(key: &str) -> Option<u16> {
    use core_graphics::event::KeyCode;
    let k = key.trim();
    let upper = k.to_ascii_uppercase();
    let code = match upper.as_str() {
        "RETURN" | "ENTER" => KeyCode::RETURN,
        "TAB" => KeyCode::TAB,
        "SPACE" => KeyCode::SPACE,
        "DELETE" | "BACKSPACE" => KeyCode::DELETE,
        "ESCAPE" | "ESC" => KeyCode::ESCAPE,
        "UP" | "ARROWUP" => KeyCode::UP_ARROW,
        "DOWN" | "ARROWDOWN" => KeyCode::DOWN_ARROW,
        "LEFT" | "ARROWLEFT" => KeyCode::LEFT_ARROW,
        "RIGHT" | "ARROWRIGHT" => KeyCode::RIGHT_ARROW,
        "HOME" => KeyCode::HOME,
        "END" => KeyCode::END,
        "PAGEUP" => KeyCode::PAGE_UP,
        "PAGEDOWN" => KeyCode::PAGE_DOWN,
        "COMMAND" | "META" => KeyCode::COMMAND,
        "SHIFT" => KeyCode::SHIFT,
        "OPTION" | "ALT" => KeyCode::OPTION,
        "CONTROL" | "CTRL" => KeyCode::CONTROL,
        other if other.chars().count() == 1 => single_keycode(other.chars().next().unwrap())?,
        _ => return None,
    };
    Some(code)
}

fn single_keycode(c: char) -> Option<u16> {
    use core_graphics::event::KeyCode;
    Some(match c.to_ascii_uppercase() {
        'A' => KeyCode::ANSI_A,
        'B' => KeyCode::ANSI_B,
        'C' => KeyCode::ANSI_C,
        'D' => KeyCode::ANSI_D,
        'E' => KeyCode::ANSI_E,
        'F' => KeyCode::ANSI_F,
        'G' => KeyCode::ANSI_G,
        'H' => KeyCode::ANSI_H,
        'I' => KeyCode::ANSI_I,
        'J' => KeyCode::ANSI_J,
        'K' => KeyCode::ANSI_K,
        'L' => KeyCode::ANSI_L,
        'M' => KeyCode::ANSI_M,
        'N' => KeyCode::ANSI_N,
        'O' => KeyCode::ANSI_O,
        'P' => KeyCode::ANSI_P,
        'Q' => KeyCode::ANSI_Q,
        'R' => KeyCode::ANSI_R,
        'S' => KeyCode::ANSI_S,
        'T' => KeyCode::ANSI_T,
        'U' => KeyCode::ANSI_U,
        'V' => KeyCode::ANSI_V,
        'W' => KeyCode::ANSI_W,
        'X' => KeyCode::ANSI_X,
        'Y' => KeyCode::ANSI_Y,
        'Z' => KeyCode::ANSI_Z,
        '0' => KeyCode::ANSI_0,
        '1' => KeyCode::ANSI_1,
        '2' => KeyCode::ANSI_2,
        '3' => KeyCode::ANSI_3,
        '4' => KeyCode::ANSI_4,
        '5' => KeyCode::ANSI_5,
        '6' => KeyCode::ANSI_6,
        '7' => KeyCode::ANSI_7,
        '8' => KeyCode::ANSI_8,
        '9' => KeyCode::ANSI_9,
        _ => return None,
    })
}

// ─────────────────────────── 角色 / 动作映射 ───────────────────────────

/// macOS AX role → 统一角色词表。
pub fn map_role(ax_role: &str) -> String {
    let r = ax_role.strip_prefix("AX").unwrap_or(ax_role);
    match r {
        "Button" => "button",
        "TextField" => "textbox",
        "TextArea" => "textbox",
        "StaticText" => "text",
        "Link" => "link",
        "CheckBox" => "checkbox",
        "RadioButton" => "radio",
        "PopUpButton" => "combobox",
        "ComboBox" => "combobox",
        "MenuItem" => "menuitem",
        "MenuBar" => "menubar",
        "Menu" => "menu",
        "Window" => "window",
        "Sheet" => "sheet",
        "Dialog" => "dialog",
        "Group" => "group",
        "ScrollArea" => "scrollarea",
        "List" => "list",
        "Row" => "row",
        "Outline" => "tree",
        "Table" => "table",
        "Image" => "image",
        "Heading" => "heading",
        "WebArea" => "webarea",
        "TabGroup" => "tabgroup",
        "Toolbar" => "toolbar",
        "Slider" => "slider",
        "DisclosureTriangle" => "disclosuretriangle",
        "Application" => "application",
        "Cell" => "cell",
        "Column" => "column",
        "ProgressIndicator" => "progressindicator",
        "ValueIndicator" => "valueindicator",
        "SplitGroup" => "splitgroup",
        "ScrollBar" => "scrollbar",
        "Incrementor" => "incrementor",
        other => return other.to_ascii_lowercase(),
    }
    .to_string()
}

/// macOS AX action → 统一动作名。
pub fn map_action(ax_action: &str) -> String {
    let a = ax_action.strip_prefix("AX").unwrap_or(ax_action);
    match a {
        "Press" => "click",
        "Increment" => "increment",
        "Decrement" => "decrement",
        "Confirm" => "confirm",
        "ShowMenu" => "show_menu",
        "Pick" => "pick",
        "Raise" => "raise",
        "ShowDefaultUI" => "invoke",
        "ShowAlternateUI" => "show_alternate",
        other => return other.to_ascii_lowercase(),
    }
    .to_string()
}

/// 解析 `desktop:<pid>:<path>`。
fn parse_desktop_ref(ref_: &str) -> Result<(i32, String)> {
    let parts: Vec<&str> = ref_.splitn(3, ':').collect();
    if parts.len() < 2 || parts[0] != "desktop" {
        return Err(EngineError::invalid(format!(
            "bad desktop ref '{ref_}' (expected desktop:<pid>:<path>)"
        )));
    }
    let pid: i32 = parts[1]
        .parse()
        .map_err(|_| EngineError::invalid(format!("bad pid in ref '{ref_}'")))?;
    let path = parts.get(2).copied().unwrap_or("").to_string();
    Ok((pid, path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::SurfaceProvider;

    #[test]
    fn role_mapping() {
        assert_eq!(map_role("AXButton"), "button");
        assert_eq!(map_role("AXTextField"), "textbox");
        assert_eq!(map_role("AXStaticText"), "text");
        assert_eq!(map_role("AXWindow"), "window");
        assert_eq!(map_role("AXApplication"), "application");
        assert_eq!(map_role("AXCustomThing"), "customthing");
    }

    #[test]
    fn action_mapping() {
        assert_eq!(map_action("AXPress"), "click");
        assert_eq!(map_action("AXIncrement"), "increment");
        assert_eq!(map_action("AXShowMenu"), "show_menu");
    }

    #[test]
    fn event_kind_mapping() {
        assert_eq!(
            event_kind_for("AXFocusedUIElementChanged"),
            SurfaceEventKind::FocusChanged
        );
        assert_eq!(
            event_kind_for("AXWindowCreated"),
            SurfaceEventKind::WindowCreated
        );
        assert_eq!(
            event_kind_for("AXValueChanged"),
            SurfaceEventKind::ValueChanged
        );
        assert_eq!(
            event_kind_for("AXTitleChanged"),
            SurfaceEventKind::TitleChanged
        );
        assert_eq!(
            event_kind_for("AXSelectedTextChanged"),
            SurfaceEventKind::SelectionChanged
        );
        assert_eq!(
            event_kind_for("AXUIElementDestroyed"),
            SurfaceEventKind::StructureChanged
        );
        assert_eq!(event_kind_for("Nope"), SurfaceEventKind::Unknown);
    }

    #[test]
    fn parse_ref_shapes() {
        assert_eq!(
            parse_desktop_ref("desktop:123:0.2.1").unwrap(),
            (123, "0.2.1".into())
        );
        assert_eq!(parse_desktop_ref("desktop:123").unwrap(), (123, "".into()));
        assert!(parse_desktop_ref("web:1:a").is_err());
    }

    #[test]
    fn keycode_mapping() {
        assert!(keycode_for("Enter").is_some());
        assert!(keycode_for("a").is_some());
        assert!(keycode_for("ArrowDown").is_some());
        assert!(keycode_for("NotAKey").is_none());
    }

    #[test]
    fn permission_probe_does_not_panic() {
        let _ = is_trusted();
    }

    /// 有权限时验证一次真实快照；无权限时验证明确报错（不失败）。
    #[tokio::test]
    async fn snapshot_or_permission_error() {
        let p = MacSurface::new();
        match p.snapshot("", SnapshotOptions::default()).await {
            Ok(snap) => {
                assert_eq!(snap.surface.kind, SurfaceKind::App);
                assert!(snap.root.is_some());
            }
            Err(e) => {
                assert_eq!(e.kind, ErrorKind::Unsupported);
                assert!(e.message.contains("accessibility permission"));
            }
        }
    }

    #[tokio::test]
    async fn list_surfaces_or_permission_error() {
        let p = MacSurface::new();
        match p.list_surfaces().await {
            Ok(v) => {
                // 有权限时至少应有聚焦应用（或空列表，若无前台应用）。
                for s in v {
                    assert!(s.id.starts_with("desktop:"));
                }
            }
            Err(e) => assert_eq!(e.kind, ErrorKind::Unsupported),
        }
    }

    /// 截图：有屏幕录制权限时返回有效位图；否则明确报错（不失败）。
    #[tokio::test]
    async fn screenshot_or_io_error() {
        let p = MacSurface::new();
        match p.screenshot("").await {
            Ok(img) => assert!(img.is_valid() && img.width > 0),
            Err(e) => assert_eq!(e.kind, ErrorKind::Io),
        }
    }
}
