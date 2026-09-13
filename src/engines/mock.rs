// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! 内存参考引擎（Mock）—— 无外部依赖，完整实现 `BrowserEngine`。
//!
//! 作用：
//! 1. CLI / 文档示例 / CI 中无需 Chromium 即可跑通内核全链路；
//! 2. 作为测试桩，验证工具集、会话、SDK、C ABI 的端到端行为；
//! 3. 提供可预期的脚本化页面模型（表单页 / 搜索页 / 通用页）。

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use crate::engine::host::{PageEventSink, ViewFrameSink};
use crate::engine::snapshot::{FrameSnapshot, ImageInfo, InteractiveElement, LinkInfo};
use crate::engine::{
    BrowserEngine, Cookie, DialogInfo, ElementRef, EngineCapabilities, EngineError, ErrorKind,
    EventNotifier, FrameStreamOptions, HistoryEntry, Image, InputEvent, KeyKind, MouseKind,
    PageEvent, PageSnapshot, Rect, RefKind, Result, TabId, TabInfo, TabOptions, ViewFrame,
    ViewHandle, Viewport,
};

// ────────────────────────────────────────────────────────────────
// 页面模型
// ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct MockElement {
    tag: String,
    role: Option<String>,
    text: Option<String>,
    href: Option<String>,
    rect: Rect,
    attrs: HashMap<String, String>,
    value: Option<String>,
    input_type: Option<String>,
    checked: bool,
    options: Vec<String>,
    selected: Option<String>,
    visible: bool,
    files: Vec<String>,
}

impl MockElement {
    fn new(tag: impl Into<String>) -> Self {
        MockElement {
            tag: tag.into(),
            role: None,
            text: None,
            href: None,
            rect: Rect::new(0.0, 0.0, 100.0, 30.0),
            attrs: HashMap::new(),
            value: None,
            input_type: None,
            checked: false,
            options: Vec::new(),
            selected: None,
            visible: true,
            files: Vec::new(),
        }
    }

    fn text(mut self, t: impl Into<String>) -> Self {
        self.text = Some(t.into());
        self
    }
    fn role(mut self, r: impl Into<String>) -> Self {
        self.role = Some(r.into());
        self
    }
    fn href(mut self, h: impl Into<String>) -> Self {
        self.href = Some(h.into());
        self
    }
    fn rect(mut self, x: f64, y: f64, w: f64, h: f64) -> Self {
        self.rect = Rect::new(x, y, w, h);
        self
    }
    fn input(mut self, ty: impl Into<String>) -> Self {
        self.input_type = Some(ty.into());
        self.value = Some(String::new());
        self
    }
    fn options(mut self, opts: Vec<&str>) -> Self {
        self.options = opts.iter().map(|s| s.to_string()).collect();
        self.selected = self.options.first().cloned();
        self
    }
    fn attr(mut self, k: impl Into<String>, v: impl Into<String>) -> Self {
        self.attrs.insert(k.into(), v.into());
        self
    }
}

struct Page {
    url: String,
    title: String,
    history: Vec<String>,
    history_idx: usize,
    elements: Vec<MockElement>,
    raw_text: String,
    table: Vec<Vec<String>>,
    loading: bool,
    viewport: Viewport,
    cookies: Vec<Cookie>,
    storage: HashMap<String, String>,
    css: Vec<String>,
    blocked: Vec<String>,
    intercepts: Vec<String>,
    events: VecDeque<PageEvent>,
    event_sink: Option<Arc<dyn PageEventSink>>,
    frame_sink: Option<Arc<dyn ViewFrameSink>>,
    frame_seq: u64,
    view_handle: Option<ViewHandle>,
    /// 帧推送流（若有）。
    frame_stream: Option<FrameStream>,
    /// 事件通知器（事件驱动等待用）：push 时唤醒等待者。
    notifier: Arc<EventNotifier>,
}

/// 帧推送流句柄（mock 引擎用轮询线程）。
struct FrameStream {
    running: Arc<AtomicBool>,
    join: Option<std::thread::JoinHandle<()>>,
}

// ────────────────────────────────────────────────────────────────
// 页面模板
// ────────────────────────────────────────────────────────────────

fn build_page(url: &str, viewport: Viewport) -> Page {
    let lower = url.to_ascii_lowercase();
    let (title, elements, raw_text, table) = if lower.contains("login") {
        login_page()
    } else if lower.contains("search") || lower.contains("google") {
        search_page()
    } else {
        generic_page()
    };

    let mut page = Page {
        url: url.to_string(),
        title: title.to_string(),
        history: vec![url.to_string()],
        history_idx: 0,
        elements,
        raw_text: raw_text.to_string(),
        table,
        loading: false,
        viewport,
        cookies: Vec::new(),
        storage: HashMap::new(),
        css: Vec::new(),
        blocked: Vec::new(),
        intercepts: Vec::new(),
        events: VecDeque::new(),
        event_sink: None,
        frame_sink: None,
        frame_seq: 0,
        view_handle: None,
        frame_stream: None,
        notifier: Arc::new(EventNotifier::new()),
    };

    page.events.push_back(PageEvent::NavigationStarted {
        url: url.to_string(),
    });
    page.events.push_back(PageEvent::NavigationCompleted {
        url: url.to_string(),
        status: 200,
    });
    page.events.push_back(PageEvent::Loaded {
        url: url.to_string(),
    });
    page.events.push_back(PageEvent::TitleChanged {
        title: title.to_string(),
    });
    page
}

fn generic_page() -> (
    &'static str,
    Vec<MockElement>,
    &'static str,
    Vec<Vec<String>>,
) {
    (
        "Example Page",
        vec![
            MockElement::new("h1").text("Welcome to FastBrowser").role("heading").rect(0.0, 0.0, 300.0, 40.0),
            MockElement::new("p").text("This is a demo page rendered by the in-memory engine.").rect(0.0, 50.0, 500.0, 40.0),
            MockElement::new("a").text("Learn more").href("https://example.com/about").role("link").rect(0.0, 100.0, 100.0, 30.0),
            MockElement::new("button").text("Click me").role("button").rect(0.0, 140.0, 100.0, 30.0),
            MockElement::new("input").input("text").attr("placeholder", "Search...").rect(0.0, 180.0, 200.0, 30.0),
            MockElement::new("img").attr("src", "https://example.com/logo.png").attr("alt", "logo").rect(0.0, 220.0, 50.0, 50.0),
        ],
        "Welcome to FastBrowser\nThis is a demo page rendered by the in-memory engine.\nLearn more\nClick me\n",
        vec![vec!["Name".into(), "Value".into()], vec!["alpha".into(), "1".into()], vec!["beta".into(), "2".into()]],
    )
}

fn login_page() -> (
    &'static str,
    Vec<MockElement>,
    &'static str,
    Vec<Vec<String>>,
) {
    (
        "Login",
        vec![
            MockElement::new("h1")
                .text("Sign in")
                .role("heading")
                .rect(0.0, 0.0, 200.0, 40.0),
            MockElement::new("input")
                .input("text")
                .attr("placeholder", "Username")
                .attr("name", "username")
                .rect(0.0, 50.0, 200.0, 30.0),
            MockElement::new("input")
                .input("password")
                .attr("placeholder", "Password")
                .attr("name", "password")
                .rect(0.0, 90.0, 200.0, 30.0),
            MockElement::new("input")
                .input("checkbox")
                .attr("name", "remember")
                .rect(0.0, 130.0, 20.0, 20.0),
            MockElement::new("select")
                .options(vec!["free", "pro", "team"])
                .rect(0.0, 160.0, 150.0, 30.0),
            MockElement::new("button")
                .text("Submit")
                .role("button")
                .rect(0.0, 200.0, 100.0, 30.0),
            MockElement::new("a")
                .text("Register")
                .href("https://example.com/register")
                .role("link")
                .rect(0.0, 240.0, 80.0, 30.0),
        ],
        "Sign in\n\n\n\nSubmit\nRegister\n",
        vec![],
    )
}

fn search_page() -> (
    &'static str,
    Vec<MockElement>,
    &'static str,
    Vec<Vec<String>>,
) {
    (
        "Search",
        vec![
            MockElement::new("input")
                .input("text")
                .attr("placeholder", "Query")
                .attr("name", "q")
                .rect(0.0, 0.0, 300.0, 30.0),
            MockElement::new("button")
                .text("Search")
                .role("button")
                .rect(310.0, 0.0, 80.0, 30.0),
            MockElement::new("a")
                .text("Result 1")
                .href("https://example.com/r1")
                .role("link")
                .rect(0.0, 50.0, 200.0, 30.0),
            MockElement::new("a")
                .text("Result 2")
                .href("https://example.com/r2")
                .role("link")
                .rect(0.0, 90.0, 200.0, 30.0),
            MockElement::new("a")
                .text("Result 3")
                .href("https://example.com/r3")
                .role("link")
                .rect(0.0, 130.0, 200.0, 30.0),
        ],
        "Result 1\nResult 2\nResult 3\n",
        vec![
            vec!["rank".into(), "title".into()],
            vec!["1".into(), "r1".into()],
            vec!["2".into(), "r2".into()],
            vec!["3".into(), "r3".into()],
        ],
    )
}

// ────────────────────────────────────────────────────────────────
// 引擎主体
// ────────────────────────────────────────────────────────────────

/// 内存参考引擎。
pub struct MockEngine {
    tabs: std::sync::RwLock<
        std::collections::HashMap<TabId, std::sync::Arc<std::sync::RwLock<Page>>>,
    >,
    active: std::sync::RwLock<Option<TabId>>,
    next_tab: std::sync::atomic::AtomicU32,
    next_handle: std::sync::atomic::AtomicU64,
    viewport: std::sync::RwLock<Viewport>,
    /// 引擎级全局事件通知器（异步事件泵唤醒用）。
    any: Arc<EventNotifier>,
}

