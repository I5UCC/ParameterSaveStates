//! Core data model, persistence and business rules for ParameterSaveStates.
//!
//! This crate is deliberately free of networking and UI code so that it can be
//! unit tested in isolation and shared between the desktop UI, the optional VR
//! overlay and any future HTTP API.

pub mod backup;
pub mod error;
pub mod legacy;
pub mod model;
pub mod paths;
pub mod rules;
pub mod store;
pub mod util;

pub use error::{CoreError, Result};
pub use model::{
    AvatarData, AvatarSummary, FixedParam, MoveDirection, ParamType, ParamValue, Profile, Settings,
    Theme,
};
pub use store::Store;
