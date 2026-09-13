// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! CDP endpoint discovery (direct HTTP to /json, /json/version).

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use serde_json::Value;

use crate::engine::{EngineError, ErrorKind, Result};

/// Minimal HTTP GET.
///
/// Key: return as soon as the body is fully read via `Content-Length`; do **not**
/// wait for the connection to close. Chromium's devtools HTTP server may keep the
/// connection alive even after `Connection: close`, so the old implementation
/// blocked until the read timeout (2s each; `bind_active_tab` calls it twice = 4s,
/// also stalling other concurrent RPCs and making the side panel take seconds to open).
pub fn http_get(url: &str) -> Result<String> {
    let rest = url
        .strip_prefix("http://")
        .ok_or_else(|| EngineError::invalid("only http:// supported"))?;
    let (host, path) = rest
        .split_once('/')
        .map(|(h, p)| (h, format!("/{p}")))
        .unwrap_or((rest, "/".to_string()));
    let addr = host
        .parse::<std::net::SocketAddr>()
        .map_err(|_| EngineError::invalid(format!("bad host {host}")))?;
    let mut conn = TcpStream::connect(addr)
        .map_err(|e| EngineError::new(ErrorKind::Io, format!("connect {host}: {e}")))?;
    conn.set_read_timeout(Some(Duration::from_secs(5))).ok();
    conn.write_all(
        format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n").as_bytes(),
    )
    .map_err(|e| EngineError::new(ErrorKind::Io, e.to_string()))?;

    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 8192];
    let mut header_end: Option<usize> = None;
    let mut content_length: Option<usize> = None;
    loop {
        if let (Some(h), Some(len)) = (header_end, content_length) {
            if buf.len() >= h + len {
                break;
            }
        }
        match conn.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if header_end.is_none() {
                    if let Some(pos) = find_subsequence(&buf, b"\r\n\r\n") {
                        header_end = Some(pos + 4);
                        let headers = String::from_utf8_lossy(&buf[..pos]);
                        for line in headers.split("\r\n") {
                            if let Some(v) =
                                line.to_ascii_lowercase().strip_prefix("content-length:")
                            {
                                content_length = v.trim().parse::<usize>().ok();
                            }
                        }
                    }
                }
            }
            Err(_) => break,
        }
    }
    let text = String::from_utf8_lossy(&buf).to_string();
    Ok(text
        .split_once("\r\n\r\n")
        .map(|(_, b)| b.to_string())
        .unwrap_or(text))
}

/// Index of the first occurrence of `needle` in `haystack`.
fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// Find a free TCP port.
pub fn free_port() -> Result<u16> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")
        .map_err(|e| EngineError::new(ErrorKind::Io, format!("free port: {e}")))?;
    let port = listener
        .local_addr()
        .map_err(|e| EngineError::new(ErrorKind::Io, e.to_string()))?
        .port();
    drop(listener);
    Ok(port)
}

/// Discover the browser-level websocket endpoint from `/json/version`.
pub fn discover_browser_ws(port: u16) -> Result<String> {
    let body = http_get(&format!("http://127.0.0.1:{port}/json/version"))?;
    let v: Value = serde_json::from_str(&body).map_err(|e| {
        EngineError::new(
            ErrorKind::Navigation,
            format!("discover /json/version: {e}"),
        )
    })?;
    v.get("webSocketDebuggerUrl")
        .and_then(Value::as_str)
        .map(String::from)
        .ok_or_else(|| EngineError::new(ErrorKind::Navigation, "no webSocketDebuggerUrl"))
}

/// Discover the first page target's websocket endpoint from `/json`.
pub fn discover_page_ws(port: u16) -> Result<String> {
    let body = http_get(&format!("http://127.0.0.1:{port}/json"))?;
    let arr: Vec<Value> = serde_json::from_str(&body)
        .map_err(|e| EngineError::new(ErrorKind::Navigation, format!("discover /json: {e}")))?;
    arr.iter()
        .find(|t| t.get("type").and_then(Value::as_str) == Some("page"))
        .and_then(|t| {
            t.get("webSocketDebuggerUrl")
                .and_then(Value::as_str)
                .map(String::from)
        })
        .ok_or_else(|| {
            EngineError::new(ErrorKind::Navigation, "no page target found on debug port")
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn serve_once(body: &'static str) -> u16 {
        use std::net::TcpListener;
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            if let Ok((mut c, _)) = l.accept() {
                let mut b = [0u8; 512];
                let _ = c.read(&mut b);
                let _ = write!(
                    c,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{}",
                    body.len(),
                    body
                );
            }
        });
        port
    }

    #[test]
    fn http_get_parses_body() {
        let p = serve_once("hello");
        assert_eq!(
            http_get(&format!("http://127.0.0.1:{p}/x")).unwrap(),
            "hello"
        );
    }

    /// Regression: the devtools HTTP endpoint keeps the connection alive, so the
    /// old implementation blocked until the read timeout (2s per call, 4s for
    /// `bind_active_tab`, which also stalled other RPCs). We must return as soon
    /// as the `Content-Length` body has been read.
    #[test]
    fn http_get_does_not_wait_for_connection_close() {
        use std::net::TcpListener;
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let body = "hello-keepalive";
        let b = body.to_string();
        std::thread::spawn(move || {
            if let Ok((mut c, _)) = l.accept() {
                let mut req = [0u8; 512];
                let _ = c.read(&mut req);
                let _ = write!(
                    c,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{}",
                    b.len(),
                    b
                );
                // Keep the socket open (no close) — simulates devtools keep-alive.
                std::thread::sleep(Duration::from_secs(3));
            }
        });
        let start = std::time::Instant::now();
        let out = http_get(&format!("http://127.0.0.1:{port}/x")).unwrap();
        assert_eq!(out, body);
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "http_get must return once the body is read, not wait for close (took {:?})",
            start.elapsed()
        );
    }

    #[test]
    fn find_subsequence_edges() {
        assert_eq!(find_subsequence(b"abcabc", b"bc"), Some(1));
        assert_eq!(find_subsequence(b"abc", b"z"), None);
        assert_eq!(find_subsequence(b"abc", b""), None);
        assert_eq!(find_subsequence(b"", b"a"), None);
        assert_eq!(find_subsequence(b"abc", b"abcd"), None);
    }

    #[test]
    fn http_get_rejects_non_http_scheme() {
        assert!(http_get("https://example.com/").is_err());
        assert!(http_get("ftp://127.0.0.1/x").is_err());
    }

    #[test]
    fn discover_browser_ws_parses_version() {
        let p =
            serve_once(r#"{"webSocketDebuggerUrl":"ws://127.0.0.1:9222/devtools/browser/abc"}"#);
        assert_eq!(
            discover_browser_ws(p).unwrap(),
            "ws://127.0.0.1:9222/devtools/browser/abc"
        );
    }

    #[test]
    fn discover_page_ws_parses_json() {
        let p = serve_once(
            r#"[{"type":"page","webSocketDebuggerUrl":"ws://127.0.0.1:9222/devtools/page/abc"}]"#,
        );
        assert_eq!(
            discover_page_ws(p).unwrap(),
            "ws://127.0.0.1:9222/devtools/page/abc"
        );
    }

    #[test]
    fn free_port_nonzero() {
        assert!(free_port().unwrap() > 0);
    }
}
