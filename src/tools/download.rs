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

/// A modern desktop-browser UA. Many image/file hosts (and WAFs) reject
/// requests whose headers look non-browser.
const BROWSER_UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) \
     AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

pub fn tools() -> Vec<Tool> {
    vec![download()]
}

fn download() -> Tool {
    Tool::new(
        "download",
        "Download a URL to a local file (binary-safe: images, PDFs, archives). 'path' must be an absolute host path. Sends a full desktop-browser header set (UA/Accept/Sec-Fetch/sec-ch-ua) and, by default, a Referer of the current tab's URL (needed by hot-link-protected hosts / WAFs); override with 'referer', or add/override any header via the 'headers' object.",
        json!({
            "url": {"type": "string", "required": true},
            "path": {"type": "string", "required": true},
            "referer": {"type": "string", "description": "Referer header; defaults to the current tab URL."},
            "headers": {"type": "object", "description": "Extra/override request headers, e.g. {\"Accept-Language\": \"zh-CN\"}."}
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

            // Browser-like header defaults. `Accept` is only image-typed for
            // image URLs; a generic download (PDF/zip/JSON) keeps `*/*` so
            // non-image hosts are not content-negotiated away.
            let url_l = url.to_ascii_lowercase();
            let url_path = url_l.split(['?', '#']).next().unwrap_or("");
            let is_image = [
                ".png", ".jpg", ".jpeg", ".gif", ".webp", ".avif", ".bmp", ".ico", ".svg",
            ]
            .iter()
            .any(|e| url_path.ends_with(*e));
            let accept = if is_image {
                "image/avif,image/webp,image/apng,image/svg+xml,image/*,*/*;q=0.8"
            } else {
                "*/*"
            };
            let mut req = ureq::get(&url)
                .set("User-Agent", BROWSER_UA)
                .set("Accept", accept)
                .set("Accept-Language", "zh-CN,zh;q=0.9,en;q=0.8")
                .set("Sec-Fetch-Dest", if is_image { "image" } else { "empty" })
                .set("Sec-Fetch-Mode", "no-cors")
                .set("Sec-Fetch-Site", "cross-site")
                .set("sec-ch-ua", "\"Chromium\";v=\"131\", \"Not_A Brand\";v=\"24\"")
                .set("sec-ch-ua-mobile", "?0")
                .set("sec-ch-ua-platform", "\"macOS\"");
            if !referer.is_empty() {
                req = req.set("Referer", &referer);
            }
            // Extra/override headers (e.g. a host-specific cookie or token).
            // `Host`/`Content-Length` are managed by the HTTP client — allowing
            // them to be overridden can bypass vhost checks or corrupt framing.
            if let Some(obj) = ctx.params.get("headers").and_then(|v| v.as_object()) {
                for (k, v) in obj {
                    if k.eq_ignore_ascii_case("host") || k.eq_ignore_ascii_case("content-length") {
                        continue;
                    }
                    if let Some(vs) = v.as_str() {
                        req = req.set(k, vs);
                    }
                }
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
        // Full browser header set (WAFs reject requests missing Sec-Fetch/sec-ch-ua).
        let low = req.to_lowercase();
        assert!(low.contains("sec-fetch-dest: image"), "request was: {req}");
        assert!(low.contains("sec-ch-ua"), "request was: {req}");
        assert!(low.contains("accept-language"), "request was: {req}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn download_non_image_uses_generic_accept_and_ignores_host() {
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
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/pdf\r\nContent-Length: 2\r\nConnection: close\r\n\r\nOK",
                );
            }
        });

        let r = Runtime::new(
            Box::new(crate::engines::mock::MockEngine::new()),
            Config::default(),
        );
        let dir = std::env::temp_dir().join(format!("fb_dl_pdf_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("doc.pdf");
        let out = download()
            .run(&ToolContext {
                runtime: &r,
                params: json!({
                    "url": format!("http://127.0.0.1:{port}/doc.pdf"),
                    "path": path.to_string_lossy(),
                    "headers": {"Cookie": "a=1", "Host": "evil.example"}
                }),
            })
            .unwrap();
        assert_eq!(out["ok"], true, "{out}");
        let req = seen.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let low = req.to_lowercase();
        assert!(
            low.contains("accept: */*"),
            "non-image download must use a generic Accept: {req}"
        );
        assert!(low.contains("sec-fetch-dest: empty"), "{req}");
        assert!(
            !low.contains("evil.example"),
            "Host header override must be ignored: {req}"
        );
        assert!(
            low.contains("cookie: a=1"),
            "custom header should be sent: {req}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
