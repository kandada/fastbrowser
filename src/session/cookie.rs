// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Cookie-jar utilities (engine-agnostic, persistable).

use serde::{Deserialize, Serialize};

pub use crate::engine::Cookie;

/// A cookie collection that filters and groups by host/path.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CookieJar {
    pub cookies: Vec<Cookie>,
}

impl CookieJar {
    pub fn new() -> Self {
        CookieJar {
            cookies: Vec::new(),
        }
    }

    /// Cookies matching a request (rough host + path + secure check).
    pub fn for_request(&self, host: &str, path: &str, secure: bool) -> Vec<Cookie> {
        self.cookies
            .iter()
            .filter(|c| c.matches(host, path))
            .filter(|c| !c.secure || secure)
            .cloned()
            .collect()
    }

    pub fn insert(&mut self, cookie: Cookie) {
        self.cookies.retain(|c| {
            !(c.name == cookie.name && c.domain == cookie.domain && c.path == cookie.path)
        });
        self.cookies.push(cookie);
    }

    pub fn remove(&mut self, name: &str, domain: &str) {
        self.cookies
            .retain(|c| !(c.name == name && c.domain == domain));
    }

    pub fn clear(&mut self) {
        self.cookies.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.cookies.is_empty()
    }
}

/// Build a CookieJar from a set of Set-Cookie headers.
pub fn from_set_cookie_headers(headers: &[String]) -> CookieJar {
    let mut jar = CookieJar::new();
    for h in headers {
        if let Some(c) = Cookie::from_set_cookie(h) {
            jar.insert(c);
        }
    }
    jar
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jar_filters_by_host_path() {
        let mut jar = CookieJar::new();
        jar.insert(Cookie::new("a", "1", "example.com"));
        jar.insert(Cookie {
            name: "b".into(),
            value: "2".into(),
            domain: "example.com".into(),
            path: "/api".into(),
            ..Cookie::new("b", "2", "example.com")
        });
        let root = jar.for_request("www.example.com", "/", false);
        assert_eq!(root.len(), 1);
        assert_eq!(root[0].name, "a");
        let api = jar.for_request("example.com", "/api/v1", false);
        assert_eq!(api.len(), 2);
        jar.remove("a", "example.com");
        assert_eq!(jar.cookies.len(), 1);
    }

    #[test]
    fn parse_set_cookie_headers() {
        let jar = from_set_cookie_headers(&[
            "sid=abc; Domain=.example.com; Path=/".into(),
            "theme=dark".into(),
        ]);
        assert_eq!(jar.cookies.len(), 2);
    }
}
