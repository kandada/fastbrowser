// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! 移动端系统 WebView 引擎（feature `engine-webview`）。
//!
//! 设计要点（对齐调研结论）：
//! - Rust 侧只定义**桥协议 + 注入 JS**，真实 WebView 操作由宿主实现的
//!   `WebViewOps` 完成（Android=WebView / iOS=WKWebView / 鸿蒙=ArkWeb）。
//! - DOM 快照与元素操作通过**注入 JS**（`data-fb` 标注 + 按选择器操作）实现，
//!   因此无需坐标输入能力即可完成全部工具语义。
//! - Cookie/Storage 由内核维护 overlay（宿主可另行持久化）。

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

use crate::engine::host::{PageEventSink, ViewFrameSink, WebViewOps};
use crate::engine::snapshot::{FrameSnapshot, ImageInfo, InteractiveElement, LinkInfo};
use crate::engine::{
    BrowserEngine, Cookie, ElementRef, EngineCapabilities, EngineError, ErrorKind,
    FrameStreamOptions, Image, InputEvent, PageEvent, PageSnapshot, RefKind, Result, SnapshotMeta,
    TabId, TabInfo, TabOptions, ViewFrame, ViewHandle, Viewport,
};

/// 注入页面的 JS（共享实现见 `engine::inject`）。
pub use crate::engine::inject::{action_js, parse_snapshot_value, runtime_extract_js};

/// 标签页内部状态。
struct WvTab {
    handle: u64,
    url: String,
    title: String,
    elements: Vec<InteractiveElement>,
    frames: Vec<FrameSnapshot>,
    meta: SnapshotMeta,
    viewport: Viewport,
    events: VecDeque<PageEvent>,
    cookies: Vec<Cookie>,
    storage: HashMap<String, String>,
    frame_seq: u64,
    view_handle: ViewHandle,
    frame_sink: Option<Arc<dyn ViewFrameSink>>,
    frame_stream: Option<FrameStream>,
}

/// 帧推送流句柄（webview 用轮询截图线程）。
struct FrameStream {
    running: Arc<AtomicBool>,
    join: Option<std::thread::JoinHandle<()>>,
}

/// 移动端系统 WebView 引擎。
pub struct WebViewEngine {
    ops: Arc<dyn WebViewOps>,
    /// per-tab 状态：`Arc<RwLock<WvTab>>`，不同标签页操作互不阻塞。
    tabs: std::sync::RwLock<HashMap<TabId, Arc<std::sync::RwLock<WvTab>>>>,
    active: std::sync::RwLock<Option<TabId>>,
    next_tab: std::sync::atomic::AtomicU32,
    viewport: std::sync::RwLock<Viewport>,
}

impl WebViewEngine {
    pub fn new(ops: Arc<dyn WebViewOps>, config: &crate::config::Config) -> Result<Self> {
        Ok(WebViewEngine {
            ops,
            tabs: std::sync::RwLock::new(HashMap::new()),
            active: std::sync::RwLock::new(None),
            next_tab: std::sync::atomic::AtomicU32::new(1),
            viewport: std::sync::RwLock::new(config.viewport.unwrap_or(Viewport::new(390, 844))),
        })
    }

    fn tab_arc(&self, tab: TabId) -> Result<Arc<std::sync::RwLock<WvTab>>> {
        self.tabs
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(&tab)
            .cloned()
            .ok_or_else(|| EngineError::tab_not_found(tab))
    }

    fn with_tab<R>(&self, tab: TabId, f: impl FnOnce(&mut WvTab) -> Result<R>) -> Result<R> {
        let p = self.tab_arc(tab)?;
        let mut guard = p.write().unwrap_or_else(|e| e.into_inner());
        f(&mut guard)
    }

    fn read_tab<R>(&self, tab: TabId, f: impl FnOnce(&WvTab) -> R) -> Result<R> {
        let p = self.tab_arc(tab)?;
        let guard = p.read().unwrap_or_else(|e| e.into_inner());
        Ok(f(&guard))
    }

