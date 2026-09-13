// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Cookie 模型（引擎层定义，session 层复用于持久化与工具）。

use serde::{Deserialize, Serialize};

/// 浏览器 Cookie。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    /// 过期时间（Unix 秒）；None 表示会话 Cookie。
    pub expires: Option<i64>,
    pub secure: bool,
    pub http_only: bool,
    pub same_site: Option<String>,
}

impl Cookie {
    pub fn new(
        name: impl Into<String>,
        value: impl Into<String>,
        domain: impl Into<String>,
    ) -> Self {
        Cookie {
            name: name.into(),
            value: value.into(),
            domain: domain.into(),
            path: "/".into(),
            expires: None,
            secure: false,
            http_only: false,
            same_site: None,
        }
    }

    /// 解析 Set-Cookie 头。
    pub fn from_set_cookie(header: &str) -> Option<Cookie> {
        let mut parts = header.split(';').map(|s| s.trim());
        let kv = parts.next()?.split_once('=')?;
        let mut c = Cookie::new(kv.0.trim(), kv.1.trim(), String::new());
        for attr in parts {
            let (k, v) = attr.split_once('=').unwrap_or((attr, ""));
            let k = k.trim().to_ascii_lowercase();
            let v = v.trim().to_string();
            match k.as_str() {
                "domain" => c.domain = v,
                "path" => c.path = v,
                "expires" => {
                    // 简化：接受 unix 秒或 RFC 失败则忽略
                    if let Ok(secs) = v.parse::<i64>() {
                        c.expires = Some(secs);
                    } else {
                        c.expires = None;
                    }
                }
                "secure" => c.secure = true,
                "httponly" => c.http_only = true,
                "samesite" => c.same_site = Some(v),
                _ => {}
            }
        }
        Some(c)
    }

    /// 生成 Cookie 请求头片段（name=value; ...）。
    pub fn to_request_header(&self) -> String {
        format!("{}={}", self.name, self.value)
    }

    /// 判断是否匹配指定 host（忽略端口与域名前导点）与路径。
    pub fn matches(&self, host: &str, path: &str) -> bool {
        let host_no_port = host.split(':').next().unwrap_or(host);
        let domain = self.domain.trim_start_matches('.');
        let host_ok = host_no_port == domain || host_no_port.ends_with(&format!(".{domain}"));
        let path_ok = path.starts_with(&self.path);
        host_ok && path_ok
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_cookie_parse_basic() {
        let c =
            Cookie::from_set_cookie("session=abc; Domain=.example.com; Path=/; Secure; HttpOnly")
                .unwrap();
        assert_eq!(c.name, "session");
        assert_eq!(c.value, "abc");
        assert_eq!(c.domain, ".example.com");
        assert!(c.secure);
        assert!(c.http_only);
        assert!(c.expires.is_none());
    }

    #[test]
    fn cookie_matches_domain_and_path() {
        let c = Cookie::new("k", "v", "example.com");
        assert!(c.matches("www.example.com", "/"));
        assert!(c.matches("example.com", "/foo"));
        assert!(!c.matches("example.org", "/"));
        let sub = Cookie {
            path: "/a".into(),
            ..c
        };
        assert!(sub.matches("example.com", "/a/b"));
        assert!(!sub.matches("example.com", "/b"));
    }

    #[test]
    fn request_header() {
        assert_eq!(Cookie::new("k", "v", "d").to_request_header(), "k=v");
    }

    #[test]
    fn set_cookie_parse_edge_cases() {
        // Expires 为 RFC 日期（无法解析为 unix 秒）→ expires=None，不崩溃
        let c = Cookie::from_set_cookie(
            "a=b; Expires=Wed, 21 Oct 2025 07:28:00 GMT; Max-Age=3600; SameSite=Lax",
        )
        .unwrap();
        assert!(c.expires.is_none());
        assert_eq!(c.same_site.as_deref(), Some("Lax"));
        // SameSite=Strict 大小写
        let c = Cookie::from_set_cookie("c=d; samesite=Strict").unwrap();
        assert_eq!(c.same_site.as_deref(), Some("Strict"));
        // 无分号的最小格式
        let c = Cookie::from_set_cookie("k=v").unwrap();
        assert_eq!(c.name, "k");
        assert_eq!(c.value, "v");
        assert_eq!(c.path, "/");
        // 非法头 → None
        assert!(Cookie::from_set_cookie("").is_none());
        assert!(Cookie::from_set_cookie("no-equals").is_none());
        // 数字 Expires（unix 秒）可解析
        let c = Cookie::from_set_cookie("s=1; Expires=1700000000").unwrap();
        assert_eq!(c.expires, Some(1700000000));
    }

    #[test]
    fn cookie_matches_ports_and_paths() {
        let c = Cookie::new("k", "v", "example.com");
        assert!(c.matches("example.com:8080", "/"));
        assert!(c.matches("sub.example.com", "/"));
        assert!(!c.matches("notexample.com", "/"));
        // 子域 cookie 不匹配父域之外
        let sc = Cookie {
            domain: ".example.com".into(),
            ..c
        };
        assert!(sc.matches("a.example.com", "/"));
    }
}
