//! Fixed parameters: always sent with the same value.

use serde::{Deserialize, Serialize};

use super::parameter::ParamValue;

/// A parameter that is always sent with a fixed value (after applying a
/// profile and after switching avatars).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FixedParam {
    /// Parameter name: full OSC path, path relative to `/avatar/parameters/`
    /// or a bare parameter name.
    pub name: String,
    pub value: ParamValue,
}

impl FixedParam {
    pub fn new(name: impl Into<String>, value: ParamValue) -> Self {
        Self {
            name: name.into(),
            value,
        }
    }
}
