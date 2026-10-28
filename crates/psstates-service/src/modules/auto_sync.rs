//! Carry selected parameter values over from the previous avatar.

use psstates_core::rules::{select_auto_sync, NameSet};

use crate::state::{lock, Inner};

impl Inner {
    pub(crate) fn apply_auto_sync(&self, avatar_id: &str) {
        let (Ok(settings), Ok(avatar)) = (self.store.settings(), self.store.avatar(avatar_id))
        else {
            return;
        };
        let names = NameSet::new(
            settings
                .auto_sync_parameters
                .iter()
                .chain(&avatar.auto_sync_parameters),
        );
        if names.is_empty() {
            return;
        }
        let msgs = select_auto_sync(&lock(&self.cache).previous, &names);
        let sent = self.sender.send_all(&msgs);
        tracing::info!(
            "Auto-sync for avatar '{avatar_id}': {} configured name(s), {} matched, {sent} sent",
            names.len(),
            msgs.len()
        );
    }
}