    fn push(&self, tab: TabId, ev: PageEvent) {
        if let Ok(p) = self.tab_arc(tab) {
            let mut t = p.write().unwrap_or_else(|e| e.into_inner());
            t.events.push_back(ev);
            // 有界缓冲：防 Agent 不 drain 时无限增长（保留最新）
            while t.events.len() > crate::engine::MAX_BUFFERED_EVENTS {
                t.events.pop_front();
            }
        }
    }

    /// 执行注入 JS 并解析 JSON 数组 → 缓存元素。
    fn refresh_snapshot(&self, tab: TabId) -> Result<()> {
        let handle = self.read_tab(tab, |t| t.handle)?;
        let raw = self.ops.evaluate_js(handle, runtime_extract_js())?;
        let (elements, meta, frames) = parse_snapshot_value(&raw)?;
        let title = self
            .ops
            .evaluate_js(handle, "document.title||''")
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_default();
        let url = self
            .ops
            .evaluate_js(handle, "location.href||''")
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_default();
        self.with_tab(tab, |t| {
            t.elements = elements;
            t.frames = frames;
            t.meta = meta;
            if !title.is_empty() {
                t.title = title;
            }
            if !url.is_empty() {
                t.url = url;
            }
            Ok(())
        })
    }

    /// 解析元素引用 → data-fb 编号。
    fn resolve_id(&self, tab: TabId, r: &ElementRef) -> Result<char> {
        self.read_tab(tab, |t| match r.kind {
            RefKind::Snapshot => r
                .value
                .chars()
                .next()
                .ok_or_else(|| EngineError::invalid("bad snapshot id")),
            _ => t
                .elements
                .iter()
                .find(|e| e.matches_ref(r))
                .map(|e| e.id)
                .ok_or_else(|| {
                    EngineError::new(ErrorKind::Dom, format!("element {r:?} not found"))
                }),
        })?
    }
}

