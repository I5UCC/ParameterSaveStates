//! Profiles: named snapshots of an avatar's parameter values.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::parameter::ParamValue;
use crate::error::{CoreError, Result};
use crate::util::unix_now;

/// Maximum length (in characters) of a profile name.
pub const MAX_PROFILE_NAME_LEN: usize = 128;

/// A saved set of avatar parameter values, keyed by full OSC address.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<u64>,
    #[serde(default)]
    pub parameters: BTreeMap<String, ParamValue>,
}

impl Profile {
    pub fn new(name: impl Into<String>, parameters: BTreeMap<String, ParamValue>) -> Self {
        let now = unix_now();
        Self {
            name: name.into(),
            created_at: Some(now),
            updated_at: Some(now),
            parameters,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveDirection {
    Up,
    Down,
}

/// Validates and normalises a profile name.
pub fn validate_profile_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(CoreError::invalid("Profile name is required"));
    }
    if name.chars().count() > MAX_PROFILE_NAME_LEN {
        return Err(CoreError::invalid(format!(
            "Profile name must be at most {MAX_PROFILE_NAME_LEN} characters"
        )));
    }
    if name.chars().any(char::is_control) {
        return Err(CoreError::invalid(
            "Profile name must not contain control characters",
        ));
    }
    Ok(name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_name_validation() {
        assert!(validate_profile_name("").is_err());
        assert!(validate_profile_name("a\nb").is_err());
        assert!(validate_profile_name(&"x".repeat(200)).is_err());
        // Characters that were invalid as Windows file names are fine now.
        assert_eq!(validate_profile_name(" a/b:c* ").unwrap(), "a/b:c*");
    }
}
