// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! CDP 客户端：命令-响应匹配、事件缓冲、WebSocket 连接。
//!
//! 使用独立 tokio Runtime（内核 trait 为同步接口，内部 block_on）。

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

use crate::cdp::command::Command;
use crate::cdp::event::CdpEvent;
use crate::cdp::transport::{Transport, WebSocketTransport};
use crate::engine::{EngineError, ErrorKind, Result};

/// CDP 客户端。
pub struct CdpClient {
    /// Owned tokio runtime. Stored as `Option` so that `shutdown`/`Drop` can
    /// move it to a dedicated std thread: dropping a multi-thread tokio
    /// runtime inline panics if we are on another tokio worker thread (e.g. a
    /// Tauri command tearing down a browser session).
    rt: Option<tokio::runtime::Runtime>,
    tx: Arc<dyn Transport>,
    next_id: AtomicU64,
    pending: Arc<Mutex<HashMap<u64, tokio::sync::oneshot::Sender<Result<Value>>>>>,
    events: Arc<Mutex<VecDeque<CdpEvent>>>,
    reader: Option<tokio::task::JoinHandle<()>>,
    timeout_ms: u64,
}

impl CdpClient {
    /// 连接到一个 devtools websocket 端点。
    pub fn connect(ws_url: &str, timeout_ms: u64) -> Result<Self> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|e| EngineError::new(ErrorKind::Internal, format!("tokio runtime: {e}")))?;
        let transport = rt
            .block_on(WebSocketTransport::connect(ws_url))
            .map_err(|e| EngineError::new(ErrorKind::Navigation, format!("cdp connect: {e}")))?;
        Self::from_transport(Some(rt), Box::new(transport), timeout_ms)
    }

    /// 从既有传输构造客户端（测试用）。
    pub fn from_transport(
        rt: Option<tokio::runtime::Runtime>,
        transport: Box<dyn Transport>,
        timeout_ms: u64,
    ) -> Result<Self> {
        let tx: Arc<dyn Transport> = Arc::from(transport);
        let mut client = CdpClient {
            rt,
            tx,
            next_id: AtomicU64::new(1),
            pending: Arc::new(Mutex::new(HashMap::new())),
            events: Arc::new(Mutex::new(VecDeque::new())),
            reader: None,
            timeout_ms,
        };
        client.spawn_reader();
        Ok(client)
    }

    fn spawn_reader(&mut self) {
        let tx = self.tx.clone();
        let events = self.events.clone();
        let pending = self.pending.clone();
        let handle = self
            .rt
            .as_ref()
            .expect("cdp runtime present")
            .spawn(async move {
                loop {
                    let text = match tx.recv_text().await {
                        Ok(Some(t)) => t,
                        _ => break,
                    };
                    let v: Value = match serde_json::from_str(&text) {
                        Ok(v) => v,
                        Err(_) => continue,
                    };
                    if let Some(id) = v.get("id").and_then(Value::as_u64) {
                        if let Some(sx) = pending
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .remove(&id)
                        {
                            let result = if let Some(err) = v.get("error") {
                                Err(EngineError::new(
                                    ErrorKind::Navigation,
                                    format!("cdp error: {err}"),
                                ))
                            } else {
                                Ok(v.clone())
                            };
                            let _ = sx.send(result);
                        }
                    } else {
                        let method = v.get("method").and_then(Value::as_str).map(String::from);
                        let session_id =
                            v.get("sessionId").and_then(Value::as_str).map(String::from);
                        let params = v
                            .get("params")
                            .cloned()
                            .unwrap_or(Value::Object(Default::default()));
                        if let Some(method) = method {
                            events
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .push_back(CdpEvent::with_session(method, params, session_id));
                        }
                    }
                }
            });
        self.reader = Some(handle);
    }

    /// 发送命令并等待响应。
    pub fn send(&self, method: &str, params: Value) -> Result<Value> {
        self.send_with_session(None, method, params)
    }

    /// 发送命令（可指定 sessionId，多标签页用；空串视为无）。
    pub fn send_with_session(
        &self,
        session_id: Option<&str>,
        method: &str,
        params: Value,
    ) -> Result<Value> {
        let sid = session_id.filter(|s| !s.is_empty());
        let cmd = Command::new(method, params);
        self.command_session(sid, &cmd)
    }

    /// 发送命令（可指定 sessionId + 自定义超时）。
    pub fn send_with_session_timeout(
        &self,
        session_id: Option<&str>,
        method: &str,
        params: Value,
        timeout_ms: u64,
    ) -> Result<Value> {
        let sid = session_id.filter(|s| !s.is_empty());
        let cmd = Command::new(method, params);
        self.command_session_timeout(sid, &cmd, timeout_ms)
    }

    /// 发送命令（构造器）并等待响应。
    pub fn command(&self, cmd: &Command) -> Result<Value> {
        self.command_session(None, cmd)
    }

    /// 发送命令（构造器，带 sessionId）。
    pub fn command_session(&self, session_id: Option<&str>, cmd: &Command) -> Result<Value> {
        self.command_session_timeout(session_id, cmd, self.timeout_ms)
    }

    /// 发送命令（构造器，带 sessionId 与自定义超时；供快速内省类命令使用，
    /// 避免页面导航期间阻塞过久）。
    pub fn command_session_timeout(
        &self,
        session_id: Option<&str>,
        cmd: &Command,
        timeout_ms: u64,
    ) -> Result<Value> {
        self.rt
            .as_ref()
            .expect("cdp runtime present")
            .block_on(self.command_session_timeout_async(session_id, cmd, timeout_ms))
    }

    // ── 异步（可并发流水线）命令 API ──────────────────────────────
    //
    // 同一连接上可同时有多个命令在途：命令按自增 `id` 发送，后台 reader 按 `id`
    // 把响应路由回各自的 oneshot。异步版本不再 `block_on`，由调用方所在 runtime
    // 驱动，因此不同标签页/不同 Agent 的命令可以真正交错（流水线化）。

    /// 发送命令并等待响应（async）。返回 future 需在某个 tokio runtime 内被 poll。
    pub async fn command_session_timeout_async(
        &self,
        session_id: Option<&str>,
        cmd: &Command,
        timeout_ms: u64,
    ) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (sx, rx) = tokio::sync::oneshot::channel();
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id, sx);

        let mut msg = serde_json::Map::new();
        msg.insert("id".into(), json!(id));
        if let Some(s) = session_id {
            msg.insert("sessionId".into(), json!(s));
        }
        msg.insert("method".into(), json!(cmd.method));
        msg.insert("params".into(), cmd.params.clone());
        let msg_str = Value::Object(msg).to_string();

        let tx = self.tx.clone();
        if let Err(e) = tx.send_text(msg_str).await {
            self.pending
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&id);
            return Err(e);
        }

        match tokio::time::timeout(Duration::from_millis(timeout_ms), rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(EngineError::new(ErrorKind::Navigation, "cdp reader closed")),
            Err(_) => Err(EngineError::new(
                ErrorKind::Timeout,
                format!("cdp command '{}' timed out", cmd.method),
            )),
        }
    }

    /// async：构造器命令（无 session）。
    pub async fn command_async(&self, cmd: &Command) -> Result<Value> {
        self.command_session_timeout_async(None, cmd, self.timeout_ms)
            .await
    }

    /// async：构造器命令（带 session）。
    pub async fn command_session_async(
        &self,
        session_id: Option<&str>,
        cmd: &Command,
    ) -> Result<Value> {
        self.command_session_timeout_async(session_id, cmd, self.timeout_ms)
            .await
    }

    /// async：发送方法命令并等待响应（无 session）。
    pub async fn send_async(&self, method: &str, params: Value) -> Result<Value> {
        self.send_with_session_timeout_async(None, method, params, self.timeout_ms)
            .await
    }

    /// async：发送方法命令并等待响应（带 session）。
    pub async fn send_with_session_async(
        &self,
        session_id: Option<&str>,
        method: &str,
        params: Value,
    ) -> Result<Value> {
        self.send_with_session_timeout_async(session_id, method, params, self.timeout_ms)
            .await
    }

    /// async：发送方法命令并等待响应（带 session 与自定义超时）。
    pub async fn send_with_session_timeout_async(
        &self,
        session_id: Option<&str>,
        method: &str,
        params: Value,
        timeout_ms: u64,
    ) -> Result<Value> {
        let sid = session_id.filter(|s| !s.is_empty());
        let cmd = Command::new(method, params);
        self.command_session_timeout_async(sid, &cmd, timeout_ms)
            .await
    }

    /// 并发发送一批命令（流水线），全部完成后返回各自结果（按输入顺序）。
    /// 任一命令失败时返回首个错误。
    pub async fn send_all_async(&self, cmds: &[Command]) -> Result<Vec<Value>> {
        use futures_util::future::join_all;
        let futs: Vec<_> = cmds.iter().map(|c| self.command_async(c)).collect();
        let results = join_all(futs).await;
        results.into_iter().collect()
    }

    /// 在客户端自带的 tokio runtime 上同步驱动一个 future。
    /// 供同步调用方（如多 Agent 线程）在不想自建 runtime 时运行异步命令批。
    pub fn block_on<F: std::future::Future>(&self, fut: F) -> F::Output {
        self.rt.as_ref().expect("cdp runtime present").block_on(fut)
    }

    /// 拉取缓冲事件。
    pub fn drain_events(&self) -> Vec<CdpEvent> {
        self.events
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .drain(..)
            .collect()
    }

    /// 把被某消费者「让出」的事件放回缓冲队列尾部（帧泵线程用：只消费
    /// screencast 帧，其余事件回灌给 `route_cdp_events` 消费）。有界防爆。
    pub fn reingest(&self, events: Vec<CdpEvent>) {
        if events.is_empty() {
            return;
        }
        let mut q = self.events.lock().unwrap_or_else(|e| e.into_inner());
        for e in events {
            q.push_back(e);
        }
        // 回灌上限：防 pump 与 route 竞态下长时间不消费导致无限增长
        while q.len() > 65_536 {
            q.pop_front();
        }
    }

    /// 关闭连接。
    pub fn shutdown(&mut self) {
        if let Some(h) = self.reader.take() {
            h.abort();
        }
        // Move the owned tokio runtime to a dedicated std thread before
        // dropping it. Dropping a multi-thread tokio runtime requires
        // blocking on its shutdown and panics ("Cannot drop a runtime in a
        // context where blocking is not allowed") when performed from within
        // another tokio worker thread — which is the norm for a desktop host
        // (Tauri async commands) tearing down a browser session.
        if let Some(rt) = self.rt.take() {
            std::thread::spawn(move || drop(rt));
        }
    }

    /// 批量发送并聚合结果（部分失败时返回错误）。
    pub fn send_all(&self, cmds: &[Command]) -> Result<Vec<Value>> {
        cmds.iter().map(|c| self.command(c)).collect()
    }
}

