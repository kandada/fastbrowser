// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! `download` tool regression tests (mock engine, no browser required).
//!
//! The download tool fetches a URL over a tiny blocking HTTPS client and writes
//! the body to an absolute host path, byte-for-byte. These tests drive it
//! through the SDK against a local HTTP server so binary payloads, status codes,
//! parent-dir creation and error shapes are all pinned.

use std::io::{Read, Write};
use std::net::TcpListener;

use serde_json::json;

use fastbrowser::sdk::Fastbrowser;
use fastbrowser::Config;

/// Binary payload with non-UTF8 bytes to prove the transfer is binary-safe.
const PNG: &[u8] = &[
    0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0xFF, 0xFE, 0x80,
];

/// Spawn a one-shot-per-connection HTTP server. Returns the base URL.
///
/// Routes:
///   /pic.png   → 200 image/png, PNG bytes
///   /missing   → 404 text/plain
///   anything   → 200 text/plain, "ok"
fn serve() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut sock) = stream else { break };
            let mut buf = [0u8; 2048];
            let n = sock.read(&mut buf).unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]);
            let path = req.split_whitespace().nth(1).unwrap_or("/").to_string();
            let (status, ctype, body): (&str, &str, Vec<u8>) = match path.as_str() {
                "/pic.png" => ("200 OK", "image/png", PNG.to_vec()),
                "/missing" => ("404 Not Found", "text/plain", b"nf".to_vec()),
                _ => ("200 OK", "text/plain", b"ok".to_vec()),
            };
            let header = format!(
                "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = sock.write_all(header.as_bytes());
            let _ = sock.write_all(&body);
        }
    });
    format!("http://127.0.0.1:{port}")
}

fn sdk() -> Fastbrowser {
    let s = Fastbrowser::new();
    s.init(Config::for_engine("mock")).unwrap();
    s
}

#[test]
fn download_writes_binary_verbatim() {
    let base = serve();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pic.png");
    let s = sdk();

    let out = s
        .tool_call(
            "download",
            json!({"url": format!("{base}/pic.png"), "path": path.to_string_lossy()}),
        )
        .unwrap();

    assert_eq!(out["ok"], true, "{out}");
    assert_eq!(out["bytes"], PNG.len(), "{out}");
    assert_eq!(out["content_type"], "image/png", "{out}");
    assert_eq!(out["path"].as_str().unwrap(), path.to_string_lossy());
    assert_eq!(
        std::fs::read(&path).unwrap(),
        PNG,
        "file must be byte-identical"
    );
}

#[test]
fn download_creates_missing_parent_dirs() {
    let base = serve();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a").join("b").join("c").join("file.bin");
    assert!(!path.parent().unwrap().exists());

    let s = sdk();
    let out = s
        .tool_call(
            "download",
            json!({"url": format!("{base}/file.bin"), "path": path.to_string_lossy()}),
        )
        .unwrap();

    assert_eq!(out["ok"], true, "{out}");
    assert_eq!(std::fs::read(&path).unwrap(), b"ok");
}

#[test]
fn download_http_error_reports_status_without_writing() {
    let base = serve();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("missing.bin");
    let s = sdk();

    let out = s
        .tool_call(
            "download",
            json!({"url": format!("{base}/missing"), "path": path.to_string_lossy()}),
        )
        .unwrap();

    assert_eq!(out["ok"], false, "{out}");
    assert_eq!(out["status"], 404, "{out}");
    assert!(!path.exists(), "a failed download must not create the file");
}

#[test]
fn download_requires_url_and_path() {
    let s = sdk();
    assert!(s.tool_call("download", json!({"path": "/tmp/x"})).is_err());
    assert!(s
        .tool_call("download", json!({"url": "http://x/"}))
        .is_err());
    assert!(s
        .tool_call("download", json!({"url": "", "path": "/tmp/x"}))
        .is_err());
    assert!(s
        .tool_call("download", json!({"url": "http://x/", "path": ""}))
        .is_err());
}

#[test]
fn download_connection_failure_is_a_structured_error() {
    let s = sdk();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nope.bin");
    // Port 1 on localhost is refused immediately.
    let r = s.tool_call(
        "download",
        json!({"url": "http://127.0.0.1:1/nope", "path": path.to_string_lossy()}),
    );
    assert!(r.is_err(), "connection failure should error: {r:?}");
}

#[test]
fn download_is_registered_and_advertised() {
    let s = sdk();
    let names: Vec<String> = s
        .tool_list()
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t["name"].as_str().map(str::to_string))
        .collect();
    assert!(names.contains(&"download".to_string()), "download missing");
}
