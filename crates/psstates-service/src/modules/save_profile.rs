//! Save the current avatar state as a new profile, or override an existing one.

use std::collections::BTreeMap;

use anyhow::{anyhow, bail, Context, Result};
use psstates_core::ParamValue;

use crate::state::Inner;

impl Inner {
    pub(crate) async fn save_profile(
        &self,
        name: Option<String>,
        override_name: Option<String>,
    ) -> Result<String> {
        let avatar_id = self
            .current_avatar_id()
            .ok_or_else(|| anyhow!("No current avatar detected"))?;
        let client = self
            .client()
            .ok_or_else(|| anyhow!("Unable to save profile. Is VRChat connected?"))?;

        self.set_busy(Some("Saving profile..."));
        let params: Result<BTreeMap<String, ParamValue>> = client.avatar_parameters().await;
        self.set_busy(None);
        let params = params.context("Unable to read avatar parameters from VRChat")?;
        if params.is_empty() {
            bail!("VRChat reported no avatar parameters");
        }
        let count = params.len();

        let msg = match override_name {
            Some(target) => {
                self.store
                    .update_avatar(&avatar_id, |a| a.override_profile(&target, params))?;
                format!("Overrode profile '{target}' ({count} parameters)")
            }
            None => {
                let final_name = self
                    .store
                    .update_avatar(&avatar_id, |a| a.add_profile(name.as_deref(), params))?;
                format!("Saved profile '{final_name}' ({count} parameters)")
            }
        };
        self.bump_revision();
        Ok(msg)
    }
}
