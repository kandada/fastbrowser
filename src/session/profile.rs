// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Profiles — multi-account isolation.

use serde::{Deserialize, Serialize};

use crate::engine::ContextId;

/// Profile ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ProfileId(pub u32);

/// A browser profile (work / personal / private, …).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub id: ProfileId,
    pub name: String,
    pub engine: String,
    pub incognito: bool,
    pub storage_path: Option<String>,
    pub user_agent: Option<String>,
    pub created_ms: u64,
    /// The isolated browser context bound to this profile (a CDP BrowserContext).
    /// Stays None when the engine does not support it.
    pub context: Option<ContextId>,
}

impl Profile {
    pub fn new(id: ProfileId, name: impl Into<String>, engine: impl Into<String>) -> Self {
        Profile {
            id,
            name: name.into(),
            engine: engine.into(),
            incognito: false,
            storage_path: None,
            user_agent: None,
            created_ms: 0,
            context: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_serde() {
        let p = Profile::new(ProfileId(1), "work", "mock");
        let json = serde_json::to_string(&p).unwrap();
        let back: Profile = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, p.id);
        assert_eq!(back.name, "work");
    }
}