impl Drop for CdpClient {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cdp::command::{page, runtime};
    use async_trait::async_trait;
    use std::sync::atomic::AtomicU64;

    /// 测试传输：记录发送内容（不自动应答）。
    struct FakeTransport {
        incoming: Arc<tokio::sync::Mutex<tokio::sync::mpsc::UnboundedReceiver<String>>>,
        sent: Arc<Mutex<Vec<String>>>,
        sent_count: Arc<AtomicU64>,
    }

    impl FakeTransport {
        fn new(
            sent: Arc<Mutex<Vec<String>>>,
            sent_count: Arc<AtomicU64>,
        ) -> (Self, tokio::sync::mpsc::UnboundedSender<String>) {
            let (sx, rx) = tokio::sync::mpsc::unbounded_channel();
            (
                FakeTransport {
                    incoming: Arc::new(tokio::sync::Mutex::new(rx)),
                    sent,
                    sent_count,
                },
                sx,
            )
        }
    }

    #[async_trait]
    impl Transport for FakeTransport {
        async fn send_text(&self, msg: String) -> Result<()> {
            self.sent.lock().unwrap().push(msg);
            self.sent_count.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        async fn recv_text(&self) -> Result<Option<String>> {
            let mut rx = self.incoming.lock().await;
            Ok(rx.recv().await)
        }
    }