impl BrowserEngine for WebViewEngine {
    fn name(&self) -> &'static str {
        "webview"
    }

    fn capabilities(&self) -> EngineCapabilities {
        EngineCapabilities::js_injection()
    }

    fn cdp_endpoint(&self, _tab: TabId) -> Option<String> {
        None // 系统 WebView 无 CDP
    }

    fn create_tab(&self, url: &str, opts: &TabOptions) -> Result<TabId> {
        let viewport = opts
            .viewport
            .unwrap_or(*self.viewport.read().unwrap_or_else(|e| e.into_inner()));
        let handle = self.ops.create_webview(url, viewport)?;
        let tab = TabId(
            self.next_tab
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst),
        );
        let view_handle = self
            .ops
            .native_view(handle)
            .unwrap_or(ViewHandle::Native(handle));
        let mut wv = WvTab {
            handle,
            url: url.to_string(),
            title: String::new(),
            elements: Vec::new(),
            frames: Vec::new(),
            meta: SnapshotMeta::default(),
            viewport,
            events: VecDeque::new(),
            cookies: Vec::new(),
            storage: HashMap::new(),
            frame_seq: 0,
            view_handle,
            frame_sink: None,
            frame_stream: None,
        };
        wv.events.push_back(PageEvent::NavigationStarted {
            url: url.to_string(),
        });
        wv.events.push_back(PageEvent::NavigationCompleted {
            url: url.to_string(),
            status: 200,
        });
        wv.events.push_back(PageEvent::Loaded {
            url: url.to_string(),
        });
        self.tabs
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(tab, Arc::new(std::sync::RwLock::new(wv)));
        let _ = self.refresh_snapshot(tab);
        self.push(tab, PageEvent::TabOpened { tab });
        if opts.active {
            *self.active.write().unwrap_or_else(|e| e.into_inner()) = Some(tab);
        }
        Ok(tab)
    }

    fn close_tab(&self, tab: TabId) -> Result<()> {
        let handle = self.read_tab(tab, |t| t.handle)?;
        let _ = self.ops.destroy_webview(handle);
        self.tabs
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&tab);
        let mut active = self.active.write().unwrap_or_else(|e| e.into_inner());
        if *active == Some(tab) {
            *active = self
                .tabs
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .keys()
                .next()
                .copied();
        }
        Ok(())
    }

    fn list_tabs(&self) -> Vec<TabInfo> {
        let map = self.tabs.read().unwrap_or_else(|e| e.into_inner());
        map.iter()
            .map(|(id, arc)| {
                let t = arc.read().unwrap_or_else(|e| e.into_inner());
                TabInfo {
                    id: *id,
                    url: t.url.clone(),
                    title: t.title.clone(),
                    loading: false,
                    pinned: false,
                    created_ms: 0,
                    target_id: None,
                }
            })
            .collect()
    }

    fn switch_tab(&self, tab: TabId) -> Result<()> {
        if !self
            .tabs
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(&tab)
        {
            return Err(EngineError::tab_not_found(tab));
        }
        *self.active.write().unwrap_or_else(|e| e.into_inner()) = Some(tab);
        Ok(())
    }

    fn active_tab(&self) -> Option<TabId> {
        *self.active.read().unwrap_or_else(|e| e.into_inner())
    }

    fn navigate(&self, tab: TabId, url: &str) -> Result<()> {
        let handle = self.read_tab(tab, |t| t.handle)?;
        self.push(
            tab,
            PageEvent::NavigationStarted {
                url: url.to_string(),
            },
        );
        self.ops.navigate_webview(handle, url)?;
        self.with_tab(tab, |t| {
            t.url = url.to_string();
            t.events.push_back(PageEvent::NavigationCompleted {
                url: url.to_string(),
                status: 200,
            });
            t.events.push_back(PageEvent::Loaded {
                url: url.to_string(),
            });
            Ok(())
        })?;
        Ok(())
    }

    fn back(&self, tab: TabId) -> Result<()> {
        self.ops.go_back(self.read_tab(tab, |t| t.handle)?)
    }

    fn forward(&self, tab: TabId) -> Result<()> {
        self.ops.go_forward(self.read_tab(tab, |t| t.handle)?)
    }

    fn reload(&self, tab: TabId) -> Result<()> {
        let handle = self.read_tab(tab, |t| t.handle)?;
        self.ops
            .navigate_webview(handle, &self.read_tab(tab, |t| t.url.clone())?)
    }

    fn stop(&self, _tab: TabId) -> Result<()> {
        Ok(())
    }

    fn snapshot(&self, tab: TabId) -> Result<PageSnapshot> {
        self.refresh_snapshot(tab)?;
        self.read_tab(tab, |t| PageSnapshot {
            title: t.title.clone(),
            url: t.url.clone(),
            viewport: t.viewport,
            interactive: t.elements.clone(),
            frames: t.frames.clone(),
            timestamp_ms: 0,
            meta: t.meta,
        })
    }

    fn get_page_text(&self, tab: TabId) -> Result<String> {
        let handle = self.read_tab(tab, |t| t.handle)?;
        let v = self
            .ops
            .evaluate_js(handle, "document.body?document.body.innerText:''")?;
        Ok(v.as_str().map(String::from).unwrap_or_default())
    }

    fn get_page_html(&self, tab: TabId) -> Result<String> {
        let handle = self.read_tab(tab, |t| t.handle)?;
        let v = self.ops.evaluate_js(
            handle,
            "document.documentElement?document.documentElement.outerHTML:''",
        )?;
        Ok(v.as_str().map(String::from).unwrap_or_default())
    }

    fn get_links(&self, tab: TabId) -> Result<Vec<LinkInfo>> {
        self.read_tab(tab, |t| {
            t.elements
                .iter()
                .filter(|e| e.tag == "a")
                .filter_map(|e| {
                    e.href.clone().map(|url| LinkInfo {
                        url,
                        text: e.text.clone().unwrap_or_default(),
                    })
                })
                .collect::<Vec<_>>()
        })
    }

    fn get_images(&self, tab: TabId) -> Result<Vec<ImageInfo>> {
        let handle = self.read_tab(tab, |t| t.handle)?;
        let v = self.ops.evaluate_js(
            handle,
            "Array.from(document.images).map(i=>({src:i.src,alt:i.alt}))",
        )?;
        let arr = v.as_array().cloned().unwrap_or_default();
        Ok(arr
            .iter()
            .map(|e| ImageInfo {
                src: e
                    .get("src")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                alt: e
                    .get("alt")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                width: None,
                height: None,
            })
            .collect())
    }

    fn get_table(&self, tab: TabId) -> Result<Vec<Vec<String>>> {
        let handle = self.read_tab(tab, |t| t.handle)?;
        let js = r#"(()=>{const t=document.querySelector('table');if(!t)return [];return Array.from(t.rows).map(r=>Array.from(r.cells).map(c=>c.innerText||''));})()"#;
        let v = self.ops.evaluate_js(handle, js)?;
        Ok(v.as_array()
            .map(|rows| {
                rows.iter()
                    .map(|r| {
                        r.as_array()
                            .map(|cells| {
                                cells
                                    .iter()
                                    .filter_map(|c| c.as_str().map(String::from))
                                    .collect()
                            })
                            .unwrap_or_default()
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    fn execute_xpath(&self, tab: TabId, expr: &str) -> Result<Value> {
        let handle = self.read_tab(tab, |t| t.handle)?;
        let js = format!(
            r#"(()=>{{const r=document.evaluate('{}',document,null,XPathResult.ORDERED_NODE_SNAPSHOT_TYPE,null);let out=[];for(let i=0;i<r.snapshotLength;i++){{out.push(r.snapshotItem(i).innerText||'');}}return out;}})()"#,
            expr.replace('\'', "\\'")
        );
        self.ops.evaluate_js(handle, &js)
    }

    fn click_element(&self, tab: TabId, element: &ElementRef) -> Result<()> {
        let handle = self.read_tab(tab, |t| t.handle)?;
        let id = self.resolve_id(tab, element)?;
        let js = action_js(
            id,
            r#"if(el.tagName==='A'&&el.getAttribute('href')){location.href=el.getAttribute('href');return 'navigating';}el.click()"#,
        );
        let v = self.ops.evaluate_js(handle, &js)?;
        if v.as_str() == Some("notfound") {
            return Err(EngineError::new(
                ErrorKind::Dom,
                "element not found in page",
            ));
        }
        self.push(tab, PageEvent::DomChanged);
        Ok(())
    }

    fn set_element_value(&self, tab: TabId, element: &ElementRef, value: &str) -> Result<()> {
        let handle = self.read_tab(tab, |t| t.handle)?;
        let id = self.resolve_id(tab, element)?;
        let val = serde_json::to_string(value).unwrap_or_else(|_| "\"\"".into());
        let js = action_js(
            id,
            &format!(
                r#"el.focus();el.value={val};el.dispatchEvent(new Event('input',{{bubbles:true}}));el.dispatchEvent(new Event('change',{{bubbles:true}}))"#
            ),
        );
        let v = self.ops.evaluate_js(handle, &js)?;
        if v.as_str() == Some("notfound") {
            return Err(EngineError::new(
                ErrorKind::Dom,
                "element not found in page",
            ));
        }
        self.push(tab, PageEvent::DomChanged);
        Ok(())
    }

    fn select_option(&self, tab: TabId, element: &ElementRef, value: &str) -> Result<()> {
        let handle = self.read_tab(tab, |t| t.handle)?;
        let id = self.resolve_id(tab, element)?;
        let val = serde_json::to_string(value).unwrap_or_else(|_| "\"\"".into());
        let js = action_js(
            id,
            &format!(r#"el.value={val};el.dispatchEvent(new Event('change',{{bubbles:true}}))"#),
        );
        let v = self.ops.evaluate_js(handle, &js)?;
        if v.as_str() == Some("notfound") {
            return Err(EngineError::new(
                ErrorKind::Dom,
                "element not found in page",
            ));
        }
        self.push(tab, PageEvent::DomChanged);
        Ok(())
    }

    fn check_element(&self, tab: TabId, element: &ElementRef, checked: bool) -> Result<()> {
        let handle = self.read_tab(tab, |t| t.handle)?;
        let id = self.resolve_id(tab, element)?;
        let js = action_js(
            id,
            &format!(
                r#"el.checked={checked};el.dispatchEvent(new Event('change',{{bubbles:true}}))"#
            ),
        );
        let v = self.ops.evaluate_js(handle, &js)?;
        if v.as_str() == Some("notfound") {
            return Err(EngineError::new(
                ErrorKind::Dom,
                "element not found in page",
            ));
        }
        self.push(tab, PageEvent::DomChanged);
        Ok(())
    }

    fn set_file_input(&self, _tab: TabId, _element: &ElementRef, _paths: &[String]) -> Result<()> {
        Err(EngineError::unsupported(
            "file input on mobile webview requires host support",
        ))
    }

    fn inject_css(&self, tab: TabId, css: &str) -> Result<()> {
        let handle = self.read_tab(tab, |t| t.handle)?;
        let css_js = serde_json::to_string(css).unwrap_or_else(|_| "\"\"".into());
        let js = format!("(()=>{{let s=document.createElement('style');s.textContent={css_js};document.head.appendChild(s);return 'ok';}})()");
        self.ops.evaluate_js(handle, &js)?;
        Ok(())
    }

    fn inject_event(&self, tab: TabId, ev: InputEvent) -> Result<()> {
        self.ops
            .dispatch_event(self.read_tab(tab, |t| t.handle)?, &ev)
    }

    fn evaluate(&self, tab: TabId, script: &str) -> Result<Value> {
        let handle = self.read_tab(tab, |t| t.handle)?;
        self.ops.evaluate_js(handle, script)
    }

    fn page_title(&self, tab: TabId) -> Result<String> {
        self.read_tab(tab, |t| t.title.clone())
    }

    fn page_url(&self, tab: TabId) -> Result<String> {
        self.read_tab(tab, |t| t.url.clone())
    }

    fn set_viewport(&self, tab: TabId, vp: Viewport) -> Result<()> {
        let handle = self.read_tab(tab, |t| t.handle)?;
        self.ops.set_viewport_webview(handle, vp)?;
        *self.viewport.write().unwrap_or_else(|e| e.into_inner()) = vp;
        self.with_tab(tab, |t| {
            t.viewport = vp;
            Ok(())
        })?;
        Ok(())
    }

    fn screenshot(&self, tab: TabId) -> Result<Image> {
        let handle = self.read_tab(tab, |t| t.handle)?;
        self.ops.screenshot_webview(handle)
    }

    fn view_handle(&self, tab: TabId) -> Option<ViewHandle> {
        self.read_tab(tab, |t| t.view_handle).ok()
    }

    fn view_frame(&self, tab: TabId) -> Result<ViewFrame> {
        let seq = self.with_tab(tab, |t| {
            t.frame_seq += 1;
            Ok(t.frame_seq)
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
        let tab_arc = self.tab_arc(tab)?;
        let ops = self.ops.clone();
        let running = Arc::new(AtomicBool::new(true));
        let run = running.clone();
        let sleep_ms = 1000u64 / u64::from(opts.fps.max(1));
        let join = std::thread::spawn(move || {
            while run.load(Ordering::Relaxed) {
                let handle = tab_arc.read().unwrap_or_else(|e| e.into_inner()).handle;
                if let Ok(img) = ops.screenshot_webview(handle) {
                    let mut t = tab_arc.write().unwrap_or_else(|e| e.into_inner());
                    t.frame_seq += 1;
                    if let Some(sink) = t.frame_sink.clone() {
                        let frame = ViewFrame::from_image(&img, t.frame_seq);
                        sink.on_view_frame(tab, &frame);
                    }
                }
                std::thread::sleep(Duration::from_millis(sleep_ms));
            }
        });
        self.with_tab(tab, |t| {
            t.frame_stream = Some(FrameStream {
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
            let mut t = p.write().unwrap_or_else(|e| e.into_inner());
            match t.frame_stream.take() {
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
        self.read_tab(tab, |t| match domain {
            Some(d) => t
                .cookies
                .iter()
                .filter(|c| c.domain == d)
                .cloned()
                .collect(),
            None => t.cookies.clone(),
        })
    }

    fn cookie_set(&self, tab: TabId, cookie: &Cookie) -> Result<()> {
        self.with_tab(tab, |t| {
            t.cookies
                .retain(|c| !(c.name == cookie.name && c.domain == cookie.domain));
            t.cookies.push(cookie.clone());
            Ok(())
        })
    }

    fn cookie_clear(&self, tab: TabId, domain: Option<&str>, name: Option<&str>) -> Result<()> {
        self.with_tab(tab, |t| {
            t.cookies.retain(|c| {
                let d_match = domain.map(|d| c.domain == d).unwrap_or(true);
                let n_match = name.map(|n| c.name == n).unwrap_or(true);
                !(d_match && n_match)
            });
            Ok(())
        })
    }

    fn storage_get(&self, tab: TabId, key: &str) -> Result<Option<String>> {
        self.read_tab(tab, |t| t.storage.get(key).cloned())
    }

    fn storage_set(&self, tab: TabId, key: &str, value: &str) -> Result<()> {
        self.with_tab(tab, |t| {
            t.storage.insert(key.to_string(), value.to_string());
            Ok(())
        })
    }

    fn storage_all(&self, tab: TabId) -> Result<HashMap<String, String>> {
        self.read_tab(tab, |t| t.storage.clone())
    }

    fn storage_clear(&self, tab: TabId) -> Result<()> {
        self.with_tab(tab, |t| {
            t.storage.clear();
            Ok(())
        })
    }

    fn block_requests(&self, _tab: TabId, _patterns: &[String], _enabled: bool) -> Result<()> {
        Err(EngineError::unsupported(
            "network control on mobile webview",
        ))
    }

    fn intercept_requests(&self, _tab: TabId, _patterns: &[String], _enabled: bool) -> Result<()> {
        Err(EngineError::unsupported(
            "network control on mobile webview",
        ))
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

    fn set_event_sink(&self, _tab: TabId, _sink: Option<Arc<dyn PageEventSink>>) -> Result<()> {
        Ok(())
    }

    fn set_frame_sink(&self, tab: TabId, sink: Option<Arc<dyn ViewFrameSink>>) -> Result<()> {
        self.with_tab(tab, |t| {
            t.frame_sink = sink;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::engine::host::NoopWebViewOps;
    use serde_json::json;

    /// 模拟宿主：返回固定的快照/文本，记录脚本。
    struct FakeOps {
        log: Arc<std::sync::Mutex<Vec<String>>>,
    }

    impl FakeOps {
        fn new() -> (Self, Arc<std::sync::Mutex<Vec<String>>>) {
            let log = Arc::new(std::sync::Mutex::new(Vec::new()));
            (FakeOps { log: log.clone() }, log)
        }
    }

    impl WebViewOps for FakeOps {
        fn create_webview(&self, _url: &str, _vp: Viewport) -> Result<u64> {
            Ok(1)
        }
        fn destroy_webview(&self, _handle: u64) -> Result<()> {
            Ok(())
        }
        fn evaluate_js(&self, _handle: u64, script: &str) -> Result<Value> {
            self.log.lock().unwrap().push(script.to_string());
            if script.starts_with("/*FB_EXTRACT_V3*/") {
                return Ok(json!(
                    r#"[{"id":"a","tag":"a","text":"Next","href":"https://example.com/next","rect":{"x":0,"y":0,"width":10,"height":10},"value":null,"input_type":null,"checked":null,"visible":true}]"#
                ));
            }
            if script.contains("document.title") {
                return Ok(json!("Fake Page"));
            }
            if script.contains("location.href") {
                return Ok(json!("https://fake.example"));
            }
            if script.contains("innerText") {
                return Ok(json!("Hello webview"));
            }
            if script.contains("outerHTML") {
                return Ok(json!("<html>...</html>"));
            }
            if script.contains("document.images") {
                return Ok(json!([{ "src": "x.png", "alt": "x" }]));
            }
            if script.contains("[data-fb") {
                return Ok(json!("ok"));
            }
            Ok(Value::Null)
        }
        fn navigate_webview(&self, _h: u64, _url: &str) -> Result<()> {
            Ok(())
        }
        fn screenshot_webview(&self, _h: u64) -> Result<Image> {
            Ok(Image::new(10, 10, vec![0u8; 400]))
        }
        fn set_viewport_webview(&self, _h: u64, _vp: Viewport) -> Result<()> {
            Ok(())
        }
        fn native_view(&self, h: u64) -> Result<ViewHandle> {
            Ok(ViewHandle::Native(h))
        }
        fn dispatch_event(&self, _h: u64, _ev: &InputEvent) -> Result<()> {
            Ok(())
        }
        fn go_back(&self, _h: u64) -> Result<()> {
            Ok(())
        }
        fn go_forward(&self, _h: u64) -> Result<()> {
            Ok(())
        }
    }

    fn engine() -> (WebViewEngine, Arc<std::sync::Mutex<Vec<String>>>) {
        let (ops, log) = FakeOps::new();
        let e = WebViewEngine::new(Arc::new(ops), &Config::for_engine("webview")).unwrap();
        (e, log)
    }

    #[test]
    fn snapshot_from_injected_js() {
        let (e, _) = engine();
        let tab = e
            .create_tab("https://fake.example", &TabOptions::default())
            .unwrap();
        let snap = e.snapshot(tab).unwrap();
        assert_eq!(snap.title, "Fake Page");
        assert_eq!(snap.url, "https://fake.example");
        assert_eq!(snap.interactive.len(), 1);
        assert_eq!(snap.interactive[0].id, 'a');
        assert_eq!(
            snap.interactive[0].href.as_deref(),
            Some("https://example.com/next")
        );
    }

    #[test]
    fn actions_generated_by_data_fb() {
        let (e, log) = engine();
        let tab = e
            .create_tab("https://fake.example", &TabOptions::default())
            .unwrap();
        e.click_element(tab, &ElementRef::snapshot('a')).unwrap();
        e.set_element_value(tab, &ElementRef::snapshot('a'), "hi")
            .unwrap();
        e.select_option(tab, &ElementRef::snapshot('a'), "v")
            .unwrap();
        e.check_element(tab, &ElementRef::snapshot('a'), true)
            .unwrap();
        let scripts = log.lock().unwrap();
        assert!(scripts.iter().any(|s| s.contains("el.click()")));
        assert!(scripts.iter().any(|s| s.contains("el.value=")));
        assert!(scripts.iter().any(|s| s.contains("el.checked=true")));
        assert!(scripts.iter().any(|s| s.contains("data-fb=\"a\"")));
    }

    #[test]
    fn text_html_images() {
        let (e, _) = engine();
        let tab = e
            .create_tab("https://fake.example", &TabOptions::default())
            .unwrap();
        assert_eq!(e.get_page_text(tab).unwrap(), "Hello webview");
        assert!(e.get_page_html(tab).unwrap().contains("<html>"));
        assert_eq!(e.get_images(tab).unwrap()[0].src, "x.png");
        assert!(e.get_links(tab).unwrap()[0].url.contains("/next"));
    }

    #[test]
    fn unsupported_operations() {
        let (e, _) = engine();
        let tab = e
            .create_tab("https://fake.example", &TabOptions::default())
            .unwrap();
        assert!(e.block_requests(tab, &["*".into()], true).is_err());
        assert!(e
            .set_file_input(tab, &ElementRef::snapshot('a'), &["/tmp/x".into()])
            .is_err());
        assert!(!e.capabilities().supports_coordinate_input);
        assert!(!e.capabilities().supports_osr);
    }

    #[test]
    fn noops_when_no_plugin() {
        let _ops = NoopWebViewOps;
        let _ = _ops.create_webview("x", Viewport::new(1, 1)).is_err();
    }
}
