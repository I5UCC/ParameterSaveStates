//! Cached view of the store, reloaded when the service revision changes.

use std::collections::HashMap;

use psstates_core::{AvatarData, AvatarSummary, Settings, Store};
use psstates_service::Status;

#[derive(Default)]
pub struct DataCache {
    loaded_revision: Option<u64>,
    pub settings: Settings,
    /// Stored avatars plus the current avatar (even if nothing is stored yet).
    pub avatars: Vec<AvatarSummary>,
    avatar_data: HashMap<String, AvatarData>,
    pub error: Option<String>,
}

impl DataCache {
    pub fn invalidate(&mut self) {
        self.loaded_revision = None;
    }

    /// Reloads if the revision changed or the cache was invalidated.
    pub fn refresh(&mut self, store: &Store, status: &Status) {
        if self.loaded_revision == Some(status.revision) {
            return;
        }
        self.loaded_revision = Some(status.revision);
        self.avatar_data.clear();
        self.error = None;

        match store.settings() {
            Ok(s) => self.settings = s,
            Err(e) => self.error = Some(e.to_string()),
        }
        match store.list_avatars() {
            Ok(list) => self.avatars = list,
            Err(e) => {
                self.avatars.clear();
                self.error = Some(e.to_string());
            }
        }
        if let Some(cur) = &status.current_avatar {
            if !self.avatars.iter().any(|a| a.id == cur.id) {
                self.avatars.push(AvatarSummary {
                    id: cur.id.clone(),
                    name: cur.name.clone(),
                    profile_count: 0,
                });
            }
        }
    }

    /// Loads (and caches) one avatar.
    pub fn avatar(&mut self, store: &Store, id: &str) -> &AvatarData {
        if !self.avatar_data.contains_key(id) {
            let data = store.avatar(id).unwrap_or_else(|e| {
                self.error = Some(e.to_string());
                AvatarData::new(id)
            });
            self.avatar_data.insert(id.to_string(), data);
        }
        &self.avatar_data[id]
    }

    pub fn avatars_with_profiles(&self) -> Vec<AvatarSummary> {
        self.avatars
            .iter()
            .filter(|a| a.profile_count > 0)
            .cloned()
            .collect()
    }
}
