//! Copy all profiles from one avatar to another.

use anyhow::{anyhow, bail, Result};

use crate::state::{read, Inner};

impl Inner {
    /// Copies the profiles of the previous avatar to the current one.
    pub(crate) fn copy_from_previous(&self) -> Result<String> {
        let (current, previous) = {
            let st = read(&self.status);
            (st.current_avatar.clone(), st.previous_avatar.clone())
        };
        let current = current.ok_or_else(|| anyhow!("No current avatar detected"))?;
        let previous = previous.ok_or_else(|| anyhow!("There is no previous avatar"))?;
        self.copy_profiles(&previous.id, &current.id)
    }

    /// Copies the profiles of `source_avatar` to the current avatar.
    pub(crate) fn copy_from_avatar(&self, source_avatar: &str) -> Result<String> {
        let current = self
            .current_avatar_id()
            .ok_or_else(|| anyhow!("No current avatar detected"))?;
        self.copy_profiles(source_avatar, &current)
    }

    fn copy_profiles(&self, from: &str, to: &str) -> Result<String> {
        if from == to {
            bail!("Source and target avatar are the same");
        }
        let source = self.store.avatar(from)?;
        if source.profiles.is_empty() {
            bail!("Selected avatar has no saved profiles");
        }
        let copied = self
            .store
            .update_avatar(to, |a| Ok(a.copy_profiles_from(&source)))?;
        self.bump_revision();
        Ok(format!(
            "Copied {copied} profile(s) from {}",
            source.display()
        ))
    }
}