impl MockEngine {
    pub fn new() -> Self {
        MockEngine {
            tabs: std::sync::RwLock::new(std::collections::HashMap::new()),
            active: std::sync::RwLock::new(None),
            next_tab: std::sync::atomic::AtomicU32::new(1),
            next_handle: std::sync::atomic::AtomicU64::new(1),
            viewport: std::sync::RwLock::new(Viewport::new(800, 600)),
            any: Arc::new(EventNotifier::new()),
        }
    }

    fn read_map(
        &self,
    ) -> std::sync::RwLockReadGuard<
        '_,
        std::collections::HashMap<TabId, std::sync::Arc<std::sync::RwLock<Page>>>,
    > {
        self.tabs.read().unwrap_or_else(|e| e.into_inner())
    }
    fn write_map(
        &self,
    ) -> std::sync::RwLockWriteGuard<
        '_,
        std::collections::HashMap<TabId, std::sync::Arc<std::sync::RwLock<Page>>>,
    > {
        self.tabs.write().unwrap_or_else(|e| e.into_inner())
    }

    /// 取出标签页的 Arc（短读锁，立即释放；随后按需 read/write 该标签页自身锁）。
    fn tab_arc(&self, tab: TabId) -> Result<std::sync::Arc<std::sync::RwLock<Page>>> {
        self.read_map()
            .get(&tab)
            .cloned()
            .ok_or_else(|| EngineError::tab_not_found(tab))
    }

    /// 对单个标签页做可变操作（per-tab 写锁；不同标签页互不阻塞 → 单实例并发多标签页）。
    /// 闭包须返回 `Result<R>`，错误会原样传播。
    fn with_tab<R>(&self, tab: TabId, f: impl FnOnce(&mut Page) -> Result<R>) -> Result<R> {
        let p = self.tab_arc(tab)?;
        let mut guard = p.write().unwrap_or_else(|e| e.into_inner());
        f(&mut guard)
    }

    /// 只读单个标签页。
    fn read_tab<R>(&self, tab: TabId, f: impl FnOnce(&Page) -> R) -> Result<R> {
        let p = self.tab_arc(tab)?;
        let guard = p.read().unwrap_or_else(|e| e.into_inner());
        Ok(f(&guard))
    }

    fn push(&self, tab: TabId, ev: PageEvent) {
        if let Ok(p) = self.tab_arc(tab) {
            let mut p = p.write().unwrap_or_else(|e| e.into_inner());
            if let Some(sink) = p.event_sink.clone() {
                sink.on_page_event(tab, &ev);
            }
            p.events.push_back(ev);
            while p.events.len() > crate::engine::MAX_BUFFERED_EVENTS {
                p.events.pop_front();
            }
            p.notifier.notify();
            self.any.notify();
        }
    }

    /// 解析元素引用为元素索引。
    fn resolve(&self, page: &Page, r: &ElementRef) -> Option<usize> {
        let visible: Vec<usize> = page
            .elements
            .iter()
            .enumerate()
            .filter(|(_, e)| e.visible)
            .map(|(i, _)| i)
            .collect();
        match r.kind {
            RefKind::Snapshot => {
                let id = r.value.chars().next()?;
                if !id.is_ascii_lowercase() {
                    return None;
                }
                visible.get((id as usize) - 'a' as usize).copied()
            }
            RefKind::Css => {
                let sel = r.value.trim();
                page.elements.iter().enumerate().find_map(|(i, e)| {
                    let sel_id = sel.strip_prefix('#');
                    let sel_cls = sel.strip_prefix('.');
                    if let Some(s) = sel_id {
                        if e.attrs.get("id").map(String::as_str) == Some(s) {
                            return Some(i);
                        }
                    }
                    if let Some(s) = sel_cls {
                        if e.attrs.get("class").map(String::as_str) == Some(s) {
                            return Some(i);
                        }
                    }
                    if !sel.starts_with(['#', '.']) && e.tag == sel {
                        return Some(i);
                    }
                    None
                })
            }
            RefKind::Xpath => {
                let expr = r.value.trim();
                page.elements.iter().enumerate().find_map(|(i, e)| {
                    let tag = expr.trim_start_matches('/').trim_start_matches('/');
                    if e.tag == tag {
                        Some(i)
                    } else {
                        None
                    }
                })
            }
            RefKind::Text => page.elements.iter().enumerate().find_map(|(i, e)| {
                if e.text.as_deref() == Some(r.value.as_str()) {
                    Some(i)
                } else {
                    None
                }
            }),
            RefKind::Role => page.elements.iter().enumerate().find_map(|(i, e)| {
                if e.role
                    .as_deref()
                    .map(|role| role.eq_ignore_ascii_case(r.value.as_str()))
                    .unwrap_or(false)
                {
                    Some(i)
                } else {
                    None
                }
            }),
        }
    }

    fn snapshot_of(&self, page: &Page) -> PageSnapshot {
        let mut id = 'a';
        let interactive: Vec<InteractiveElement> = page
            .elements
            .iter()
            .filter(|e| e.visible)
            .map(|e| {
                let this_id = id;
                id = ((id as u8) + 1) as char;
                let mut refs = vec![ElementRef::snapshot(this_id)];
                if let Some(id_attr) = e.attrs.get("id") {
                    refs.push(ElementRef::css(format!("#{id_attr}")));
                }
                if let Some(t) = &e.text {
                    refs.push(ElementRef::text(t.clone()));
                }
                refs.push(ElementRef::xpath(format!("//{}", e.tag)));
                InteractiveElement {
                    id: this_id,
                    tag: e.tag.clone(),
                    role: e.role.clone(),
                    text: e.text.clone(),
                    href: e.href.clone(),
                    rect: e.rect,
                    refs,
                    attrs: e.attrs.clone(),
                    value: e.value.clone(),
                    input_type: e.input_type.clone(),
                    checked: if e.input_type.as_deref() == Some("checkbox")
                        || e.input_type.as_deref() == Some("radio")
                    {
                        Some(e.checked)
                    } else {
                        None
                    },
                    selectable_options: if !e.options.is_empty() {
                        Some(e.options.clone())
                    } else {
                        None
                    },
                    selected_option: e.selected.clone(),
                    visible: e.visible,
                }
            })
            .collect();

        PageSnapshot {
            title: page.title.clone(),
            url: page.url.clone(),
            viewport: page.viewport,
            interactive,
            frames: Vec::<FrameSnapshot>::new(),
            timestamp_ms: now_ms(),
            meta: crate::engine::SnapshotMeta {
                total: page.elements.iter().filter(|e| e.visible).count(),
                truncated: false,
                viewport_h: page.viewport.height,
                scroll_h: page.viewport.height,
                scroll_y: 0,
                stale: false,
            },
        }
    }

    fn html_of(&self, page: &Page) -> String {
        let mut html = String::from("<!doctype html><html><head><title>");
        html.push_str(&page.title);
        html.push_str("</title></head><body>");
        for e in &page.elements {
            if !e.visible {
                continue;
            }
            match e.tag.as_str() {
                "input" => {
                    let ty = e.input_type.as_deref().unwrap_or("text");
                    let val = e.value.as_deref().unwrap_or("");
                    html.push_str(&format!(
                        "<input type=\"{ty}\" value=\"{val}\" placeholder=\"{}\"/>",
                        e.attrs.get("placeholder").map(String::as_str).unwrap_or("")
                    ));
                }
                "button" => html.push_str(&format!(
                    "<button>{}</button>",
                    e.text.as_deref().unwrap_or("")
                )),
                "a" => html.push_str(&format!(
                    "<a href=\"{}\">{}</a>",
                    e.href.as_deref().unwrap_or("#"),
                    e.text.as_deref().unwrap_or("")
                )),
                "select" => html.push_str("<select></select>"),
                "img" => html.push_str(&format!(
                    "<img src=\"{}\"/>",
                    e.attrs.get("src").map(String::as_str).unwrap_or("")
                )),
                other => html.push_str(&format!(
                    "<{other}>{}</{other}>",
                    e.text.as_deref().unwrap_or("")
                )),
            }
        }
        html.push_str("</body></html>");
        html
    }

    /// 记录导航：更新页面 + 追加导航事件（单一写锁内完成，避免嵌套锁死锁）。
    fn record_navigation(&self, tab: TabId, url: &str, push_history: bool) -> Result<()> {
        self.push(
            tab,
            PageEvent::NavigationStarted {
                url: url.to_string(),
            },
        );
        let p = self.tab_arc(tab)?;
        let mut page = p.write().unwrap_or_else(|e| e.into_inner());
        page.url = url.to_string();
        page.loading = true;
        let (title, elements, raw_text, table) = classify_url(url);
        page.title = title;
        page.elements = elements;
        page.raw_text = raw_text;
        page.table = table;
        page.loading = false;
        if push_history {
            let keep = page.history_idx + 1;
            page.history.truncate(keep);
            page.history.push(url.to_string());
            page.history_idx = page.history.len() - 1;
        }
        for ev in [
            PageEvent::NavigationCompleted {
                url: url.to_string(),
                status: 200,
            },
            PageEvent::Loaded {
                url: url.to_string(),
            },
            PageEvent::TitleChanged {
                title: page.title.clone(),
            },
        ] {
            if let Some(sink) = page.event_sink.clone() {
                sink.on_page_event(tab, &ev);
            }
            page.events.push_back(ev);
            while page.events.len() > crate::engine::MAX_BUFFERED_EVENTS {
                page.events.pop_front();
            }
        }
        page.notifier.notify();
        self.any.notify();
        Ok(())
    }
}

fn classify_url(url: &str) -> (String, Vec<MockElement>, String, Vec<Vec<String>>) {
    let lower = url.to_ascii_lowercase();
    if lower.contains("login") {
        let (t, e, r, tb) = login_page();
        (t.into(), e, r.into(), tb)
    } else if lower.contains("search") || lower.contains("google") {
        let (t, e, r, tb) = search_page();
        (t.into(), e, r.into(), tb)
    } else {
        let (t, e, r, tb) = generic_page();
        (t.into(), e, r.into(), tb)
    }
}

