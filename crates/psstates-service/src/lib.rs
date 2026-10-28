//! The application service ties VRChat (OSC / OSCQuery / mDNS) to the profile
//! store. It is UI agnostic: the desktop UI, the VR overlay and a possible
//! HTTP API all drive the same [`Service`].
//!
//! * Read state via [`Service::status`] (cheap clone).
//! * Fire-and-forget operations (`save_profile`, `apply_profile`, ...) run on
//!   the tokio runtime and report results as [`Notice`]s.
//! * Pure storage operations go directly through [`Service::store`].
//! * Register [`Service::add_on_change`] to get notified (e.g. to request a repaint).
//! * Each UI reads notices with its own [`NoticeCursor`].
//!
//! The code is organised by what the app does: [`modules`] has one module per
//! user-facing operation, `vrchat_connection` keeps the link to VRChat alive,
//! and `service` is the facade tying them together.

mod modules;
mod notices;
mod service;
mod state;
mod vrchat_connection;

#[cfg(test)]
mod tests;

pub use notices::{Notice, NoticeCursor, NoticeLevel};
pub use service::Service;
pub use state::{AvatarRef, Status};

/// Application version (overridable at build time via `PSSTATES_VERSION`).
pub const VERSION: &str = match option_env!("PSSTATES_VERSION") {
    Some(v) => v,
    None => env!("CARGO_PKG_VERSION"),
};
