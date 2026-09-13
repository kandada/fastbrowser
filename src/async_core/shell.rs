// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! 异步壳：`AsyncFastbrowser`。
//!
//! Browser-control entry point for async hosts (any async agent runtime).
//! 与同步壳共享同一个 `Fastbrowser` 实例：引擎操作经 `spawn_blocking` 调度到
//! tokio 阻塞池（async 执行器不阻塞）；事件推送与多标签编排是纯 tokio 异步。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::sync::broadcast;

use crate::async_core::orchestrate;
use crate::engine::{
    EngineError, ErrorKind, FrameStreamOptions, Image, PageEvent, PageSnapshot, Result, TabId,
};
use crate::sdk::Fastbrowser;

// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.
/// 异步壳。
pub struct AsyncFastbrowser {
    inner: Arc<Fastbrowser>,
    events: broadcast::Sender<(TabId, PageEvent)>,
    /// 事件监听线程：阻塞等待引擎事件并广播（无订阅者/无事件时零唤醒）。
    listener: Option<std::thread::JoinHandle<()>>,
    shutdown: Arc<AtomicBool>,
    /// 订阅者出现信号（条件变量）：无订阅者时监听线程阻塞于此。
    sub_wake: Arc<(Mutex<()>, Condvar)>,
}

impl AsyncFastbrowser {
    /// 在**当前 tokio runtime 内**创建异步壳。`inner` 可随后 `init`。
    /// 必须在 tokio 运行时上下文调用（否则 panic：无 runtime）。
    pub fn spawn(inner: Arc<Fastbrowser>) -> Self {
        let (tx, _rx) = broadcast::channel(4096);
        let shutdown = Arc::new(AtomicBool::new(false));
        let sub_wake = Arc::new((Mutex::new(()), Condvar::new()));
        let listener = {
            let tx = tx.clone();
            let inner = inner.clone();
            let shutdown = shutdown.clone();
            let sub_wake = sub_wake.clone();
            std::thread::spawn(move || event_listener(inner, tx, shutdown, sub_wake))
        };
        AsyncFastbrowser {
            inner,
            events: tx,
            listener: Some(listener),
            shutdown,
            sub_wake,
        }
    }

    /// 底层同步壳（同一浏览器实例）。
    pub fn inner(&self) -> &Arc<Fastbrowser> {
        &self.inner
    }