    /// 自动应答传输：解析命令 id，回一条成功响应或错误响应。
    struct AutoResponder {
        incoming: Arc<tokio::sync::Mutex<tokio::sync::mpsc::UnboundedReceiver<String>>>,
        sender: tokio::sync::mpsc::UnboundedSender<String>,
        result: Value,
        error: Option<String>,
    }

    #[async_trait]
    impl Transport for AutoResponder {
        async fn send_text(&self, msg: String) -> Result<()> {
            let id = serde_json::from_str::<Value>(&msg)
                .ok()
                .and_then(|v| v.get("id").cloned())
                .unwrap_or(Value::Null);
            let resp = match &self.error {
                Some(e) => json!({ "id": id, "error": { "code": -32601, "message": e } }),
                None => json!({ "id": id, "result": self.result }),
            };
            let _ = self.sender.send(resp.to_string());
            Ok(())
        }
        async fn recv_text(&self) -> Result<Option<String>> {
            let mut rx = self.incoming.lock().await;
            Ok(rx.recv().await)
        }
    }

    impl AutoResponder {
        fn new_parts(result: Value) -> (AutoResponder, tokio::sync::mpsc::UnboundedSender<String>) {
            let (sx, rx) = tokio::sync::mpsc::unbounded_channel();
            (
                AutoResponder {
                    incoming: Arc::new(tokio::sync::Mutex::new(rx)),
                    sender: sx.clone(),
                    result,
                    error: None,
                },
                sx,
            )
        }
        fn error_parts(msg: &str) -> (AutoResponder, tokio::sync::mpsc::UnboundedSender<String>) {
            let (sx, rx) = tokio::sync::mpsc::unbounded_channel();
            (
                AutoResponder {
                    incoming: Arc::new(tokio::sync::Mutex::new(rx)),
                    sender: sx.clone(),
                    result: Value::Null,
                    error: Some(msg.to_string()),
                },
                sx,
            )
        }
    }

