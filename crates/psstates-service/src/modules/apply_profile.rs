//! Send a saved profile (plus fixed parameters) to VRChat.

use anyhow::{anyhow, Result};
use psstates_core::rules::{profile_messages, NameSet};

use crate::state::{read, Inner};

impl Inner {
    pub(crate) fn apply_profile(&self, name: &str) -> Result<String> {
        let avatar_id = self
            .current_avatar_id()
            .ok_or_else(|| anyhow!("No current avatar detected"))?;
        let settings = self.store.settings()?;
        let avatar = self.store.avatar(&avatar_id)?;
        let profile = avatar
            .find_profile(name)
            .ok_or_else(|| anyhow!("Profile '{name}' not found"))?;
        let excluded = NameSet::new(
            settings
                .excluded_parameters
                .iter()
                .chain(&avatar.excluded_parameters),
        );
        let msgs = profile_messages(profile, &excluded);
        let sent = self.sender.send_all(&msgs);
        let skipped = profile.parameters.len() - msgs.len();
        let fixed = self.apply_fixed(&avatar_id);

        let mut text = format!("Applied '{name}': {sent} parameter(s)");
        if skipped > 0 {
            text.push_str(&format!(", {skipped} excluded"));
        }
        if fixed > 0 {
            text.push_str(&format!(", {fixed} fixed"));
        }
        if !read(&self.status).connected {
            text.push_str(" (VRChat not connected)");
        }
        Ok(text)
    }
}