    /// 订阅页面事件流（`(TabId, PageEvent)`）。多订阅者互不影响。
    /// 订阅时唤醒监听线程（若有缓冲事件，可立即补发）。
    pub fn subscribe_events(&self) -> broadcast::Receiver<(TabId, PageEvent)> {
        let rx = self.events.subscribe();
        let (lock, cvar) = &*self.sub_wake;
        let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());
        cvar.notify_all();
        rx
    }

    /// 当前是否有事件订阅者（监听线程据此决定是否抽取事件）。
    pub fn has_event_subscribers(&self) -> bool {
        self.events.receiver_count() > 0
    }

    // ── 基础操作（spawn_blocking 到阻塞池，不阻塞 async executor）────────

    pub async fn init(&self, config: crate::config::Config) -> Result<Value> {
        let s = self.inner.clone();
        tokio::task::spawn_blocking(move || s.init(config).map(|_| json!({"ok": true})))
            .await
            .map_err(join_err)?
    }

    pub async fn open(&self, url: String) -> Result<Value> {
        let s = self.inner.clone();
        tokio::task::spawn_blocking(move || s.open(&url))
            .await
            .map_err(join_err)?
    }

    pub async fn navigate(&self, url: String) -> Result<Value> {
        let s = self.inner.clone();
        tokio::task::spawn_blocking(move || s.navigate(&url))
            .await
            .map_err(join_err)?
    }

    pub async fn tool_call(&self, name: String, params: Value) -> Result<Value> {
        let s = self.inner.clone();
        tokio::task::spawn_blocking(move || s.tool_call(&name, params))
            .await
            .map_err(join_err)?
    }

    /// 向当前聚焦元素插入文本（真实键盘通道；OSR 预览 UI 打字用）。
    pub async fn type_text_focused(&self, text: String) -> Result<Value> {
        let s = self.inner.clone();
        tokio::task::spawn_blocking(move || s.type_text_focused(&text))
            .await
            .map_err(join_err)?
    }

    /// 把当前活动标签页的窗口提到前台（有头模式下聚焦真实浏览器窗口）。
    pub async fn focus_window(&self) -> Result<Value> {
        let s = self.inner.clone();
        tokio::task::spawn_blocking(move || s.focus_window())
            .await
            .map_err(join_err)?
    }

    pub async fn snapshot(&self) -> Result<PageSnapshot> {
        let s = self.inner.clone();
        tokio::task::spawn_blocking(move || s.snapshot())
            .await
            .map_err(join_err)?
    }

    /// 对指定标签页生成快照（多标签场景，不依赖活动标签）。
    pub async fn snapshot_on(&self, tab: TabId) -> Result<PageSnapshot> {
        let s = self.inner.clone();
        tokio::task::spawn_blocking(move || {
            let rt = s.runtime()?;
            let rt = rt.as_ref().ok_or_else(EngineError::not_initialized)?;
            rt.engine().snapshot(tab)
        })
        .await
        .map_err(join_err)?
    }

    pub async fn screenshot(&self) -> Result<Image> {
        let s = self.inner.clone();
        tokio::task::spawn_blocking(move || s.screenshot())
            .await
            .map_err(join_err)?
    }

    /// 对指定标签页截图。
    pub async fn screenshot_on(&self, tab: TabId) -> Result<Image> {
        let s = self.inner.clone();
        tokio::task::spawn_blocking(move || {
            let rt = s.runtime()?;
            let rt = rt.as_ref().ok_or_else(EngineError::not_initialized)?;
            rt.engine().screenshot(tab)
        })
        .await
        .map_err(join_err)?
    }

    pub async fn set_viewport(&self, width: u32, height: u32) -> Result<()> {
        let s = self.inner.clone();
        tokio::task::spawn_blocking(move || s.set_viewport(width, height))
            .await
            .map_err(join_err)?
    }

    pub async fn session_save(&self, path: String) -> Result<Value> {
        let s = self.inner.clone();
        tokio::task::spawn_blocking(move || s.session_save(&path))
            .await
            .map_err(join_err)?
    }

    pub async fn session_load(&self, path: String) -> Result<Value> {
        let s = self.inner.clone();
        tokio::task::spawn_blocking(move || s.session_load(&path))
            .await
            .map_err(join_err)?
    }

    pub async fn clear_state(&self) -> Result<Value> {
        let s = self.inner.clone();
        tokio::task::spawn_blocking(move || s.clear_state())
            .await
            .map_err(join_err)?
    }

    pub async fn start_frame_stream(&self, tab: TabId, opts: FrameStreamOptions) -> Result<Value> {
        let s = self.inner.clone();
        tokio::task::spawn_blocking(move || s.start_frame_stream(tab, opts))
            .await
            .map_err(join_err)?
    }

    pub async fn stop_frame_stream(&self, tab: TabId) -> Result<Value> {
        let s = self.inner.clone();
        tokio::task::spawn_blocking(move || s.stop_frame_stream(tab))
            .await
            .map_err(join_err)?
    }

    /// 关闭内核（释放引擎）。
    pub async fn shutdown(&self) {
        let s = self.inner.clone();
        let _ = tokio::task::spawn_blocking(move || s.shutdown()).await;
    }

    // ── 轻量读取（短锁，直接返回）──────────────────────────────────────

    pub fn tool_list(&self) -> Value {
        self.inner.tool_list()
    }

    pub fn tool_count(&self) -> usize {
        self.inner.tool_count()
    }

    pub fn status(&self) -> Value {
        self.inner.status()
    }

    pub fn audit(&self) -> Value {
        self.inner.audit()
    }

    // ── 每标签页状态读取（异步）────────────────────────────────────────

    pub async fn page_url(&self, tab: TabId) -> Result<String> {
        let s = self.inner.clone();
        tokio::task::spawn_blocking(move || {
            let rt = s.runtime()?;
            let rt = rt.as_ref().ok_or_else(EngineError::not_initialized)?;
            rt.engine().page_url(tab)
        })
        .await
        .map_err(join_err)?
    }

    pub async fn page_title(&self, tab: TabId) -> Result<String> {
        let s = self.inner.clone();
        tokio::task::spawn_blocking(move || {
            let rt = s.runtime()?;
            let rt = rt.as_ref().ok_or_else(EngineError::not_initialized)?;
            rt.engine().page_title(tab)
        })
        .await
        .map_err(join_err)?
    }

    pub async fn drain_events(&self, tab: TabId) -> Vec<PageEvent> {
        let s = self.inner.clone();
        tokio::task::spawn_blocking(move || {
            s.runtime()
                .ok()
                .and_then(|rt| rt.as_ref().map(|r| r.engine().drain_events(tab)))
                .unwrap_or_default()
        })
        .await
        .unwrap_or_default()
    }

    // ── 多标签并发编排（tokio 原生）────────────────────────────────────

    /// 并发执行多个工具调用（不同标签页并行，同一标签页串行）。
    /// 示例：`vec![("click", {"id":"a","tab":1}), ("type", {"id":"e","text":"x","tab":2})]`
    pub async fn run_concurrently(&self, tasks: Vec<(String, Value)>) -> Result<Vec<Value>> {
        let n = tasks.len();
        let mut set = tokio::task::JoinSet::new();
        for (name, params) in tasks {
            let s = self.inner.clone();
            set.spawn(async move {
                tokio::task::spawn_blocking(move || s.tool_call(&name, params))
                    .await
                    .map_err(join_err)?
            });
        }
        let mut out = Vec::with_capacity(n);
        while let Some(res) = set.join_next().await {
            let value: Value =
                res.map_err(|e| EngineError::new(ErrorKind::Internal, format!("task join: {e}")))??;
            out.push(value);
        }
        Ok(out)
    }

    /// 并发打开多个 URL（每个一个新标签页）。
    pub async fn open_many(&self, urls: Vec<String>) -> Result<Vec<Value>> {
        let mut set = tokio::task::JoinSet::new();
        for url in urls {
            let s = self.inner.clone();
            set.spawn(async move {
                tokio::task::spawn_blocking(move || s.open(&url))
                    .await
                    .map_err(join_err)?
            });
        }
        let mut out = Vec::new();
        while let Some(res) = set.join_next().await {
            let value: Value =
                res.map_err(|e| EngineError::new(ErrorKind::Internal, format!("task join: {e}")))??;
            out.push(value);
        }
        Ok(out)
    }

    // ── 事件编排 ───────────────────────────────────────────────────────

    /// 等待某个（未来）事件。`pred` 返回 true 即返回该标签页。
    /// 先查已缓冲事件（覆盖订阅前错过的事件），再订阅未来事件。
    pub async fn wait_for_event(
        &self,
        tab: Option<TabId>,
        timeout: Duration,
        mut pred: impl FnMut(&PageEvent) -> bool,
    ) -> Result<TabId> {
        // 先查当前缓冲的（已发生）事件
        if let Some(t) = tab {
            for ev in self.drain_events(t).await {
                if pred(&ev) {
                    return Ok(t);
                }
            }
        }
        let mut rx = self.subscribe_events();
        orchestrate::wait_for_event(&mut rx, tab, timeout, pred).await
    }

    /// 任意一个目标标签页产生匹配事件即返回（tokio select）。
    /// 先查各标签页已缓冲事件，再订阅未来事件。
    pub async fn wait_any(
        &self,
        tabs: &[TabId],
        timeout: Duration,
        mut pred: impl FnMut(&PageEvent) -> bool,
    ) -> Result<TabId> {
        for t in tabs {
            for ev in self.drain_events(*t).await {
                if pred(&ev) {
                    return Ok(*t);
                }
            }
        }
        let mut rx = self.subscribe_events();
        orchestrate::wait_any(&mut rx, tabs, timeout, pred).await
    }

    /// 等待标签页完成一次导航（URL 变化或 NavigationCompleted 事件）。
    pub async fn wait_for_navigation(&self, tab: TabId, timeout: Duration) -> Result<()> {
        // 先查已缓冲事件（同步引擎 open/navigate 会立即推事件，订阅前错过）
        if self.drain_events(tab).await.iter().any(|e| {
            matches!(
                e,
                PageEvent::NavigationCompleted { .. } | PageEvent::Loaded { .. }
            )
        }) {
            return Ok(());
        }
        let start_url = self.page_url(tab).await.ok();
        let mut rx = self.subscribe_events();
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if let Ok(u) = self.page_url(tab).await {
                if Some(&u) != start_url.as_ref() && !u.is_empty() {
                    return Ok(());
                }
            }
            let remain = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remain.is_zero() {
                return Err(EngineError::new(
                    ErrorKind::Timeout,
                    format!("wait_for_navigation timed out after {timeout:?}"),
                ));
            }
            match tokio::time::timeout(Duration::from_millis(25), rx.recv()).await {
                Ok(Ok((t, ev))) if t == tab => {
                    if matches!(
                        ev,
                        PageEvent::NavigationCompleted { .. } | PageEvent::Loaded { .. }
                    ) {
                        return Ok(());
                    }
                }
                _ => {}
            }
        }
    }

    /// 等待页面 `document.readyState` 达到目标（load/interactive/complete）。
    pub async fn wait_for_load_state(
        &self,
        tab: TabId,
        state: &str,
        timeout: Duration,
    ) -> Result<()> {
        let want = state.to_string();
        let mut rx = self.subscribe_events();
        orchestrate::wait_until_event(
            &mut rx,
            tab,
            timeout,
            &format!("wait_for_load_state('{state}')"),
            || {
                let s = self.inner.clone();
                let want = want.clone();
                async move {
                    Ok(tokio::task::spawn_blocking(move || {
                        let rt = s.runtime().ok()?;
                        let rt = rt.as_ref()?;
                        rt.engine()
                            .evaluate(tab, "document.readyState||''")
                            .ok()
                            .and_then(|v| v.as_str().map(String::from))
                    })
                    .await
                    .ok()
                    .flatten()
                    .map(|ready| match want.as_str() {
                        "domcontentloaded" => ready == "interactive" || ready == "complete",
                        "networkidle" => ready == "complete",
                        _ => ready == "complete",
                    })
                    .unwrap_or(false))
                }
            },
        )
        .await
    }

    /// 等待 JS 谓词为真。
    pub async fn wait_for_condition(
        &self,
        tab: TabId,
        script: String,
        timeout: Duration,
    ) -> Result<()> {
        let mut rx = self.subscribe_events();
        orchestrate::wait_until_event(&mut rx, tab, timeout, "wait_for_condition", || {
            let s = self.inner.clone();
            let script = script.clone();
            async move {
                Ok(tokio::task::spawn_blocking(move || {
                    let rt = s.runtime().ok()?;
                    let rt = rt.as_ref()?;
                    rt.engine().evaluate(tab, &script).ok()
                })
                .await
                .ok()
                .flatten()
                .map(|v| match v {
                    Value::Bool(b) => b,
                    Value::Null => false,
                    Value::Number(n) => n.as_f64() != Some(0.0),
                    Value::String(s) => !s.is_empty(),
                    _ => true,
                })
                .unwrap_or(false))
            }
        })
        .await
    }
}

