// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! CDP transport abstraction.
//!
//! `Transport` methods take `&self`; implementations achieve concurrency safety by
//! splitting the sink and having a background receive task push into a channel:
//! sends lock only briefly, receives wait lock-free, avoiding a read/write deadlock.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures_util::stream::SplitSink;
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use crate::engine::{EngineError, ErrorKind, Result};

/// Transport abstraction: implemented by WebSocket and the test stub.
#[async_trait]
pub trait Transport: Send + Sync {
    async fn send_text(&self, msg: String) -> Result<()>;
    /// Returning `None` means the connection is closed.
    async fn recv_text(&self) -> Result<Option<String>>;
}

/// Real WebSocket transport (tokio-tungstenite).
pub struct WebSocketTransport {
    sink: Arc<Mutex<SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>>>,
    inbound: Arc<Mutex<tokio::sync::mpsc::UnboundedReceiver<Result<Option<String>>>>>,
}

impl WebSocketTransport {
    pub async fn connect(url: &str) -> Result<Self> {
        let fut = tokio_tungstenite::connect_async(url);
        let (stream, _) = tokio::time::timeout(Duration::from_secs(10), fut)
            .await
            .map_err(|_| {
                EngineError::new(ErrorKind::Navigation, format!("cdp connect timeout: {url}"))
            })?
            .map_err(|e| {
                EngineError::new(ErrorKind::Navigation, format!("cdp connect {url}: {e}"))
            })?;
        let (sink, mut stream) = stream.split();
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        // Background receive task: pushes messages into a channel so senders need not hold the receive lock.
        tokio::spawn(async move {
            loop {
                match stream.next().await {
                    Some(Ok(Message::Text(t))) => {
                        if tx.send(Ok(Some(t.to_string()))).is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Binary(b))) => {
                        let r = String::from_utf8(b.to_vec()).map(Some).map_err(|_| {
                            EngineError::new(ErrorKind::Navigation, "cdp: non-utf8 binary")
                        });
                        if tx.send(r).is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Ping(_))) | Some(Ok(Message::Pong(_))) => continue,
                    Some(Ok(Message::Close(_))) | None => {
                        let _ = tx.send(Ok(None));
                        break;
                    }
                    Some(Err(e)) => {
                        let _ = tx.send(Err(EngineError::new(
                            ErrorKind::Navigation,
                            format!("cdp recv: {e}"),
                        )));
                        break;
                    }
                    _ => {
                        let _ = tx.send(Ok(None));
                        break;
                    }
                }
            }
        });
        Ok(WebSocketTransport {
            sink: Arc::new(Mutex::new(sink)),
            inbound: Arc::new(Mutex::new(rx)),
        })
    }
}

#[async_trait]
impl Transport for WebSocketTransport {
    async fn send_text(&self, msg: String) -> Result<()> {
        let mut sink = self.sink.lock().await;
        sink.send(Message::Text(msg))
            .await
            .map_err(|e| EngineError::new(ErrorKind::Navigation, format!("cdp send: {e}")))
    }

    async fn recv_text(&self) -> Result<Option<String>> {
        let mut inbound = self.inbound.lock().await;
        match inbound.recv().await {
            Some(r) => r,
            None => Ok(None),
        }
    }
}

/// Build a devtools page websocket URL.
pub fn ws_url_for(port: u16, target_id: &str) -> String {
    format!("ws://127.0.0.1:{port}/devtools/page/{target_id}")
}

/// Fetch the page target list from `http://127.0.0.1:PORT/json`.
pub fn list_url_for(port: u16) -> String {
    format!("http://127.0.0.1:{port}/json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ws_url_format() {
        assert_eq!(
            ws_url_for(9222, "abc"),
            "ws://127.0.0.1:9222/devtools/page/abc"
        );
        assert_eq!(list_url_for(9222), "http://127.0.0.1:9222/json");
    }
}