impl Default for MockEngine {
    fn default() -> Self {
        Self::new()
    }
}

// ────────────────────────────────────────────────────────────────
// BrowserEngine 实现
// ────────────────────────────────────────────────────────────────

impl BrowserEngine for MockEngine {
    fn name(&self) -> &'static str {
        "mock"
    }

    fn capabilities(&self) -> EngineCapabilities {
        EngineCapabilities::full()
    }

    fn cdp_endpoint(&self, _tab: TabId) -> Option<String> {
        None // 内存引擎无真实 CDP
    }

    fn create_tab(&self, url: &str, opts: &TabOptions) -> Result<TabId> {
        let viewport = opts
            .viewport
            .unwrap_or(*self.viewport.read().unwrap_or_else(|e| e.into_inner()));
        let mut page = build_page(url, viewport);
        let tab = TabId(self.next_tab.fetch_add(1, Ordering::SeqCst));
        let handle = self.next_handle.fetch_add(1, Ordering::SeqCst);
        page.view_handle = Some(ViewHandle::Native(handle));
        self.write_map()
            .insert(tab, std::sync::Arc::new(std::sync::RwLock::new(page)));
        self.push(tab, PageEvent::TabOpened { tab });
        if opts.active {
            *self.active.write().unwrap_or_else(|e| e.into_inner()) = Some(tab);
        }
        Ok(tab)
    }

    fn close_tab(&self, tab: TabId) -> Result<()> {
        {
            let mut tabs = self.write_map();
            if tabs.remove(&tab).is_none() {
                return Err(EngineError::tab_not_found(tab));
            }
        }
        let mut active = self.active.write().unwrap_or_else(|e| e.into_inner());
        if *active == Some(tab) {
            let first = self.read_map().keys().next().copied();
            *active = first;
        }
        Ok(())
    }

    fn list_tabs(&self) -> Vec<TabInfo> {
        let map = self.read_map();
        map.iter()
            .map(|(id, arc)| {
                let p = arc.read().unwrap_or_else(|e| e.into_inner());
                TabInfo {
                    id: *id,
                    url: p.url.clone(),
                    title: p.title.clone(),
                    loading: p.loading,
                    pinned: false,
                    created_ms: 0,
                    target_id: None,
                }
            })
            .collect()
    }

    fn switch_tab(&self, tab: TabId) -> Result<()> {
        if !self.read_map().contains_key(&tab) {
            return Err(EngineError::tab_not_found(tab));
        }
        *self.active.write().unwrap_or_else(|e| e.into_inner()) = Some(tab);
        Ok(())
    }

    fn active_tab(&self) -> Option<TabId> {
        *self.active.read().unwrap_or_else(|e| e.into_inner())
    }

    fn navigate(&self, tab: TabId, url: &str) -> Result<()> {
        if !self.read_map().contains_key(&tab) {
            return Err(EngineError::tab_not_found(tab));
        }
        self.record_navigation(tab, url, true)
    }

    fn back(&self, tab: TabId) -> Result<()> {
        let back_url = {
            let page = self.tab_arc(tab)?;
            let page = page.read().unwrap_or_else(|e| e.into_inner());
            if page.history_idx == 0 {
                return Ok(());
            }
            let idx = page.history_idx - 1;
            let u = page.history[idx].clone();
            drop(page);
            let p = self.tab_arc(tab)?;
            let mut page = p.write().unwrap_or_else(|e| e.into_inner());
            page.history_idx = idx;
            u
        };
        self.record_navigation(tab, &back_url, false)
    }

    fn forward(&self, tab: TabId) -> Result<()> {
        let fwd_url = {
            let page = self.tab_arc(tab)?;
            let page = page.read().unwrap_or_else(|e| e.into_inner());
            if page.history_idx + 1 >= page.history.len() {
                return Ok(());
            }
            let idx = page.history_idx + 1;
            let u = page.history[idx].clone();
            drop(page);
            let p = self.tab_arc(tab)?;
            let mut page = p.write().unwrap_or_else(|e| e.into_inner());
            page.history_idx = idx;
            u
        };
        self.record_navigation(tab, &fwd_url, false)
    }

    fn reload(&self, tab: TabId) -> Result<()> {
        let url = self.read_tab(tab, |p| p.url.clone())?;
        self.push(
            tab,
            PageEvent::Request {
                url: url.clone(),
                method: "GET".into(),
            },
        );
        self.push(
            tab,
            PageEvent::Response {
                url: url.clone(),
                status: 200,
            },
        );
        self.push(tab, PageEvent::Loaded { url });
        Ok(())
    }

    fn stop(&self, tab: TabId) -> Result<()> {
        self.with_tab(tab, |p| {
            p.loading = false;
            Ok(())
        })?;
        Ok(())
    }

    fn snapshot(&self, tab: TabId) -> Result<PageSnapshot> {
        self.read_tab(tab, |p| self.snapshot_of(p))
    }

    fn get_page_text(&self, tab: TabId) -> Result<String> {
        self.read_tab(tab, snapshot_text)
    }

    fn get_page_html(&self, tab: TabId) -> Result<String> {
        self.read_tab(tab, |p| self.html_of(p))
    }

    fn get_links(&self, tab: TabId) -> Result<Vec<LinkInfo>> {
        self.read_tab(tab, |p| {
            p.elements
                .iter()
                .filter(|e| e.tag == "a" && e.visible)
                .filter_map(|e| {
                    e.href.clone().map(|url| LinkInfo {
                        url,
                        text: e.text.clone().unwrap_or_default(),
                    })
                })
                .collect()
        })
    }

    fn get_images(&self, tab: TabId) -> Result<Vec<ImageInfo>> {
        self.read_tab(tab, |p| {
            p.elements
                .iter()
                .filter(|e| e.tag == "img" && e.visible)
                .map(|e| ImageInfo {
                    src: e.attrs.get("src").cloned().unwrap_or_default(),
                    alt: e.attrs.get("alt").cloned().unwrap_or_default(),
                    width: None,
                    height: None,
                })
                .collect()
        })
    }

    fn get_table(&self, tab: TabId) -> Result<Vec<Vec<String>>> {
        self.read_tab(tab, |p| p.table.clone())
    }

    fn execute_xpath(&self, tab: TabId, expr: &str) -> Result<Value> {
        self.read_tab(tab, |page| {
            let e = expr.trim();
            let (kind, sel) = if let Some(rest) = e.strip_prefix("count(") {
                let sel = rest
                    .trim_end_matches(')')
                    .trim_start_matches('/')
                    .trim_start_matches('/');
                let n = page
                    .elements
                    .iter()
                    .filter(|el| el.visible && el.tag == sel)
                    .count();
                return json!(n);
            } else {
                ("tag", e.trim_start_matches('/').trim_start_matches('/'))
            };
            let values: Vec<Value> = page
                .elements
                .iter()
                .filter(|el| el.visible)
                .filter_map(|el| {
                    if kind == "tag" && el.tag == sel {
                        Some(json!(el.text.clone().unwrap_or_default()))
                    } else {
                        None
                    }
                })
                .collect();
            json!(values)
        })
    }

    fn click_element(&self, tab: TabId, element: &ElementRef) -> Result<()> {
        let (tag, href) = self.with_tab(tab, |page| {
            let idx = self.resolve(page, element).ok_or_else(|| {
                EngineError::new(ErrorKind::Dom, format!("element {element:?} not found"))
            })?;
            let e = &mut page.elements[idx];
            let tag = e.tag.clone();
            let href = e.href.clone();
            let input_type = e.input_type.clone();
            if input_type.as_deref() == Some("checkbox") || input_type.as_deref() == Some("radio") {
                e.checked = !e.checked;
            }
            Ok::<_, EngineError>((tag, href))
        })?;
        if tag == "a" {
            if let Some(href) = href {
                return self.record_navigation(tab, &href, true);
            }
        }
        self.push(tab, PageEvent::DomChanged);
        Ok(())
    }

    fn set_element_value(&self, tab: TabId, element: &ElementRef, value: &str) -> Result<()> {
        self.with_tab(tab, |page| {
            let idx = self.resolve(page, element).ok_or_else(|| {
                EngineError::new(ErrorKind::Dom, format!("element {element:?} not found"))
            })?;
            page.elements[idx].value = Some(value.to_string());
            Ok(())
        })?;
        self.push(tab, PageEvent::DomChanged);
        Ok(())
    }

    /// 真实键盘输入语义：`clear=false` 时追加到已有值（模拟逐键输入）。
    fn type_text(&self, tab: TabId, element: &ElementRef, text: &str, clear: bool) -> Result<()> {
        self.with_tab(tab, |page| {
            let idx = self.resolve(page, element).ok_or_else(|| {
                EngineError::new(ErrorKind::Dom, format!("element {element:?} not found"))
            })?;
            if clear {
                page.elements[idx].value = Some(String::new());
            }
            let cur = page.elements[idx].value.clone().unwrap_or_default();
            page.elements[idx].value = Some(format!("{cur}{text}"));
            Ok(())
        })?;
        self.push(tab, PageEvent::DomChanged);
        Ok(())
    }

    fn select_option(&self, tab: TabId, element: &ElementRef, value: &str) -> Result<()> {
        self.with_tab(tab, |page| {
            let idx = self.resolve(page, element).ok_or_else(|| {
                EngineError::new(ErrorKind::Dom, format!("element {element:?} not found"))
            })?;
            if !page.elements[idx].options.contains(&value.to_string()) {
                return Err(EngineError::new(
                    ErrorKind::InvalidArgument,
                    format!("option '{value}' not in {:?}", page.elements[idx].options),
                ));
            }
            page.elements[idx].selected = Some(value.to_string());
            Ok(())
        })?;
        self.push(tab, PageEvent::DomChanged);
        Ok(())
    }

    fn check_element(&self, tab: TabId, element: &ElementRef, checked: bool) -> Result<()> {
        self.with_tab(tab, |page| {
            let idx = self.resolve(page, element).ok_or_else(|| {
                EngineError::new(ErrorKind::Dom, format!("element {element:?} not found"))
            })?;
            page.elements[idx].checked = checked;
            Ok(())
        })?;
        self.push(tab, PageEvent::DomChanged);
        Ok(())
    }

    fn set_file_input(&self, tab: TabId, element: &ElementRef, paths: &[String]) -> Result<()> {
        self.with_tab(tab, |page| {
            let idx = self.resolve(page, element).ok_or_else(|| {
                EngineError::new(ErrorKind::Dom, format!("element {element:?} not found"))
            })?;
            page.elements[idx].files = paths.to_vec();
            Ok(())
        })?;
        self.push(tab, PageEvent::DomChanged);
        Ok(())
    }

    fn inject_css(&self, tab: TabId, css: &str) -> Result<()> {
        self.with_tab(tab, |p| {
            p.css.push(css.to_string());
            Ok(())
        })?;
        Ok(())
    }

    fn inject_event(&self, tab: TabId, ev: InputEvent) -> Result<()> {
        match ev {
            InputEvent::Mouse(m) => {
                if m.kind == MouseKind::Click || m.kind == MouseKind::DoubleClick {
                    let hit: Option<ElementRef> = {
                        let page = self.tab_arc(tab)?;
                        let page = page.read().unwrap_or_else(|e| e.into_inner());
                        let visible: Vec<usize> = page
                            .elements
                            .iter()
                            .enumerate()
                            .filter(|(_, e)| e.visible && e.rect.contains(m.x, m.y))
                            .map(|(i, _)| i)
                            .collect();
                        visible.first().map(|i| {
                            let char_id = (b'a' + *i as u8) as char;
                            ElementRef {
                                kind: RefKind::Snapshot,
                                value: char_id.to_string(),
                            }
                        })
                    };
                    if let Some(r) = hit {
                        return self.click_element(tab, &r);
                    }
                }
                self.push(tab, PageEvent::DomChanged);
            }
            InputEvent::Key(k) => {
                if k.kind == KeyKind::Press {
                    self.with_tab(tab, |page| {
                        if let Some(i) = page
                            .elements
                            .iter()
                            .position(|e| e.input_type.as_deref() == Some("text") && e.visible)
                        {
                            let cur = page.elements[i].value.clone().unwrap_or_default();
                            page.elements[i].value = Some(format!("{cur}{}", k.text));
                        }
                        Ok(())
                    })?;
                    self.push(tab, PageEvent::DomChanged);
                }
            }
            InputEvent::Touch(t) => {
                let _ = t;
                self.push(tab, PageEvent::DomChanged);
            }
            InputEvent::Wheel(_) => {
                self.push(tab, PageEvent::DomChanged);
            }
        }
        Ok(())
    }

    fn evaluate(&self, tab: TabId, script: &str) -> Result<Value> {
        self.read_tab(tab, |page| {
            let mut js = MiniJs::new(page);
            js.run(script)
                .map_err(|e| EngineError::new(ErrorKind::Evaluate, format!("js error: {e}")))
        })?
    }

    fn page_title(&self, tab: TabId) -> Result<String> {
        self.read_tab(tab, |p| p.title.clone())
    }

    fn page_url(&self, tab: TabId) -> Result<String> {
        self.read_tab(tab, |p| p.url.clone())
    }

    fn set_viewport(&self, tab: TabId, vp: Viewport) -> Result<()> {
        *self.viewport.write().unwrap_or_else(|e| e.into_inner()) = vp;
        self.with_tab(tab, |p| {
            p.viewport = vp;
            Ok(())
        })?;
        Ok(())
    }

    fn screenshot(&self, tab: TabId) -> Result<Image> {
        self.read_tab(tab, mock_image)
    }

    fn view_handle(&self, tab: TabId) -> Option<ViewHandle> {
        let p = self.tab_arc(tab).ok()?;
        let guard = p.read().ok()?;
        guard.view_handle
    }

    fn view_frame(&self, tab: TabId) -> Result<ViewFrame> {
        let seq = self.with_tab(tab, |p| {
            p.frame_seq += 1;
            Ok(p.frame_seq)
        })?;
        let img = self.screenshot(tab)?;
        let frame = ViewFrame::from_image(&img, seq);
        if let Ok(p) = self.tab_arc(tab) {
            if let Some(sink) = p
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .frame_sink
                .clone()
            {
                sink.on_view_frame(tab, &frame);
            }
        }
        Ok(frame)
    }

    fn start_frame_stream(&self, tab: TabId, opts: FrameStreamOptions) -> Result<()> {
        self.stop_frame_stream(tab)?;
        let page_arc = self.tab_arc(tab)?;
        let running = Arc::new(AtomicBool::new(true));
        let run = running.clone();
        let sleep_ms = 1000u64 / u64::from(opts.fps.max(1));
        let join = std::thread::spawn(move || {
            while run.load(Ordering::Relaxed) {
                {
                    let mut page = page_arc.write().unwrap_or_else(|e| e.into_inner());
                    page.frame_seq += 1;
                    // 仅当流仍运行时推送，保证 stop 后帧数立即冻结
                    if run.load(Ordering::Relaxed) {
                        if let Some(sink) = page.frame_sink.clone() {
                            let img = mock_image(&page);
                            let frame = ViewFrame::from_image(&img, page.frame_seq);
                            sink.on_view_frame(tab, &frame);
                        }
                    }
                }
                std::thread::sleep(Duration::from_millis(sleep_ms));
            }
        });
        self.with_tab(tab, |p| {
            p.frame_stream = Some(FrameStream {
                running,
                join: Some(join),
            });
            Ok(())
        })?;
        Ok(())
    }

    fn stop_frame_stream(&self, tab: TabId) -> Result<()> {
        let join = {
            let p = self.tab_arc(tab)?;
            let mut page = p.write().unwrap_or_else(|e| e.into_inner());
            match page.frame_stream.take() {
                Some(fs) => {
                    fs.running.store(false, Ordering::Relaxed);
                    fs.join
                }
                None => None,
            }
        };
        if let Some(j) = join {
            let _ = j.join();
        }
        Ok(())
    }

    fn cookie_get(&self, tab: TabId, domain: Option<&str>) -> Result<Vec<Cookie>> {
        self.read_tab(tab, |p| match domain {
            Some(d) => p
                .cookies
                .iter()
                .filter(|c| c.domain == d)
                .cloned()
                .collect(),
            None => p.cookies.clone(),
        })
    }

    fn cookie_set(&self, tab: TabId, cookie: &Cookie) -> Result<()> {
        self.with_tab(tab, |p| {
            p.cookies
                .retain(|c| !(c.name == cookie.name && c.domain == cookie.domain));
            p.cookies.push(cookie.clone());
            Ok(())
        })?;
        Ok(())
    }

    fn cookie_clear(&self, tab: TabId, domain: Option<&str>, name: Option<&str>) -> Result<()> {
        self.with_tab(tab, |p| {
            p.cookies.retain(|c| {
                let d_match = domain.map(|d| c.domain == d).unwrap_or(true);
                let n_match = name.map(|n| c.name == n).unwrap_or(true);
                !(d_match && n_match)
            });
            Ok(())
        })?;
        Ok(())
    }

    fn storage_get(&self, tab: TabId, key: &str) -> Result<Option<String>> {
        self.read_tab(tab, |p| p.storage.get(key).cloned())
    }

    fn storage_set(&self, tab: TabId, key: &str, value: &str) -> Result<()> {
        self.with_tab(tab, |p| {
            p.storage.insert(key.to_string(), value.to_string());
            Ok(())
        })?;
        Ok(())
    }

    fn storage_all(&self, tab: TabId) -> Result<std::collections::HashMap<String, String>> {
        self.read_tab(tab, |p| p.storage.clone())
    }

    fn storage_clear(&self, tab: TabId) -> Result<()> {
        self.with_tab(tab, |p| {
            p.storage.clear();
            Ok(())
        })?;
        Ok(())
    }

    fn block_requests(&self, tab: TabId, patterns: &[String], enabled: bool) -> Result<()> {
        self.with_tab(tab, |p| {
            if enabled {
                for pat in patterns {
                    if !p.blocked.contains(pat) {
                        p.blocked.push(pat.clone());
                    }
                }
            } else {
                p.blocked.retain(|x| !patterns.contains(x));
            }
            Ok(())
        })?;
        Ok(())
    }

    fn intercept_requests(&self, tab: TabId, patterns: &[String], enabled: bool) -> Result<()> {
        self.with_tab(tab, |p| {
            if enabled {
                for pat in patterns {
                    if !p.intercepts.contains(pat) {
                        p.intercepts.push(pat.clone());
                    }
                }
            } else {
                p.intercepts.retain(|x| !patterns.contains(x));
            }
            Ok(())
        })?;
        Ok(())
    }

    fn drain_events(&self, tab: TabId) -> Vec<PageEvent> {
        self.tab_arc(tab)
            .map(|p| {
                p.write()
                    .unwrap_or_else(|e| e.into_inner())
                    .events
                    .drain(..)
                    .collect()
            })
            .unwrap_or_default()
    }

    fn event_generation(&self, tab: TabId) -> u64 {
        self.read_tab(tab, |p| p.notifier.generation()).unwrap_or(0)
    }

    fn wait_event(&self, tab: TabId, since: u64, timeout: Duration) -> (u64, bool) {
        // 短读锁克隆 Arc 后立即释放，避免持有标签页锁阻塞等待 → 与 push 的写锁死锁。
        let notifier = self.read_tab(tab, |p| p.notifier.clone());
        match notifier {
            Ok(n) => {
                let woke = n.wait_changed(since, timeout);
                (n.generation(), woke)
            }
            Err(_) => (since, false),
        }
    }

    fn any_event_generation(&self) -> u64 {
        self.any.generation()
    }

    fn wait_any_event(&self, since: u64, timeout: Duration) -> (u64, bool) {
        let woke = self.any.wait_changed(since, timeout);
        (self.any.generation(), woke)
    }

    fn set_event_sink(&self, tab: TabId, sink: Option<Arc<dyn PageEventSink>>) -> Result<()> {
        self.with_tab(tab, |p| {
            p.event_sink = sink;
            Ok(())
        })?;
        Ok(())
    }

    fn set_frame_sink(&self, tab: TabId, sink: Option<Arc<dyn ViewFrameSink>>) -> Result<()> {
        self.with_tab(tab, |p| {
            p.frame_sink = sink;
            Ok(())
        })?;
        Ok(())
    }

    // ── JS 对话框（mock 页面无真实弹窗，返回 None / 空操作）────────
    fn pending_dialog(&self, _tab: TabId) -> Option<DialogInfo> {
        None
    }
    fn dialog_accept(&self, _tab: TabId, _prompt_text: Option<&str>) -> Result<()> {
        Ok(())
    }
    fn dialog_dismiss(&self, _tab: TabId) -> Result<()> {
        Ok(())
    }

    // ── 元素动作变体 ───────────────────────────────────────────
    fn double_click_element(&self, tab: TabId, element: &ElementRef) -> Result<()> {
        self.click_element(tab, element)?;
        self.click_element(tab, element)
    }
    fn right_click_element(&self, tab: TabId, element: &ElementRef) -> Result<()> {
        self.click_element(tab, element)
    }

    // ── PDF 导出（返回最小合法 PDF 头，供工具/文件写入验证）────────
    fn print_to_pdf(&self, _tab: TabId) -> Result<Vec<u8>> {
        Ok(MINIMAL_PDF.to_vec())
    }

    // ── 导航历史 ──────────────────────────────────────────────
    fn get_history(&self, tab: TabId) -> Result<Vec<HistoryEntry>> {
        self.read_tab(tab, |page| {
            page.history
                .iter()
                .enumerate()
                .map(|(i, url)| HistoryEntry {
                    url: url.clone(),
                    title: page.title.clone(),
                    transition: if i == page.history_idx {
                        "current"
                    } else {
                        "link"
                    }
                    .to_string(),
                })
                .collect()
        })
    }

    // ── 无障碍树（从元素构建，ARIA 语义 role）──────────────────
    fn accessibility_tree(&self, tab: TabId) -> Result<Value> {
        self.read_tab(tab, |page| {
            let nodes: Vec<Value> = page
                .elements
                .iter()
                .filter(|e| e.visible)
                .map(|e| {
                    let role = e.role.clone().unwrap_or_else(|| match e.tag.as_str() {
                        "a" => "link".to_string(),
                        "button" => "button".to_string(),
                        "input" => match e.input_type.as_deref() {
                            Some("checkbox") => "checkbox".to_string(),
                            Some("radio") => "radio".to_string(),
                            _ => "textbox".to_string(),
                        },
                        "select" => "combobox".to_string(),
                        "textarea" => "textbox".to_string(),
                        "img" => "img".to_string(),
                        "h1" | "h2" | "h3" => "heading".to_string(),
                        other => other.to_string(),
                    });
                    json!({
                        "role": role,
                        "name": e.text.clone().unwrap_or_default(),
                        "value": e.value.clone().unwrap_or_default(),
                        "props": {
                            "checked": if e.input_type.as_deref() == Some("checkbox") || e.input_type.as_deref() == Some("radio") { json!(e.checked) } else { Value::Null },
                            "selected": e.selected.clone().map(|s| json!(s)).unwrap_or(Value::Null),
                        },
                    })
                })
                .collect();
            json!({ "tree": nodes, "count": nodes.len() })
        })
    }
}