impl Drop for AsyncFastbrowser {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        // 唤醒可能阻塞在"等待订阅者"上的监听线程
        let (lock, cvar) = &*self.sub_wake;
        let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());
        cvar.notify_all();
        drop(_guard);
        if let Some(h) = self.listener.take() {
            let _ = h.join();
        }
    }
}

/// 事件监听线程（事件驱动，无轮询）：把每标签页的页面事件推送到广播频道。
///
/// - **有订阅者**：阻塞等待引擎全局事件（`wait_any_event`），事件到来立即醒来
///   抽取并广播；空闲时零唤醒。
/// - **无订阅者**：不抽取事件（避免与同步壳 `drain_events` 争抢），阻塞在订阅
///   信号上，`subscribe_events` 会唤醒它。
fn event_listener(
    inner: Arc<Fastbrowser>,
    tx: broadcast::Sender<(TabId, PageEvent)>,
    shutdown: Arc<AtomicBool>,
    sub_wake: Arc<(Mutex<()>, Condvar)>,
) {
    let mut gen = 0u64;
    loop {
        if shutdown.load(Ordering::Relaxed) {
            return;
        }
        if tx.receiver_count() == 0 {
            // 无订阅者：阻塞等待订阅者出现（最长 1s 兜底复查 shutdown）
            let (lock, cvar) = &*sub_wake;
            let guard = lock.lock().unwrap_or_else(|e| e.into_inner());
            let _ = cvar
                .wait_timeout(guard, Duration::from_secs(1))
                .unwrap_or_else(|e| e.into_inner());
            continue;
        }
        // 取运行时读锁（引擎可能尚未 init，短暂重试）
        let rt = match inner.runtime() {
            Ok(rt) => rt,
            Err(_) => {
                std::thread::sleep(Duration::from_millis(10));
                continue;
            }
        };
        let Some(rt) = rt.as_ref() else {
            std::thread::sleep(Duration::from_millis(10));
            continue;
        };
        let engine = rt.engine();
        // 阻塞等待任意标签页事件（事件到来立即唤醒；50ms 兜底复查 shutdown/订阅变化）
        let (new_gen, _woke) = engine.wait_any_event(gen, Duration::from_millis(50));
        gen = new_gen;
        let tabs = engine.list_tabs();
        for t in tabs {
            for ev in engine.drain_events(t.id) {
                let _ = tx.send((t.id, ev));
            }
        }
    }
}