    fn test_rt() -> tokio::runtime::Runtime {
        // 多线程运行时：后台 reader 自动轮询（事件缓冲测试依赖）。
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap()
    }

    #[test]
    fn command_response_matching() {
        let rt = test_rt();
        let (transport, _feed) = AutoResponder::new_parts(json!({ "frameId": "f1" }));
        let client = CdpClient::from_transport(Some(rt), Box::new(transport), 5000).unwrap();
        let resp = client
            .command(&page::navigate("https://example.com"))
            .unwrap();
        assert_eq!(resp["result"]["frameId"], "f1");
    }

    #[test]
    fn error_responses_are_errors() {
        let rt = test_rt();
        let (transport, _) = AutoResponder::error_parts("boom");
        let client = CdpClient::from_transport(Some(rt), Box::new(transport), 5000).unwrap();
        let resp = client.command(&runtime::evaluate("x", false, true));
        assert!(resp.is_err());
    }

    #[test]
    fn events_are_buffered() {
        let rt = test_rt();
        let (transport, feed) = AutoResponder::new_parts(json!({}));
        let client = CdpClient::from_transport(Some(rt), Box::new(transport), 5000).unwrap();
        feed.send(
            json!({"method": "Page.loadEventFired", "params": {"timestamp": 1.0}}).to_string(),
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(80));
        let evs = client.drain_events();
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].method, "Page.loadEventFired");
    }

    #[test]
    fn framing_and_ids_increment() {
        let rt = test_rt();
        let sent: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let count = Arc::new(AtomicU64::new(0));
        let (transport, _feed) = FakeTransport::new(sent.clone(), count.clone());
        // 短超时：不回响应会走超时分支
        let client = CdpClient::from_transport(Some(rt), Box::new(transport), 100).unwrap();
        let res = client.command(&page::navigate("u"));
        assert!(res.is_err()); // 超时
        let frames = sent.lock().unwrap();
        let first: Value = serde_json::from_str(&frames[0]).unwrap();
        assert_eq!(first["method"], "Page.navigate");
        assert_eq!(first["id"], 1);
    }

    /// 回显传输：把命令的 `id` 原样放进结果返回，用于验证并发下响应按 id 正确路由。
    struct EchoTransport {
        incoming: Arc<tokio::sync::Mutex<tokio::sync::mpsc::UnboundedReceiver<String>>>,
        sender: tokio::sync::mpsc::UnboundedSender<String>,
    }

    #[async_trait]
    impl Transport for EchoTransport {
        async fn send_text(&self, msg: String) -> Result<()> {
            let id = serde_json::from_str::<Value>(&msg)
                .ok()
                .and_then(|v| v.get("id").cloned())
                .unwrap_or(Value::Null);
            let _ = self
                .sender
                .send(json!({ "id": id, "result": { "echo": id } }).to_string());
            Ok(())
        }
        async fn recv_text(&self) -> Result<Option<String>> {
            let mut rx = self.incoming.lock().await;
            Ok(rx.recv().await)
        }
    }

    impl EchoTransport {
        fn new_parts() -> (EchoTransport, tokio::sync::mpsc::UnboundedSender<String>) {
            let (sx, rx) = tokio::sync::mpsc::unbounded_channel();
            (
                EchoTransport {
                    incoming: Arc::new(tokio::sync::Mutex::new(rx)),
                    sender: sx.clone(),
                },
                sx,
            )
        }
    }

    /// 流水线核心：N 个异步命令同时在途，响应按 id 一一对应（不错配）。
    #[test]
    fn concurrent_pipeline_routes_responses_by_id() {
        let rt = test_rt();
        let (transport, _feed) = EchoTransport::new_parts();
        let client =
            Arc::new(CdpClient::from_transport(Some(rt), Box::new(transport), 5000).unwrap());

        let n = 64usize;
        let client_for_tasks = client.clone();
        client.block_on(async move {
            let futs: Vec<_> = (0..n)
                .map(|i| {
                    let c = client_for_tasks.clone();
                    let cmd =
                        Command::new("Runtime.evaluate", json!({ "expression": format!("{i}") }));
                    async move {
                        let resp = c.command_async(&cmd).await.unwrap();
                        // EchoTransport 回显命令自身的 id：每个调用者都应收到【自己那条】的响应。
                        resp["result"]["echo"].as_u64().unwrap()
                    }
                })
                .collect();
            let mut echoes = futures_util::future::join_all(futs).await;
            // id 从 1 起自增：N 个并发命令的响应应恰好覆盖 1..=N，无丢失、无重复、无错配。
            echoes.sort_unstable();
            assert_eq!(
                echoes,
                (1..=n as u64).collect::<Vec<_>>(),
                "pipelined responses misrouted/lost"
            );
        });
    }

    /// 回归：在另一个 tokio worker 线程上 drop `CdpClient`（其自带的 runtime）
    /// 不再 panic。这是桌面宿主（Tauri async 命令）拆除浏览器会话时的典型路径：
    /// 之前 `rt: Runtime` 被内联 drop → "Cannot drop a runtime in a context
    /// where blocking is not allowed"。修复后 shutdown 把 runtime 移到独立线程释放。
    #[test]
    fn dropping_client_from_tokio_worker_does_not_panic() {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        let incoming = Arc::new(tokio::sync::Mutex::new(rx));
        let transport = AutoResponder {
            incoming,
            sender: tx.clone(),
            result: Value::Null,
            error: None,
        };
        let rt = test_rt();
        let client = CdpClient::from_transport(Some(rt), Box::new(transport), 100).unwrap();

        // 主线程是 tokio worker：这里 drop 客户端应正常结束，不 panic。
        let main = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap();
        main.block_on(async move {
            drop(client);
        });
    }

    /// 多线程同步调用（多 Agent 各自持有引用并发驱动同一浏览器）：命令不串线。
    #[test]
    fn concurrent_threads_on_shared_client() {
        let rt = test_rt();
        let (transport, _feed) = EchoTransport::new_parts();
        let client =
            Arc::new(CdpClient::from_transport(Some(rt), Box::new(transport), 5000).unwrap());

        let threads: Vec<_> = (0..8)
            .map(|t| {
                let c = client.clone();
                std::thread::spawn(move || {
                    for i in 0..25 {
                        let cmd = Command::new(
                            "Runtime.evaluate",
                            json!({ "expression": format!("{t}-{i}") }),
                        );
                        let resp = c.command(&cmd).unwrap();
                        // EchoTransport 回显调用方自己的 id → 响应必然正确路由，否则会 panic 或错配
                        let _ = resp;
                    }
                    t
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
    }
}
