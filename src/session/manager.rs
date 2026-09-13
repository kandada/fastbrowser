// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Session manager: profile registry, active profile, tab ownership, visit history.

use std::collections::HashMap;

use crate::engine::{ContextId, EngineError, ErrorKind, Result, TabId};
use crate::session::profile::{Profile, ProfileId};

/// Session manager.
pub struct SessionManager {
    profiles: HashMap<ProfileId, Profile>,
    next_profile: u32,
    active_profile: Option<ProfileId>,
    tab_profiles: HashMap<TabId, ProfileId>,
    history: HashMap<ProfileId, Vec<String>>,
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionManager {
    pub fn new() -> Self {
        SessionManager {
            profiles: HashMap::new(),
            next_profile: 1,
            active_profile: None,
            tab_profiles: HashMap::new(),
            history: HashMap::new(),
        }
    }

    /// Ensure a default profile exists (create it if missing).
    pub fn ensure_default(&mut self) -> ProfileId {
        if self.active_profile.is_none() {
            self.active_profile =
                Some(self.create_profile("default", "mock", false, None, None, None));
        }
        self.active_profile.unwrap()
    }

    pub fn create_profile(
        &mut self,
        name: &str,
        engine: &str,
        incognito: bool,
        storage_path: Option<String>,
        user_agent: Option<String>,
        context: Option<ContextId>,
    ) -> ProfileId {
        let id = ProfileId(self.next_profile);
        self.next_profile += 1;
        let profile = Profile {
            id,
            name: name.to_string(),
            engine: engine.to_string(),
            incognito,
            storage_path,
            user_agent,
            created_ms: 0,
            context,
        };
        self.profiles.insert(id, profile);
        self.history.insert(id, Vec::new());
        id
    }

    /// Set a profile's isolated context.
    pub fn set_profile_context(&mut self, id: ProfileId, context: Option<ContextId>) -> Result<()> {
        let p = self.profiles.get_mut(&id).ok_or_else(|| {
            EngineError::new(
                ErrorKind::InvalidArgument,
                format!("profile {id:?} not found"),
            )
        })?;
        p.context = context;
        Ok(())
    }

    /// A profile's isolated context.
    pub fn profile_context(&self, id: ProfileId) -> Option<ContextId> {
        self.profiles.get(&id).and_then(|p| p.context)
    }

    pub fn delete_profile(&mut self, id: ProfileId) -> Result<()> {
        self.profiles.remove(&id).ok_or_else(|| {
            EngineError::new(
                ErrorKind::InvalidArgument,
                format!("profile {id:?} not found"),
            )
        })?;
        self.history.remove(&id);
        self.tab_profiles.retain(|_, p| *p != id);
        if self.active_profile == Some(id) {
            self.active_profile = self.profiles.keys().next().copied();
        }
        Ok(())
    }

    pub fn get_profile(&self, id: ProfileId) -> Option<&Profile> {
        self.profiles.get(&id)
    }

    pub fn list_profiles(&self) -> Vec<Profile> {
        self.profiles.values().cloned().collect()
    }

    pub fn set_active_profile(&mut self, id: ProfileId) -> Result<()> {
        if !self.profiles.contains_key(&id) {
            return Err(EngineError::new(
                ErrorKind::InvalidArgument,
                format!("profile {id:?} not found"),
            ));
        }
        self.active_profile = Some(id);
        Ok(())
    }

    pub fn active_profile(&self) -> Option<ProfileId> {
        self.active_profile
    }

    /// Find a profile by name.
    pub fn profile_id_by_name(&self, name: &str) -> Option<ProfileId> {
        self.profiles
            .iter()
            .find(|(_, p)| p.name == name)
            .map(|(id, _)| *id)
    }

    pub fn bind_tab(&mut self, tab: TabId, profile: ProfileId) {
        self.tab_profiles.insert(tab, profile);
    }

    pub fn unbind_tab(&mut self, tab: TabId) {
        self.tab_profiles.remove(&tab);
    }

    pub fn tab_profile(&self, tab: TabId) -> Option<ProfileId> {
        self.tab_profiles.get(&tab).copied()
    }

    /// Record a visit (used by the "recently visited" capability).
    pub fn record_visit(&mut self, profile: ProfileId, url: &str) {
        if let Some(h) = self.history.get_mut(&profile) {
            h.push(url.to_string());
        }
    }

    pub fn visit_history(&self, profile: ProfileId) -> Vec<String> {
        self.history.get(&profile).cloned().unwrap_or_default()
    }

    pub fn profile_count(&self) -> usize {
        self.profiles.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_lifecycle_and_isolation() {
        let mut sm = SessionManager::new();
        let def = sm.ensure_default();
        assert_eq!(def, ProfileId(1));
        let work = sm.create_profile("work", "mock", true, None, None, None);
        let life = sm.create_profile("life", "mock", false, None, None, None);
        assert_eq!(sm.profile_count(), 3);

        sm.set_active_profile(work).unwrap();
        assert_eq!(sm.active_profile(), Some(work));

        sm.bind_tab(TabId(10), work);
        sm.bind_tab(TabId(11), life);
        assert_eq!(sm.tab_profile(TabId(10)), Some(work));
        assert_eq!(sm.tab_profile(TabId(11)), Some(life));

        sm.record_visit(work, "https://work.example.com");
        sm.record_visit(work, "https://work.example.com/2");
        sm.record_visit(life, "https://life.example.com");
        // Each profile's history is isolated.
        assert_eq!(sm.visit_history(work).len(), 2);
        assert_eq!(sm.visit_history(life).len(), 1);
    }

    #[test]
    fn delete_profile_unbinds_and_switches_active() {
        let mut sm = SessionManager::new();
        let a = sm.create_profile("a", "mock", false, None, None, None);
        let b = sm.create_profile("b", "mock", false, None, None, None);
        sm.set_active_profile(a).unwrap();
        sm.bind_tab(TabId(1), a);
        sm.delete_profile(a).unwrap();
        assert_eq!(sm.active_profile(), Some(b));
        assert_eq!(sm.tab_profile(TabId(1)), None);
        assert!(sm.delete_profile(ProfileId(999)).is_err());
    }
}