/// 最小合法单页 PDF（mock 引擎 print_to_pdf 输出）。
const MINIMAL_PDF: &[u8] = b"%PDF-1.4\n\
1 0 obj\n\
<< /Type /Catalog /Pages 2 0 R >>\n\
endobj\n\
2 0 obj\n\
<< /Type /Pages /Kids [3 0 R] /Count 1 >>\n\
endobj\n\
3 0 obj\n\
<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>\n\
endobj\n\
xref\n\
0 4\n\
0000000000 65535 f \n\
0000000009 00000 n \n\
0000000058 00000 n \n\
0000000115 00000 n \n\
trailer\n\
<< /Size 4 /Root 1 0 R >>\n\
startxref\n\
186\n\
%%EOF\n";

/// 生成 mock 页面的位图（梯度伪画面）。
fn mock_image(page: &Page) -> Image {
    let (w, h) = (page.viewport.width as usize, page.viewport.height as usize);
    let seed = page.url.len();
    let mut rgba = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            let r = (x + seed) as u8;
            let g = (y + seed) as u8;
            let b = ((x + y + seed) % 255) as u8;
            rgba.push(r);
            rgba.push(g);
            rgba.push(b);
            rgba.push(255);
        }
    }
    Image::new(page.viewport.width, page.viewport.height, rgba)
}

