//! One module per thing a user can do. Each adds behaviour to
//! [`crate::state::Inner`]; the facade reports the outcome as a notice.

mod apply_profile;
mod auto_sync;
mod copy_profiles;
mod fixed_parameters;
mod follow_avatar;
mod save_profile;
