//! Global settings (`settings.json`).

use serde::{Deserialize, Serialize};

use super::fixed_parameter::FixedParam;
use super::FORMAT_VERSION;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    Dark,
    Light,
}

/// Global settings (`settings.json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub format_version: u32,
    pub theme: Theme,
    /// Parameter names never applied from profiles (all avatars).
    pub excluded_parameters: Vec<String>,
    /// Parameter names copied from the previous avatar on avatar change (all avatars).
    pub auto_sync_parameters: Vec<String>,
    /// Parameters with fixed values (all avatars).
    pub fixed_parameters: Vec<FixedParam>,
    /// Whether the user was already asked to import legacy (Unity version) data.
    pub legacy_import_prompted: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            format_version: FORMAT_VERSION,
            theme: Theme::Dark,
            excluded_parameters: Vec::new(),
            auto_sync_parameters: Vec::new(),
            fixed_parameters: Vec::new(),
            legacy_import_prompted: false,
        }
    }
}