fn snapshot_text(page: &Page) -> String {
    let mut parts = vec![page.raw_text.clone()];
    for e in &page.elements {
        if e.visible {
            if let Some(t) = &e.text {
                parts.push(t.clone());
            }
            if let Some(v) = &e.value {
                if !v.is_empty() {
                    parts.push(v.clone());
                }
            }
        }
    }
    parts.join(" ")
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ────────────────────────────────────────────────────────────────
// 迷你 JS 求值器（供 extract_json / execute_js 测试）
// ────────────────────────────────────────────────────────────────

struct MiniJs<'a> {
    page: &'a Page,
    toks: Vec<Tok>,
    pos: usize,
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Str(String),
    Ident(String),
    Op(String),
    LParen,
    RParen,
    LBracket,
    RBracket,
    Comma,
    Dot,
    LBrace,
    RBrace,
    Colon,
}

fn tokenize(s: &str) -> std::result::Result<Vec<Tok>, String> {
    let mut toks = Vec::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            ' ' | '\t' | '\n' | '\r' => i += 1,
            '(' => {
                toks.push(Tok::LParen);
                i += 1;
            }
            ')' => {
                toks.push(Tok::RParen);
                i += 1;
            }
            '[' => {
                toks.push(Tok::LBracket);
                i += 1;
            }
            ']' => {
                toks.push(Tok::RBracket);
                i += 1;
            }
            '{' => {
                toks.push(Tok::LBrace);
                i += 1;
            }
            '}' => {
                toks.push(Tok::RBrace);
                i += 1;
            }
            ',' => {
                toks.push(Tok::Comma);
                i += 1;
            }
            '.' => {
                toks.push(Tok::Dot);
                i += 1;
            }
            ':' => {
                toks.push(Tok::Colon);
                i += 1;
            }
            '\'' | '"' => {
                let quote = c;
                let mut buf = String::new();
                i += 1;
                let mut closed = false;
                while i < chars.len() {
                    if chars[i] == quote {
                        i += 1;
                        closed = true;
                        break;
                    }
                    buf.push(chars[i]);
                    i += 1;
                }
                if !closed {
                    return Err("unterminated string".into());
                }
                toks.push(Tok::Str(buf));
            }
            '0'..='9' => {
                let start = i;
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                    i += 1;
                }
                let txt: String = chars[start..i].iter().collect();
                let n: f64 = txt.parse().map_err(|_| format!("bad number {txt}"))?;
                toks.push(Tok::Num(n));
            }
            'a'..='z' | 'A'..='Z' | '_' => {
                let start = i;
                while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                toks.push(Tok::Ident(chars[start..i].iter().collect()));
            }
            '+' | '-' | '*' | '/' | '%' | '=' | '!' | '<' | '>' | '&' | '|' => {
                let start = i;
                while i < chars.len()
                    && matches!(
                        chars[i],
                        '+' | '-' | '*' | '/' | '%' | '=' | '!' | '<' | '>' | '&' | '|'
                    )
                {
                    i += 1;
                }
                toks.push(Tok::Op(chars[start..i].iter().collect()));
            }
            other => return Err(format!("unexpected char '{other}'")),
        }
    }
    Ok(toks)
}