fn join_err(e: tokio::task::JoinError) -> EngineError {
    EngineError::new(ErrorKind::Internal, format!("blocking task join: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn mock_shell() -> AsyncFastbrowser {
        let fb = Arc::new(Fastbrowser::new());
        fb.init(Config::default()).unwrap();
        AsyncFastbrowser::spawn(fb)
    }

    #[tokio::test]
    async fn async_open_snapshot_tool() {
        let s = mock_shell();
        let out = s.open("https://example.com".into()).await.unwrap();
        assert_eq!(out["title"], "Example Page");
        let snap = s.snapshot().await.unwrap();
        assert!(!snap.interactive.is_empty());
        let v = s
            .tool_call("get_current_url".into(), json!({}))
            .await
            .unwrap();
        assert_eq!(v["url"], "https://example.com");
    }

    #[tokio::test]
    async fn async_event_push_and_wait() {
        let s = mock_shell();
        let tab = s.open("https://example.com/login".into()).await.unwrap()["tab"]
            .as_u64()
            .unwrap() as u32;
        let tab = TabId(tab);
        // mock navigate 产生 NavigationCompleted 事件 → 泵推送到频道
        let got = s.wait_for_navigation(tab, Duration::from_secs(2)).await;
        assert!(got.is_ok(), "navigation event should be pushed: {got:?}");
        // 订阅后触发一次导航，验证事件推送
        s.navigate("https://example.com/search".into())
            .await
            .unwrap();
        let got = s
            .wait_for_event(Some(tab), Duration::from_secs(2), |e| {
                matches!(e, PageEvent::NavigationCompleted { .. })
            })
            .await
            .unwrap();
        assert_eq!(got, tab);
    }

    #[tokio::test]
    async fn async_concurrent_multi_tab() {
        let s = mock_shell();
        // 开两个标签
        let t1 = s.open("https://example.com".into()).await.unwrap()["tab"]
            .as_u64()
            .unwrap() as u32;
        let t2 = s.open("https://example.com/login".into()).await.unwrap()["tab"]
            .as_u64()
            .unwrap() as u32;
        // 并发：在 tab1 输入、tab2 填表
        let results = s
            .run_concurrently(vec![
                (
                    "type".into(),
                    json!({"id": "e", "text": "hello", "tab": t1}),
                ),
                (
                    "fill_form".into(),
                    json!({"values": {"a": "alice"}, "tab": t2}),
                ),
            ])
            .await
            .unwrap();
        assert_eq!(results.len(), 2);
        // tab1 输入生效
        let v = s
            .tool_call("get_element_info".into(), json!({"id": "e", "tab": t1}))
            .await
            .unwrap();
        assert_eq!(v["value"], "hello");
        // tab2 填表生效
        let v2 = s
            .tool_call("get_element_info".into(), json!({"id": "a", "tab": t2}))
            .await
            .unwrap();
        assert_eq!(v2["value"], "alice");
    }

    #[tokio::test]
    async fn async_open_many() {
        let s = mock_shell();
        let out = s
            .open_many(vec![
                "https://example.com".into(),
                "https://example.com/login".into(),
                "https://example.com/search".into(),
            ])
            .await
            .unwrap();
        assert_eq!(out.len(), 3);
        assert_eq!(
            s.tool_call("list_tabs".into(), json!({})).await.unwrap()["tabs"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
    }

    #[tokio::test]
    async fn async_wait_for_load_state_and_condition() {
        let s = mock_shell();
        s.open("https://example.com".into()).await.unwrap();
        s.wait_for_load_state(TabId(1), "load", Duration::from_secs(2))
            .await
            .unwrap();
        s.wait_for_condition(
            TabId(1),
            "document.querySelectorAll('a').length >= 1".into(),
            Duration::from_secs(2),
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn async_wait_any_over_tabs() {
        let s = mock_shell();
        let t1 = TabId(1);
        let t2 = TabId(2);
        s.open("https://example.com".into()).await.unwrap(); // t1
        s.open("https://example.com/login".into()).await.unwrap(); // t2
                                                                   // 两个 tab 都已产生 NavigationCompleted；wait_any 命中其一
        let got = s
            .wait_any(&[t1, t2], Duration::from_secs(2), |e| {
                matches!(e, PageEvent::NavigationCompleted { .. })
            })
            .await;
        let got = got.unwrap();
        assert!(got == t1 || got == t2);
    }

    // ── 事件泵语义 ───────────────────────────────────────────────────

    #[tokio::test]
    async fn async_event_pump_respects_subscribers() {
        let s = mock_shell();
        let tab = s.open("https://example.com".into()).await.unwrap()["tab"]
            .as_u64()
            .unwrap() as u32;
        let tab = TabId(tab);
        // 无订阅者：泵不抽取，同步 drain_events 仍能看到事件
        s.navigate("https://example.com/search".into())
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(60)).await; // 让泵有机会跑一轮
        let evs = s.drain_events(tab).await;
        assert!(
            !evs.is_empty(),
            "sync drain must still see events when no async subscriber"
        );
    }

    #[tokio::test]
    async fn async_event_multi_subscribers_all_receive() {
        let s = mock_shell();
        let tab = s.open("https://example.com".into()).await.unwrap()["tab"]
            .as_u64()
            .unwrap() as u32;
        let tab = TabId(tab);
        let mut rx1 = s.subscribe_events();
        let mut rx2 = s.subscribe_events();
        s.navigate("https://example.com/search".into())
            .await
            .unwrap();
        let a = tokio::time::timeout(Duration::from_secs(2), rx1.recv())
            .await
            .unwrap()
            .unwrap();
        let b = tokio::time::timeout(Duration::from_secs(2), rx2.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(a.0, tab);
        assert_eq!(b.0, tab);
        assert!(matches!(a.1, PageEvent::NavigationStarted { .. }));
    }

    #[tokio::test]
    async fn async_pump_stops_when_shell_dropped() {
        let fb = Arc::new(Fastbrowser::new());
        fb.init(Config::default()).unwrap();
        let _tab = TabId(
            fb.open("https://example.com").unwrap()["tab"]
                .as_u64()
                .unwrap() as u32,
        );
        let s = AsyncFastbrowser::spawn(fb.clone());
        let mut rx = s.subscribe_events();
        s.navigate("https://example.com/search".into())
            .await
            .unwrap();
        // 收到至少一次事件（泵在工作）
        let _ = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .unwrap()
            .unwrap();
        // 排空广播缓冲
        while let Ok(Ok(_)) = tokio::time::timeout(Duration::from_millis(40), rx.recv()).await {
            // drain
        }
        drop(s); // 泵任务 abort（异步生效）
        tokio::time::sleep(Duration::from_millis(30)).await; // 给 abort 落地时间
                                                             // 再用同步壳触发导航：事件只进引擎缓冲，不再推送到频道
        fb.navigate("https://example.com/login").unwrap();
        let res = tokio::time::timeout(Duration::from_millis(80), rx.recv()).await;
        match res {
            Err(_) => {}                                                    // 超时：泵已停 → 正确
            Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) => {} // 所有 sender 已 drop → 同样正确
            Ok(Ok(_)) => panic!("event delivered after pump stop"),
            Ok(Err(_)) => panic!("unexpected recv error"),
        }
    }

    // ── 并发语义 ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn async_concurrent_same_tab_serializes() {
        let s = mock_shell();
        let tab = s.open("https://example.com".into()).await.unwrap()["tab"]
            .as_u64()
            .unwrap() as u32;
        // 同一标签页 10 次追加输入（clear=false）：per-tab 锁保证无丢失更新
        let tasks: Vec<(String, Value)> = (0..10)
            .map(|_| {
                (
                    "type".into(),
                    json!({"id": "e", "text": "x", "clear": false, "tab": tab}),
                )
            })
            .collect();
        let res = s.run_concurrently(tasks).await.unwrap();
        assert_eq!(res.len(), 10);
        let v = s
            .tool_call("get_element_info".into(), json!({"id": "e", "tab": tab}))
            .await
            .unwrap();
        assert_eq!(
            v["value"].as_str().unwrap().len(),
            10,
            "no update must be lost"
        );
    }

    #[tokio::test]
    async fn async_concurrent_error_propagation() {
        let s = mock_shell();
        let tab = s.open("https://example.com".into()).await.unwrap()["tab"]
            .as_u64()
            .unwrap() as u32;
        let res = s
            .run_concurrently(vec![
                ("type".into(), json!({"id": "e", "text": "ok", "tab": tab})),
                ("nope_tool".into(), json!({})),
                ("type".into(), json!({"id": "e", "text": "ok2", "tab": tab})),
            ])
            .await;
        assert!(res.is_err(), "invalid tool must surface as error");
        // 合法的调用仍执行了
        let v = s
            .tool_call("get_element_info".into(), json!({"id": "e", "tab": tab}))
            .await
            .unwrap();
        let val = v["value"].as_str().unwrap_or("");
        assert!(
            val == "ok" || val == "ok2",
            "valid calls must still execute, got '{val}'"
        );
    }

    #[tokio::test]
    async fn async_stress_many_concurrent_calls() {
        let s = mock_shell();
        let opened = s
            .open_many(vec![
                "https://example.com".into(),
                "https://example.com/login".into(),
                "https://example.com/search".into(),
            ])
            .await
            .unwrap();
        let tabs: Vec<u32> = opened
            .iter()
            .map(|v| v["tab"].as_u64().unwrap() as u32)
            .collect();
        let mut tasks = Vec::new();
        for i in 0..30 {
            tasks.push(("get_page_title".into(), json!({"tab": tabs[i % 3]})));
        }
        let res = s.run_concurrently(tasks).await.unwrap();
        assert_eq!(res.len(), 30);
        // 全部返回标题
        for r in &res {
            assert!(r["title"].is_string());
        }
    }

    // ── 会话 / 状态 ──────────────────────────────────────────────────

    #[tokio::test]
    async fn async_session_save_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json").to_str().unwrap().to_string();
        let s = mock_shell();
        s.open("https://example.com".into()).await.unwrap();
        s.tool_call(
            "cookie_set".into(),
            json!({"name": "sid", "value": "abc", "domain": "example.com"}),
        )
        .await
        .unwrap();
        s.tool_call("storage_set".into(), json!({"key": "token", "value": "t1"}))
            .await
            .unwrap();
        s.session_save(path.clone()).await.unwrap();
        s.shutdown().await;

        let fb = Arc::new(Fastbrowser::new());
        fb.init(Config::default()).unwrap();
        let s2 = AsyncFastbrowser::spawn(fb);
        s2.session_load(path).await.unwrap();
        s2.open("https://example.com".into()).await.unwrap();
        let c = s2
            .tool_call("cookie_get".into(), json!({"domain": "example.com"}))
            .await
            .unwrap();
        assert_eq!(c["cookies"][0]["value"], "abc");
        let st = s2
            .tool_call("storage_get".into(), json!({"key": "token"}))
            .await
            .unwrap();
        assert_eq!(st["value"], "t1");
    }

    #[tokio::test]
    async fn async_clear_state_isolates() {
        let s = mock_shell();
        s.open("https://example.com".into()).await.unwrap();
        s.open("https://example.com/login".into()).await.unwrap();
        s.tool_call(
            "cookie_set".into(),
            json!({"name": "x", "value": "v", "domain": "example.com"}),
        )
        .await
        .unwrap();
        let out = s.clear_state().await.unwrap();
        assert_eq!(out["cleared"], true);
        let tabs = s.tool_call("list_tabs".into(), json!({})).await.unwrap();
        assert_eq!(tabs["tabs"].as_array().unwrap().len(), 1);
        let c = s.tool_call("cookie_get".into(), json!({})).await.unwrap();
        assert_eq!(c["cookies"].as_array().unwrap().len(), 0);
    }

    // ── 帧流 / 等待 ──────────────────────────────────────────────────

    #[tokio::test]
    async fn async_frame_stream_mock() {
        use crate::engine::host::ViewFrameSink;
        use std::sync::atomic::{AtomicUsize, Ordering};
        // 先注册帧回调，再 open（open 时会把 sink 附加到标签页）
        let fb = Arc::new(Fastbrowser::new());
        fb.init(Config::default()).unwrap();
        let count = Arc::new(AtomicUsize::new(0));
        struct Sink(Arc<AtomicUsize>);
        impl ViewFrameSink for Sink {
            fn on_view_frame(&self, _t: crate::engine::TabId, _f: &crate::engine::ViewFrame) {
                self.0.fetch_add(1, Ordering::Relaxed);
            }
        }
        fb.register_frame_sink(Arc::new(Sink(count.clone())));
        let s = AsyncFastbrowser::spawn(fb);
        let tab = TabId(
            s.open("https://example.com".into()).await.unwrap()["tab"]
                .as_u64()
                .unwrap() as u32,
        );
        s.start_frame_stream(
            tab,
            FrameStreamOptions {
                fps: 100,
                max_width: 0,
                max_height: 0,
                format: "png".into(),
            },
        )
        .await
        .unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(
            count.load(Ordering::Relaxed) >= 2,
            "frames should be pushed"
        );
        s.stop_frame_stream(tab).await.unwrap();
        tokio::time::sleep(Duration::from_millis(80)).await;
        let a = count.load(Ordering::Relaxed);
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert_eq!(
            a,
            count.load(Ordering::Relaxed),
            "frames must stop after stop"
        );
    }

    #[tokio::test]
    async fn async_wait_timeouts() {
        let s = mock_shell();
        let bad = TabId(999);
        let res = s.wait_for_navigation(bad, Duration::from_millis(30)).await;
        assert!(res.is_err());
        let res = s
            .wait_for_load_state(bad, "load", Duration::from_millis(30))
            .await;
        assert!(res.is_err());
        let res = s
            .wait_for_condition(bad, "false".into(), Duration::from_millis(30))
            .await;
        assert!(res.is_err());
    }

    // ── per-tab 读取 / 双壳共存 ──────────────────────────────────────

    #[tokio::test]
    async fn async_page_url_title_per_tab() {
        let s = mock_shell();
        let t1 = s.open("https://example.com".into()).await.unwrap()["tab"]
            .as_u64()
            .unwrap() as u32;
        let t2 = s.open("https://example.com/login".into()).await.unwrap()["tab"]
            .as_u64()
            .unwrap() as u32;
        let u1 = s.page_url(TabId(t1)).await.unwrap();
        assert!(u1.contains("example.com"));
        let title2 = s.page_title(TabId(t2)).await.unwrap();
        assert_eq!(title2, "Login");
    }

    #[tokio::test]
    async fn async_sync_shell_coexistence() {
        let s = mock_shell();
        let tab = s.open("https://example.com".into()).await.unwrap()["tab"]
            .as_u64()
            .unwrap() as u32;
        // 同步壳直接操作同一实例
        let v = s.inner().tool_call("get_page_title", json!({})).unwrap();
        assert_eq!(v["title"], "Example Page");
        // 异步壳继续
        let v2 = s
            .tool_call("get_current_url".into(), json!({}))
            .await
            .unwrap();
        assert_eq!(v2["url"], "https://example.com");
        // 同步事件读取仍可用（无订阅者时泵不抽）
        let evs = s
            .inner()
            .runtime()
            .unwrap()
            .as_ref()
            .unwrap()
            .engine()
            .drain_events(TabId(tab));
        assert!(!evs.is_empty());
    }
}
