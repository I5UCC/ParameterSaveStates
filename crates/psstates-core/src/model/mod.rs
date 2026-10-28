//! The domain model, one module per concept:
//!
//! * [`parameter`]: typed avatar parameter values
//! * [`profile`]: named snapshots of parameter values
//! * [`fixed_parameter`]: parameters forced to a value
//! * [`avatar`]: everything stored for one avatar, and its profile operations
//! * [`settings`]: global settings

pub mod avatar;
pub mod fixed_parameter;
pub mod parameter;
pub mod profile;
pub mod settings;

pub use avatar::{display_avatar, AvatarData, AvatarSummary};
pub use fixed_parameter::FixedParam;
pub use parameter::{ParamType, ParamValue};
pub use profile::{validate_profile_name, MoveDirection, Profile, MAX_PROFILE_NAME_LEN};
pub use settings::{Settings, Theme};

/// Current on-disk format version of [`AvatarData`] and [`Settings`].
pub const FORMAT_VERSION: u32 = 1;
