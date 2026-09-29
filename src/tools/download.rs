// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! `download` — fetch a URL to a local file, byte-for-byte (binary-safe).
//!
//! Runs in the kernel so every consumer (voya-core, aacode-rs, the CLI) gets
//! the same tool. Uses a small blocking HTTPS client and `std::fs` (the kernel
//! is linked into the host process, so it can write the host filesystem).
//! `path` must be an absolute host path — the caller resolves it (aacode-rs maps
//! a project-relative path to `project_path/<path>`).

use serde_json::json;
use std::io::Read;

use crate::engine::{EngineError, ErrorKind};
use crate::tools::tool::Tool;

/// A desktop-browser UA. Many image/file hosts reject unknown clients.
const BROWSER_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) \
     AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

pub fn tools() -> Vec<Tool> {
    vec![download()]
}

fn download() -> Tool {
    Tool::new(
        "download",
        "Download a URL to a local file (binary-safe: images, PDFs, archives). 'path' must be an absolute host path. Sends a browser User-Agent and, by default, a Referer of the current tab's URL (needed by hot-link-protected hosts); override with 'referer'.",
        json!({
            "url": {"type": "string", "required": true},
            "path": {"type": "string", "required": true},
            "referer": {"type": "string", "description": "Referer header; defaults to the current tab URL."}
        }),
        r#"{"url": "https://example.com/pic.jpg", "path": "/abs/pic.jpg"}"#,
        |ctx| {
            let url = ctx.param_str("url")?;
            let path = ctx.param_str("path")?;
            if url.is_empty() {
                return Err(EngineError::invalid("'url' is required"));
            }
            if path.is_empty() {
                return Err(EngineError::invalid("'path' is required"));
            }

            // Referer: explicit param, else the page we are currently on (so
            // hot-link-protected images/PDFs load), else none.
            let mut referer = ctx.param_opt::<String>("referer")?.unwrap_or_default();
            if referer.is_empty() {
                if let Ok(tab) = ctx.target_tab() {
                    referer = ctx
                        .runtime
                        .engine()
                        .list_tabs()
                        .into_iter()
                        .find(|t| t.id == tab)
                        .map(|t| t.url)
                        .unwrap_or_default();
                }
            }

            if let Some(parent) = std::path::Path::new(&path).parent() {
                if !parent.as_os_str().is_empty() {
                    std::fs::create_dir_all(parent).map_err(|e| {
                        EngineError::new(
                            ErrorKind::Navigation,
                            format!("download mkdir {}: {e}", parent.display()),
                        )
                    })?;
                }
            }

            let mut req = ureq::get(&url)
                .set("User-Agent", BROWSER_UA)
                .set("Accept", "*/*");
            if !referer.is_empty() {
                req = req.set("Referer", &referer);
            }

            let resp = match req.call() {
                Ok(r) => r,
                Err(ureq::Error::Status(code, _)) => {
                    return Ok(json!({"ok": false, "status": code, "url": url}));
                }
                Err(e) => {
                    return Err(EngineError::new(ErrorKind::Navigation, format!("download: {e}")));
                }
            };
            let content_type = resp.header("content-type").unwrap_or("").to_string();

            let mut bytes: Vec<u8> = Vec::new();
            resp.into_reader()
                .read_to_end(&mut bytes)
                .map_err(|e| EngineError::new(ErrorKind::Navigation, format!("download: {e}")))?;
            std::fs::write(&path, &bytes).map_err(|e| {
                EngineError::new(ErrorKind::Navigation, format!("download write: {e}"))
            })?;

            Ok(json!({
                "ok": true,
                "path": path,
                "bytes": bytes.len(),
                "content_type": content_type,
            }))
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::Runtime;
    use crate::config::Config;
    use crate::tools::tool::ToolContext;
    use std::io::Write;

    #[test]
    fn download_writes_bytes_verbatim() {
        let payload: Vec<u8> = vec![0x89, b'P', b'N', b'G', 0x00, 0xFF, 0xFE, 0x01, 0x80];
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let body = payload.clone();
        std::thread::spawn(move || {
            if let Ok((mut sock, _)) = listener.accept() {
                let mut buf = [0u8; 1024];
                let _ = std::io::Read::read(&mut sock, &mut buf);
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = sock.write_all(header.as_bytes());
                let _ = sock.write_all(&body);
            }
        });

        let r = Runtime::new(
            Box::new(crate::engines::mock::MockEngine::new()),
            Config::default(),
        );
        let dir = std::env::temp_dir().join(format!("fb_dl_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("pic.png");
        let out = download()
            .run(&ToolContext {
                runtime: &r,
                params: json!({
                    "url": format!("http://127.0.0.1:{port}/pic.png"),
                    "path": path.to_string_lossy(),
                }),
            })
            .unwrap();
        assert_eq!(out["ok"], true, "{out}");
        assert_eq!(out["bytes"], 9);
        assert_eq!(
            std::fs::read(&path).unwrap(),
            vec![0x89, b'P', b'N', b'G', 0x00, 0xFF, 0xFE, 0x01, 0x80]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn download_sends_referer_and_browser_ua() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let seen2 = seen.clone();
        std::thread::spawn(move || {
            if let Ok((mut sock, _)) = listener.accept() {
                let mut buf = [0u8; 2048];
                let n = std::io::Read::read(&mut sock, &mut buf).unwrap_or(0);
                *seen2.lock().unwrap_or_else(|e| e.into_inner()) =
                    String::from_utf8_lossy(&buf[..n]).to_string();
                let _ = sock.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: image/jpeg\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                );
            }
        });

        let r = Runtime::new(
            Box::new(crate::engines::mock::MockEngine::new()),
            Config::default(),
        );
        let dir = std::env::temp_dir().join(format!("fb_dl_ref_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("pic.jpg");
        let out = download()
            .run(&ToolContext {
                runtime: &r,
                params: json!({
                    "url": format!("http://127.0.0.1:{port}/pic.jpg"),
                    "path": path.to_string_lossy(),
                    "referer": "https://ppbc.iplant.cn/",
                }),
            })
            .unwrap();
        assert_eq!(out["ok"], true, "{out}");
        let req = seen.lock().unwrap_or_else(|e| e.into_inner()).clone();
        assert!(
            req.contains("Referer: https://ppbc.iplant.cn/"),
            "request was: {req}"
        );
        assert!(req.contains("Mozilla/5.0"), "request was: {req}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
