// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Session persistence (mirrors browser-use's `storage_state`).
//!
//! Serialize a tab's cookies + localStorage to a JSON file for cross-task/process restore.

use serde::{Deserialize, Serialize};

use crate::engine::{BrowserEngine, Cookie, EngineError, Result, TabId};
use crate::session::Profile;

/// Session snapshot of a single tab.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TabSessionState {
    pub url: String,
    pub title: String,
    pub cookies: Vec<Cookie>,
    pub storage: std::collections::HashMap<String, String>,
}

/// Full session of a profile.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionState {
    pub version: u32,
    pub profile: Option<Profile>,
    pub tabs: Vec<TabSessionState>,
}

impl SessionState {
    pub fn new() -> Self {
        SessionState {
            version: 1,
            profile: None,
            tabs: Vec::new(),
        }
    }

    /// Export the state of one engine tab.
    pub fn capture(engine: &dyn BrowserEngine, tab: TabId, title: &str) -> Result<TabSessionState> {
        let cookies = engine.cookie_get(tab, None)?;
        let storage = engine.storage_all(tab)?;
        let url = engine.snapshot(tab).map(|s| s.url).unwrap_or_default();
        Ok(TabSessionState {
            url,
            title: title.to_string(),
            cookies,
            storage,
        })
    }

    /// Write session state to a JSON file.
    pub fn save(state: &SessionState, path: &str) -> Result<()> {
        let data = serde_json::to_vec_pretty(state).map_err(EngineError::from_serde)?;
        if let Some(parent) = std::path::Path::new(path).parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, data)?;
        Ok(())
    }

    /// Read session state from a JSON file.
    pub fn load(path: &str) -> Result<SessionState> {
        let data = std::fs::read(path)?;
        serde_json::from_slice(&data).map_err(EngineError::from_serde)
    }

    /// Apply the state back to an engine tab (restore cookies + storage).
    pub fn apply(&self, engine: &dyn BrowserEngine, tab: TabId) -> Result<()> {
        if let Some(state) = self.tabs.first() {
            for cookie in &state.cookies {
                engine.cookie_set(tab, cookie)?;
            }
            for (k, v) in &state.storage {
                engine.storage_set(tab, k, v)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::engine::TabOptions;
    use crate::engines::mock::MockEngine;

    #[test]
    fn capture_save_load_apply_roundtrip() {
        let e = MockEngine::new();
        let tab = e
            .create_tab("https://example.com", &TabOptions::default())
            .unwrap();
        e.cookie_set(tab, &Cookie::new("sid", "abc", "example.com"))
            .unwrap();
        e.storage_set(tab, "token", "t1").unwrap();

        let ts = TabSessionState {
            url: "https://example.com".into(),
            title: "t".into(),
            cookies: e.cookie_get(tab, None).unwrap(),
            storage: e.storage_all(tab).unwrap(),
        };
        let mut full = SessionState::new();
        full.tabs.push(ts);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        SessionState::save(&full, path.to_str().unwrap()).unwrap();
        let loaded = SessionState::load(path.to_str().unwrap()).unwrap();
        assert_eq!(loaded.tabs[0].cookies[0].value, "abc");
        assert_eq!(loaded.tabs[0].storage["token"], "t1");

        // Restore into the new engine.
        let e2 = MockEngine::new();
        let tab2 = e2
            .create_tab("https://example.com", &TabOptions::default())
            .unwrap();
        loaded.apply(&e2, tab2).unwrap();
        assert_eq!(e2.cookie_get(tab2, None).unwrap()[0].value, "abc");
        assert_eq!(
            e2.storage_get(tab2, "token").unwrap().as_deref(),
            Some("t1")
        );
    }

    #[test]
    fn config_includes_browser_use_fields() {
        let c = Config::default();
        assert_eq!(c.locale, None);
        assert!(!c.accept_downloads);
        assert_eq!(c.extra_browser_args, Vec::<String>::new());
    }
}