impl<'a> MiniJs<'a> {
    fn new(page: &'a Page) -> Self {
        MiniJs {
            page,
            toks: Vec::new(),
            pos: 0,
        }
    }

    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }
    fn next(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.pos).cloned();
        if t.is_some() {
            self.pos += 1;
        }
        t
    }
    fn expect(&mut self, tok: Tok) -> std::result::Result<(), String> {
        match self.next() {
            Some(t) if t == tok => Ok(()),
            other => Err(format!("expected {tok:?}, got {other:?}")),
        }
    }

    fn run(&mut self, script: &str) -> std::result::Result<Value, String> {
        self.toks = tokenize(script)?;
        self.pos = 0;
        let v = self.parse_expr(0)?;
        Ok(v)
    }

    fn parse_expr(&mut self, min_prec: u8) -> std::result::Result<Value, String> {
        let mut lhs = self.parse_unary()?;
        loop {
            let prec = match self.peek() {
                Some(Tok::Op(op)) => op_prec(op),
                _ => None,
            };
            match prec {
                Some(p) if p >= min_prec => {
                    let op = match self.next() {
                        Some(Tok::Op(o)) => o,
                        _ => break,
                    };
                    let rhs = self.parse_expr(p + 1)?;
                    lhs = self.apply_bin(op, lhs, rhs)?;
                }
                _ => break,
            }
        }
        Ok(lhs)
    }

    fn parse_unary(&mut self) -> std::result::Result<Value, String> {
        let is_unary = matches!(self.peek(), Some(Tok::Op(op)) if op == "!" || op == "-");
        if is_unary {
            let op = match self.next() {
                Some(Tok::Op(o)) => o,
                _ => unreachable!(),
            };
            let v = self.parse_unary()?;
            return if op == "!" {
                Ok(Value::Bool(!truthy(&v)))
            } else {
                Ok(num(&v).map(|n| json!(-n)).unwrap_or(Value::Null))
            };
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> std::result::Result<Value, String> {
        match self.next() {
            Some(Tok::Num(n)) => self.postfix(json!(n)),
            Some(Tok::Str(s)) => self.postfix(json!(s)),
            Some(Tok::Ident(id)) if id == "true" => self.postfix(Value::Bool(true)),
            Some(Tok::Ident(id)) if id == "false" => self.postfix(Value::Bool(false)),
            Some(Tok::Ident(id)) if id == "null" => self.postfix(Value::Null),
            Some(Tok::LParen) => {
                let v = self.parse_expr(0)?;
                self.expect(Tok::RParen)?;
                self.postfix(v)
            }
            Some(Tok::LBracket) => {
                let mut arr = Vec::new();
                if let Some(Tok::RBracket) = self.peek() {
                    self.next();
                    return self.postfix(Value::Array(arr));
                }
                loop {
                    arr.push(self.parse_expr(0)?);
                    match self.next() {
                        Some(Tok::Comma) => continue,
                        Some(Tok::RBracket) => break,
                        other => return Err(format!("expected ',' or ']', got {other:?}")),
                    }
                }
                self.postfix(Value::Array(arr))
            }
            Some(Tok::LBrace) => {
                let mut map = serde_json::Map::new();
                loop {
                    let key = match self.next() {
                        Some(Tok::Str(k)) => k,
                        Some(Tok::Ident(k)) => k,
                        Some(Tok::RBrace) => break,
                        other => return Err(format!("bad object key {other:?}")),
                    };
                    self.expect(Tok::Colon)?;
                    let v = self.parse_expr(0)?;
                    map.insert(key, v);
                    match self.next() {
                        Some(Tok::Comma) => continue,
                        Some(Tok::RBrace) => break,
                        other => return Err(format!("expected ',' or '}}', got {other:?}")),
                    }
                }
                self.postfix(Value::Object(map))
            }
            Some(Tok::Ident(id)) => self.eval_ident(id),
            other => Err(format!("unexpected token {other:?}")),
        }
    }

    fn eval_ident(&mut self, id: String) -> std::result::Result<Value, String> {
        let base = match id.as_str() {
            "document" => {
                let mut map = serde_json::Map::new();
                map.insert("title".into(), json!(self.page.title));
                map.insert("readyState".into(), json!("complete"));
                map.insert(
                    "activeElement".into(),
                    json!({ "tagName": "BODY", "id": null, "text": "" }),
                );
                map.insert("body".into(), json!({}));
                Value::Object(map)
            }
            "location" => {
                let mut map = serde_json::Map::new();
                map.insert("href".into(), json!(self.page.url));
                Value::Object(map)
            }
            "window" => {
                let mut map = serde_json::Map::new();
                map.insert("innerWidth".into(), json!(self.page.viewport.width));
                map.insert("innerHeight".into(), json!(self.page.viewport.height));
                map.insert("scrollX".into(), json!(0));
                map.insert("scrollY".into(), json!(0));
                Value::Object(map)
            }
            _ => return Err(format!("unknown identifier '{id}'")),
        };
        self.postfix(base)
    }

    /// 统一的后缀链解析（`.` 成员/调用、`[]` 索引），对任意基础值生效。
    fn postfix(&mut self, mut base: Value) -> std::result::Result<Value, String> {
        loop {
            match self.peek() {
                Some(Tok::Dot) => {
                    self.next();
                    let key = match self.next() {
                        Some(Tok::Ident(k)) => k,
                        other => return Err(format!("expected member, got {other:?}")),
                    };
                    if let Some(Tok::LParen) = self.peek() {
                        self.next();
                        base = self.call_member(base, &key)?;
                    } else {
                        base = member(base, &key);
                    }
                }
                Some(Tok::LBracket) => {
                    self.next();
                    let idx = self.parse_expr(0)?;
                    self.expect(Tok::RBracket)?;
                    base = index(base, idx);
                }
                _ => break,
            }
        }
        Ok(base)
    }

    fn call_member(&mut self, base: Value, key: &str) -> std::result::Result<Value, String> {
        // 解析实参
        let mut args = Vec::new();
        if let Some(Tok::RParen) = self.peek() {
            self.next();
        } else {
            loop {
                args.push(self.parse_expr(0)?);
                match self.next() {
                    Some(Tok::Comma) => continue,
                    Some(Tok::RParen) => break,
                    other => return Err(format!("expected ',' or ')', got {other:?}")),
                }
            }
        }
        match (key, base) {
            ("querySelectorAll", Value::Object(_)) => {
                let sel = args.first().and_then(|v| v.as_str()).unwrap_or("");
                Ok(self.query_selector_all(sel))
            }
            ("textContent", Value::Object(_)) => Ok(json!(self.page.raw_text)),
            ("getSelection", Value::Object(_)) => Ok(json!({ "toString": true })),
            ("toString", Value::Object(_)) => Ok(json!("")),
            ("scrollTo", Value::Object(_)) => Ok(Value::Null),
            (other, _) => Err(format!("unsupported call .{other}()")),
        }
    }

    fn query_selector_all(&self, sel: &str) -> Value {
        let sel = sel.trim();
        let (tag, attr) = if let Some(rest) = sel.strip_prefix("a") {
            ("a", rest.trim().trim_start_matches(':').trim())
        } else if let Some(rest) = sel.strip_prefix("img") {
            ("img", rest.trim().trim_start_matches(':').trim())
        } else if let Some(rest) = sel.strip_prefix("button") {
            ("button", rest.trim().trim_start_matches(':').trim())
        } else if let Some(rest) = sel.strip_prefix("input") {
            ("input", rest.trim().trim_start_matches(':').trim())
        } else if let Some(rest) = sel.strip_prefix("li") {
            ("li", rest.trim().trim_start_matches(':').trim())
        } else {
            let (t, a) = sel.split_once('[').unwrap_or((sel, ""));
            (
                t,
                a.trim_end_matches(']')
                    .trim()
                    .trim_start_matches(':')
                    .trim(),
            )
        };
        let out: Vec<Value> = self
            .page
            .elements
            .iter()
            .filter(|e| e.visible && e.tag == tag)
            .filter(|e| attr.is_empty() || e_has_attr(e, attr))
            .map(|e| {
                let mut m = serde_json::Map::new();
                m.insert("tag".into(), json!(e.tag));
                if let Some(t) = &e.text {
                    m.insert("text".into(), json!(t));
                }
                if let Some(h) = &e.href {
                    m.insert("href".into(), json!(h));
                }
                if let Some(v) = &e.value {
                    m.insert("value".into(), json!(v));
                }
                if let Some(src) = e.attrs.get("src") {
                    m.insert("src".into(), json!(src));
                }
                if let Some(alt) = e.attrs.get("alt") {
                    m.insert("alt".into(), json!(alt));
                }
                Value::Object(m)
            })
            .collect();
        Value::Array(out)
    }

    fn apply_bin(&self, op: String, l: Value, r: Value) -> std::result::Result<Value, String> {
        match op.as_str() {
            "+" => {
                if let (Some(a), Some(b)) = (l.as_f64(), r.as_f64()) {
                    Ok(json!(a + b))
                } else if let (Some(a), Some(b)) = (l.as_str(), r.as_str()) {
                    Ok(json!(format!("{a}{b}")))
                } else {
                    Ok(l.clone())
                }
            }
            "-" => Ok(json!(num(&l).unwrap_or(0.0) - num(&r).unwrap_or(0.0))),
            "*" => Ok(json!(num(&l).unwrap_or(0.0) * num(&r).unwrap_or(0.0))),
            "/" => {
                let d = num(&r).unwrap_or(0.0);
                if d == 0.0 {
                    Err("division by zero".into())
                } else {
                    Ok(json!(num(&l).unwrap_or(0.0) / d))
                }
            }
            "%" => Ok(json!(num(&l).unwrap_or(0.0) % num(&r).unwrap_or(0.0))),
            "==" => Ok(json!(l == r)),
            "!=" => Ok(json!(l != r)),
            "<" => Ok(json!(ord_cmp(&l, &r, |a, b| a < b, |a, b| a < b))),
            "<=" => Ok(json!(ord_cmp(&l, &r, |a, b| a <= b, |a, b| a <= b))),
            ">" => Ok(json!(ord_cmp(&l, &r, |a, b| a > b, |a, b| a > b))),
            ">=" => Ok(json!(ord_cmp(&l, &r, |a, b| a >= b, |a, b| a >= b))),
            "&&" => Ok(if truthy(&l) { r } else { l }),
            "||" => Ok(if truthy(&l) { l } else { r }),
            other => Err(format!("unsupported operator '{other}'")),
        }
    }
}

fn op_prec(op: &str) -> Option<u8> {
    match op {
        "||" => Some(1),
        "&&" => Some(2),
        "==" | "!=" | "<" | "<=" | ">" | ">=" => Some(3),
        "+" | "-" => Some(4),
        "*" | "/" | "%" => Some(5),
        _ => None,
    }
}

fn num(v: &Value) -> Option<f64> {
    v.as_f64()
        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
}

/// 有序比较：数值按数值比较，字符串按字典序（JS 语义）。
fn ord_cmp(
    l: &Value,
    r: &Value,
    num_fn: impl FnOnce(f64, f64) -> bool,
    str_fn: impl FnOnce(&str, &str) -> bool,
) -> bool {
    match (l.as_str(), r.as_str()) {
        (Some(a), Some(b)) => str_fn(a, b),
        _ => num_fn(num(l).unwrap_or(0.0), num(r).unwrap_or(0.0)),
    }
}

fn truthy(v: &Value) -> bool {
    match v {
        Value::Null | Value::Bool(false) => false,
        Value::Number(n) => n.as_f64().map(|n| n != 0.0).unwrap_or(true),
        Value::String(s) => !s.is_empty(),
        _ => true,
    }
}

fn member(base: Value, key: &str) -> Value {
    match &base {
        Value::Array(a) if key == "length" => json!(a.len()),
        Value::String(s) if key == "length" => json!(s.len()),
        _ => base.get(key).cloned().unwrap_or(Value::Null),
    }
}

fn index(base: Value, idx: Value) -> Value {
    match (base, idx) {
        (Value::Array(a), Value::Number(n)) => {
            // MiniJs 数字为 f64，as_u64 对 f64 表示返回 None；统一用 as_f64 折算。
            let i = n
                .as_u64()
                .or_else(|| n.as_f64().map(|f| f as u64))
                .unwrap_or(0) as usize;
            a.get(i).cloned().unwrap_or(Value::Null)
        }
        (Value::Object(m), Value::String(k)) => m.get(&k).cloned().unwrap_or(Value::Null),
        _ => Value::Null,
    }
}

fn e_has_attr(e: &MockElement, attr: &str) -> bool {
    let attr = attr
        .trim_start_matches('@')
        .split('=')
        .next()
        .unwrap_or(attr);
    e.attrs.contains_key(attr)
}

// ────────────────────────────────────────────────────────────────
// 测试
// ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{MouseButton, MouseEvent};

    fn engine() -> MockEngine {
        MockEngine::new()
    }

    #[test]
    fn frame_stream_pushes_and_stops() {
        use std::sync::atomic::AtomicUsize;
        let e = engine();
        let tab = e
            .create_tab("https://example.com", &TabOptions::default())
            .unwrap();
        let count = Arc::new(AtomicUsize::new(0));
        struct Sink(Arc<AtomicUsize>);
        impl ViewFrameSink for Sink {
            fn on_view_frame(&self, _t: TabId, _f: &ViewFrame) {
                self.0.fetch_add(1, Ordering::Relaxed);
            }
        }
        let sink: Arc<dyn ViewFrameSink> = Arc::new(Sink(count.clone()));
        e.set_frame_sink(tab, Some(sink)).unwrap();
        e.start_frame_stream(
            tab,
            FrameStreamOptions {
                fps: 100,
                max_width: 0,
                max_height: 0,
                format: "png".into(),
            },
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(200));
        assert!(count.load(Ordering::Relaxed) >= 2, "expected >= 2 frames");
        e.stop_frame_stream(tab).unwrap();
        std::thread::sleep(Duration::from_millis(120));
        let a1 = count.load(Ordering::Relaxed);
        std::thread::sleep(Duration::from_millis(80));
        let a2 = count.load(Ordering::Relaxed);
        assert_eq!(
            a1, a2,
            "frames must stop after stop_frame_stream (a1={a1}, a2={a2})"
        );
    }

    #[test]
    fn type_text_appends_without_clear() {
        let e = engine();
        let tab = e
            .create_tab("https://example.com", &TabOptions::default())
            .unwrap();
        let input = ElementRef::snapshot('e');
        e.type_text(tab, &input, "ab", true).unwrap();
        e.type_text(tab, &input, "cd", false).unwrap();
        let snap = e.snapshot(tab).unwrap();
        assert_eq!(
            snap.element_by_id('e').unwrap().value.as_deref(),
            Some("abcd")
        );
    }

    /// 事件驱动等待：push 事件应唤醒 `wait_event`，且代数递增（无丢唤醒）。
    #[test]
    fn wait_event_wakes_on_push() {
        let e = Arc::new(engine());
        let tab = e
            .create_tab("https://example.com", &TabOptions::default())
            .unwrap();
        let gen0 = e.event_generation(tab);
        let e2 = e.clone();
        let h = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            let input = ElementRef::snapshot('e');
            e2.type_text(tab, &input, "x", false).unwrap();
        });
        let (gen1, woke) = e.wait_event(tab, gen0, Duration::from_secs(5));
        h.join().unwrap();
        assert!(woke, "wait_event must be woken by push (no lost wakeup)");
        assert_ne!(gen1, gen0, "generation must advance after a push");
    }

    /// 无事件时 `wait_event` 应阻塞满超时并返回 false（代数不变）。
    #[test]
    fn wait_event_times_out_without_push() {
        let e = engine();
        let tab = e
            .create_tab("https://example.com", &TabOptions::default())
            .unwrap();
        let gen0 = e.event_generation(tab);
        let start = std::time::Instant::now();
        let (gen1, woke) = e.wait_event(tab, gen0, Duration::from_millis(50));
        assert!(!woke);
        assert_eq!(gen1, gen0);
        assert!(start.elapsed() >= Duration::from_millis(50));
    }

    /// 全局事件通知器（异步事件泵唤醒用）：任意标签页 push 都应唤醒等待者。
    #[test]
    fn wait_any_event_wakes_on_any_tab_push() {
        let e = Arc::new(engine());
        let tab = e
            .create_tab("https://example.com", &TabOptions::default())
            .unwrap();
        let gen0 = e.any_event_generation();
        let e2 = e.clone();
        let h = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            let input = ElementRef::snapshot('e');
            e2.type_text(tab, &input, "x", false).unwrap();
        });
        let (gen1, woke) = e.wait_any_event(gen0, Duration::from_secs(5));
        h.join().unwrap();
        assert!(woke, "wait_any_event must be woken by any-tab push");
        assert_ne!(gen1, gen0, "global generation must advance after a push");
    }

    /// 无事件时全局等待应阻塞满超时并返回 false。
    #[test]
    fn wait_any_event_times_out_without_push() {
        let e = engine();
        let _tab = e
            .create_tab("https://example.com", &TabOptions::default())
            .unwrap();
        let gen0 = e.any_event_generation();
        let start = std::time::Instant::now();
        let (gen1, woke) = e.wait_any_event(gen0, Duration::from_millis(50));
        assert!(!woke);
        assert_eq!(gen1, gen0);
        assert!(start.elapsed() >= Duration::from_millis(50));
    }

    #[test]
    fn create_and_snapshot() {
        let e = engine();
        let tab = e
            .create_tab("https://example.com", &TabOptions::default())
            .unwrap();
        assert_eq!(e.active_tab(), Some(tab));
        let snap = e.snapshot(tab).unwrap();
        assert_eq!(snap.title, "Example Page");
        assert!(!snap.interactive.is_empty());
        assert_eq!(snap.interactive[0].id, 'a');
    }

    #[test]
    fn navigate_changes_page() {
        let e = engine();
        let tab = e
            .create_tab("https://example.com", &TabOptions::default())
            .unwrap();
        e.navigate(tab, "https://example.com/login").unwrap();
        let snap = e.snapshot(tab).unwrap();
        assert_eq!(snap.title, "Login");
        assert_eq!(snap.url, "https://example.com/login");
    }

    #[test]
    fn click_link_navigates() {
        let e = engine();
        let tab = e
            .create_tab("https://example.com", &TabOptions::default())
            .unwrap();
        let snap = e.snapshot(tab).unwrap();
        let link = snap
            .interactive
            .iter()
            .find(|el| el.href.is_some())
            .unwrap()
            .clone();
        e.click_element(tab, &ElementRef::snapshot(link.id))
            .unwrap();
        assert_eq!(e.snapshot(tab).unwrap().url, "https://example.com/about");
    }

    #[test]
    fn set_value_and_click_checkbox() {
        let e = engine();
        let tab = e
            .create_tab("https://example.com/login", &TabOptions::default())
            .unwrap();
        let snap = e.snapshot(tab).unwrap();
        let user = snap
            .interactive
            .iter()
            .find(|el| {
                el.attrs
                    .get("name")
                    .map(|s| s == "username")
                    .unwrap_or(false)
            })
            .unwrap()
            .clone();
        e.set_element_value(tab, &ElementRef::snapshot(user.id), "alice")
            .unwrap();
        let snap2 = e.snapshot(tab).unwrap();
        assert_eq!(
            snap2.element_by_id(user.id).unwrap().value.as_deref(),
            Some("alice")
        );
        let cb = snap2
            .interactive
            .iter()
            .find(|el| el.input_type.as_deref() == Some("checkbox"))
            .unwrap()
            .clone();
        e.click_element(tab, &ElementRef::snapshot(cb.id)).unwrap();
        let snap3 = e.snapshot(tab).unwrap();
        assert_eq!(snap3.element_by_id(cb.id).unwrap().checked, Some(true));
    }

    #[test]
    fn history_back_forward() {
        let e = engine();
        let tab = e
            .create_tab("https://example.com", &TabOptions::default())
            .unwrap();
        e.navigate(tab, "https://example.com/login").unwrap();
        e.back(tab).unwrap();
        assert_eq!(e.snapshot(tab).unwrap().url, "https://example.com");
        e.forward(tab).unwrap();
        assert_eq!(e.snapshot(tab).unwrap().url, "https://example.com/login");
    }

    #[test]
    fn coordinate_click_hits_element() {
        let e = engine();
        let tab = e
            .create_tab("https://example.com", &TabOptions::default())
            .unwrap();
        // "Click me" button rect at (0,140,100,30) center (50,155)
        e.inject_event(
            tab,
            MouseEvent::click(50.0, 155.0, MouseButton::Left).into(),
        )
        .unwrap();
        assert!(e
            .drain_events(tab)
            .iter()
            .any(|ev| matches!(ev, PageEvent::DomChanged)));
    }

    #[test]
    fn js_eval_support() {
        let e = engine();
        let tab = e
            .create_tab("https://example.com", &TabOptions::default())
            .unwrap();
        assert_eq!(e.evaluate(tab, "1+2*3").unwrap(), json!(7.0));
        assert_eq!(
            e.evaluate(tab, "document.title").unwrap(),
            json!("Example Page")
        );
        assert_eq!(
            e.evaluate(tab, "location.href").unwrap(),
            json!("https://example.com")
        );
        let links = e.evaluate(tab, "document.querySelectorAll('a')").unwrap();
        assert_eq!(links.as_array().unwrap().len(), 1);
        assert_eq!(e.evaluate(tab, "\"a\"+\"b\"").unwrap(), json!("ab"));
        assert_eq!(e.evaluate(tab, "2 > 1 && 3 < 4").unwrap(), json!(true));
    }

    #[test]
    fn js_eval_edge_cases() {
        let e = engine();
        let tab = e
            .create_tab("https://example.com", &TabOptions::default())
            .unwrap();
        // 运算符优先级与结合性
        assert_eq!(e.evaluate(tab, "2 + 3 * 4").unwrap(), json!(14.0));
        assert_eq!(e.evaluate(tab, "10 - 2 - 3").unwrap(), json!(5.0));
        assert_eq!(e.evaluate(tab, "20 / 5 % 3").unwrap(), json!(1.0));
        // 括号
        assert_eq!(e.evaluate(tab, "(1 + 2) * 3").unwrap(), json!(9.0));
        // 负数 / 取负
        assert_eq!(e.evaluate(tab, "-5").unwrap(), json!(-5.0));
        assert_eq!(e.evaluate(tab, "1 - -2").unwrap(), json!(3.0));
        // 逻辑短路按 JS 语义返回操作数
        assert_eq!(e.evaluate(tab, "1 && 2").unwrap(), json!(2.0));
        assert_eq!(e.evaluate(tab, "0 || 42").unwrap(), json!(42.0));
        assert_eq!(e.evaluate(tab, "!true").unwrap(), json!(false));
        // 比较
        assert_eq!(e.evaluate(tab, "5 >= 5").unwrap(), json!(true));
        assert_eq!(e.evaluate(tab, "\"abc\" < \"abd\"").unwrap(), json!(true));
        assert_eq!(e.evaluate(tab, "1 != 2").unwrap(), json!(true));
        // 字符串长度 / 数组长度
        assert_eq!(e.evaluate(tab, "\"abc\".length").unwrap(), json!(3));
        assert_eq!(e.evaluate(tab, "[10,20,30].length").unwrap(), json!(3));
        // 嵌套结构与索引
        assert_eq!(e.evaluate(tab, "[1,[2,3]][1][1]").unwrap(), json!(3.0));
        assert_eq!(
            e.evaluate(tab, "{\"a\":{\"b\":5}}.a.b").unwrap(),
            json!(5.0)
        );
        assert_eq!(e.evaluate(tab, "{\"a\":[7,8]}.a[1]").unwrap(), json!(8.0));
        // 字面量
        assert_eq!(e.evaluate(tab, "true").unwrap(), json!(true));
        assert_eq!(e.evaluate(tab, "null").unwrap(), json!(null));
        assert_eq!(e.evaluate(tab, "[]").unwrap(), json!([]));
        // 错误输入
        assert!(e.evaluate(tab, "").is_err());
        assert!(e.evaluate(tab, "1/0").is_err());
        assert!(e.evaluate(tab, "\"unterminated").is_err());
        assert!(e.evaluate(tab, "unknown_var").is_err());
        assert!(e.evaluate(tab, "@").is_err());
        assert!(e.evaluate(tab, "document.queryX()").is_err());
        // readyState（wait_for_load_state 依赖）
        assert_eq!(
            e.evaluate(tab, "document.readyState").unwrap(),
            json!("complete")
        );
    }

    #[test]
    fn history_edge_cases() {
        let e = engine();
        let tab = e
            .create_tab("https://example.com", &TabOptions::default())
            .unwrap();
        // 起始处 back 为 no-op
        e.back(tab).unwrap();
        assert_eq!(e.snapshot(tab).unwrap().url, "https://example.com");
        // 前进
        e.navigate(tab, "https://example.com/login").unwrap();
        e.navigate(tab, "https://example.com/search").unwrap();
        e.back(tab).unwrap();
        e.back(tab).unwrap();
        e.forward(tab).unwrap();
        e.forward(tab).unwrap();
        assert_eq!(e.snapshot(tab).unwrap().url, "https://example.com/search");
        // 末尾处 forward 为 no-op
        e.forward(tab).unwrap();
        assert_eq!(e.snapshot(tab).unwrap().url, "https://example.com/search");
        // 新导航截断前进历史
        e.back(tab).unwrap();
        e.navigate(tab, "https://example.com/register").unwrap();
        e.forward(tab).unwrap();
        assert_eq!(e.snapshot(tab).unwrap().url, "https://example.com/register");
    }

    #[test]
    fn tab_lifecycle_edge_cases() {
        let e = engine();
        // 关闭/切换不存在的标签 → 报错
        assert!(e.close_tab(TabId(999)).is_err());
        assert!(e.switch_tab(TabId(999)).is_err());
        // 无活动标签 → None
        assert_eq!(e.active_tab(), None);
        let t1 = e
            .create_tab("https://example.com", &TabOptions::default())
            .unwrap();
        let t2 = e
            .create_tab("https://example.com/search", &TabOptions::default())
            .unwrap();
        assert_eq!(e.active_tab(), Some(t2));
        // 关闭活动标签 → 切换到剩余标签
        e.close_tab(t2).unwrap();
        assert_eq!(e.active_tab(), Some(t1));
        assert_eq!(e.list_tabs().len(), 1);
        // 关闭全部
        e.close_tab(t1).unwrap();
        assert_eq!(e.list_tabs().len(), 0);
        assert_eq!(e.active_tab(), None);
        // 无标签时 snapshot 返回 TabNotFound
        assert!(e.snapshot(TabId(1)).is_err());
    }

    #[test]
    fn input_event_edge_cases() {
        let e = engine();
        let tab = e
            .create_tab("https://example.com", &TabOptions::default())
            .unwrap();
        // 点击空白处（无元素命中）不 panic
        e.inject_event(
            tab,
            MouseEvent::click(9999.0, 9999.0, MouseButton::Left).into(),
        )
        .unwrap();
        // 键盘输入到第一个文本输入框
        e.inject_event(tab, crate::engine::KeyEvent::press("z").into())
            .unwrap();
        let snap = e.snapshot(tab).unwrap();
        assert_eq!(snap.element_by_id('e').unwrap().value.as_deref(), Some("z"));
    }

    #[test]
    fn element_ops_on_non_inputs() {
        let e = engine();
        let tab = e
            .create_tab("https://example.com", &TabOptions::default())
            .unwrap();
        // 对 h1 设置 value（mock 不校验类型，不应 panic）
        e.set_element_value(tab, &ElementRef::snapshot('a'), "X")
            .unwrap();
        // 无效 option
        assert!(e
            .select_option(tab, &ElementRef::snapshot('e'), "nope")
            .is_err());
        // 元素引用解析失败
        assert!(e.click_element(tab, &ElementRef::css("#missing")).is_err());
        assert!(e
            .set_element_value(tab, &ElementRef::text("missing"), "v")
            .is_err());
    }

    #[test]
    fn storage_and_clear() {
        let e = engine();
        let tab = e
            .create_tab("https://example.com", &TabOptions::default())
            .unwrap();
        e.storage_set(tab, "a", "1").unwrap();
        e.storage_set(tab, "b", "2").unwrap();
        let all = e.storage_all(tab).unwrap();
        assert_eq!(all.len(), 2);
        e.storage_clear(tab).unwrap();
        assert!(e.storage_all(tab).unwrap().is_empty());
        assert_eq!(e.storage_get(tab, "a").unwrap(), None);
    }

    #[test]
    fn cookies_and_storage() {
        let e = engine();
        let tab = e
            .create_tab("https://example.com", &TabOptions::default())
            .unwrap();
        let c = Cookie::new("session", "abc", "example.com");
        e.cookie_set(tab, &c).unwrap();
        let got = e.cookie_get(tab, Some("example.com")).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].value, "abc");
        e.cookie_clear(tab, None, Some("session")).unwrap();
        assert!(e.cookie_get(tab, None).unwrap().is_empty());
        e.storage_set(tab, "k", "v").unwrap();
        assert_eq!(e.storage_get(tab, "k").unwrap().as_deref(), Some("v"));
    }

    #[test]
    fn screenshot_and_frame() {
        let e = engine();
        let tab = e
            .create_tab("https://example.com", &TabOptions::default())
            .unwrap();
        let img = e.screenshot(tab).unwrap();
        assert!(img.is_valid());
        let f1 = e.view_frame(tab).unwrap();
        let f2 = e.view_frame(tab).unwrap();
        assert!(f2.seq > f1.seq);
        assert!(e.view_handle(tab).is_some());
    }

    #[test]
    fn events_drained_and_sinked() {
        let e = engine();
        let tab = e
            .create_tab("https://example.com", &TabOptions::default())
            .unwrap();
        e.navigate(tab, "https://example.com/login").unwrap();
        let evs = e.drain_events(tab);
        assert!(evs
            .iter()
            .any(|ev| matches!(ev, PageEvent::NavigationCompleted { status: 200, .. })));
        assert!(e.drain_events(tab).is_empty());
    }

    #[test]
    fn block_and_intercept() {
        let e = engine();
        let tab = e
            .create_tab("https://example.com", &TabOptions::default())
            .unwrap();
        e.block_requests(tab, &["*ads*".to_string()], true).unwrap();
        e.intercept_requests(tab, &["*.png".to_string()], true)
            .unwrap();
        // 幂等
        e.block_requests(tab, &["*ads*".to_string()], true).unwrap();
        assert_eq!(
            e.read_tab(tab, |p| p.blocked.clone()).unwrap(),
            vec!["*ads*".to_string()]
        );
        assert_eq!(e.read_tab(tab, |p| p.intercepts.len()).unwrap(), 1);
    }
}
