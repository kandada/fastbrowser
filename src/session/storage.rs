// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Storage key model (localStorage / sessionStorage).

use serde::{Deserialize, Serialize};

/// Storage key: origin + key.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct StorageKey {
    pub origin: String,
    pub key: String,
}

impl StorageKey {
    pub fn new(origin: impl Into<String>, key: impl Into<String>) -> Self {
        StorageKey {
            origin: origin.into(),
            key: key.into(),
        }
    }

    /// Extract the origin from a URL.
    pub fn origin_of(url: &str) -> String {
        url::origin_str(url)
    }
}

mod url {
    /// Simplified extraction of scheme://host (including port).
    pub fn origin_str(url: &str) -> String {
        let rest = url.split("://").nth(1).unwrap_or(url);
        let host = rest.split('/').next().unwrap_or(rest);
        let scheme = if url.starts_with("https://") {
            "https"
        } else if url.starts_with("http://") {
            "http"
        } else {
            "unknown"
        };
        format!("{scheme}://{host}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_extraction() {
        assert_eq!(
            StorageKey::origin_of("https://example.com/a/b?q=1"),
            "https://example.com"
        );
        assert_eq!(
            StorageKey::origin_of("http://a.io:8080/x"),
            "http://a.io:8080"
        );
    }
}
