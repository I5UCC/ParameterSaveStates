//! Parameters that are always forced to a value (global plus per avatar).

use psstates_core::rules::{effective_fixed_parameters, fixed_messages};

use crate::state::Inner;

impl Inner {
    /// Sends the fixed parameters of `avatar_id`; returns how many were sent.
    pub(crate) fn apply_fixed(&self, avatar_id: &str) -> usize {
        let (Ok(settings), Ok(avatar)) = (self.store.settings(), self.store.avatar(avatar_id))
        else {
            return 0;
        };
        let fixed =
            effective_fixed_parameters(&settings.fixed_parameters, &avatar.fixed_parameters);
        if fixed.is_empty() {
            return 0;
        }
        let sent = self.sender.send_all(&fixed_messages(&fixed));
        tracing::info!(
            "Fixed parameters for avatar '{avatar_id}': {sent}/{} sent",
            fixed.len()
        );
        sent
    }
}
