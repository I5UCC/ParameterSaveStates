//! Track which avatar VRChat has loaded and react when it changes.

use std::sync::Arc;
use std::time::Duration;

use psstates_osc::osc::IncomingMessage;

use crate::state::{lock, write, AvatarRef, Inner};

const AVATAR_CHANGE: &str = "/avatar/change";
const AVATAR_PARAMETERS: &str = "/avatar/parameters/";

impl Inner {
    pub(crate) fn handle_osc(self: &Arc<Self>, msg: IncomingMessage) {
        if msg.addr == AVATAR_CHANGE {
            if let Some(id) = msg.first_string().filter(|s| !s.is_empty()) {
                self.on_avatar_change(id.to_string(), false);
            }
        } else if msg.addr.starts_with(AVATAR_PARAMETERS) {
            if let Some(v) = msg.param_value() {
                lock(&self.cache).current.insert(msg.addr, v);
            }
        }
    }

    /// Handles an avatar change. With `force`, fixed parameters are re-applied
    /// even if the avatar did not change (e.g. after reconnecting).
    pub(crate) fn on_avatar_change(self: &Arc<Self>, id: String, force: bool) {
        let changed = {
            let mut st = write(&self.status);
            let same = st.current_avatar.as_ref().is_some_and(|a| a.id == id);
            if same && !force {
                return;
            }
            if !same {
                let name = self.store.avatar(&id).ok().and_then(|a| a.name);
                st.previous_avatar = st.current_avatar.take();
                st.current_avatar = Some(AvatarRef {
                    id: id.clone(),
                    name,
                });
                st.revision += 1;
            }
            !same
        };
        tracing::info!("Avatar changed to {id}");

        if changed {
            let mut cache = lock(&self.cache);
            // Keep the older snapshot if nothing was received for the avatar
            // in between (rapid switching).
            if !cache.current.is_empty() {
                cache.previous = std::mem::take(&mut cache.current);
            }
        }
        if changed {
            self.apply_auto_sync(&id);
        }
        self.apply_fixed(&id);
        self.notify();

        let me = self.clone();
        self.rt
            .spawn(async move { me.resolve_avatar_name(id).await });
    }

    async fn resolve_avatar_name(self: Arc<Self>, id: String) {
        // Give VRChat a moment to update its OSCQuery tree.
        tokio::time::sleep(Duration::from_millis(300)).await;
        let Some(client) = self.client() else { return };
        let name = match client.current_avatar_name().await {
            Ok(Some(name)) => name,
            Ok(None) => return,
            Err(e) => {
                tracing::debug!("Could not read avatar name: {e:#}");
                return;
            }
        };
        if let Err(e) = self.store.set_avatar_name(&id, &name) {
            tracing::warn!("Failed to store avatar name: {e}");
        }
        {
            let mut st = write(&self.status);
            match st.current_avatar.as_mut() {
                Some(a) if a.id == id => a.name = Some(name),
                _ => return,
            }
            st.revision += 1;
        }
        self.notify();
    }
}
