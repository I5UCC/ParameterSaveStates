//! Everything stored for one avatar, and the operations on its profiles.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::fixed_parameter::FixedParam;
use super::parameter::ParamValue;
use super::profile::{validate_profile_name, MoveDirection, Profile};
use super::FORMAT_VERSION;
use crate::error::{CoreError, Result};
use crate::util::unix_now;

/// Everything stored for one avatar (`avatars/<id>.json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AvatarData {
    pub format_version: u32,
    pub id: String,
    /// Last known display name of the avatar.
    pub name: Option<String>,
    /// Ordered list of profiles.
    pub profiles: Vec<Profile>,
    pub excluded_parameters: Vec<String>,
    pub auto_sync_parameters: Vec<String>,
    pub fixed_parameters: Vec<FixedParam>,
}

impl Default for AvatarData {
    fn default() -> Self {
        Self::new("")
    }
}

/// Lightweight avatar listing entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvatarSummary {
    pub id: String,
    pub name: Option<String>,
    pub profile_count: usize,
}

impl AvatarSummary {
    pub fn display(&self) -> String {
        display_avatar(&self.id, self.name.as_deref())
    }
}

/// Formats an avatar for display: `Name (avtr_...)` or just the id.
pub fn display_avatar(id: &str, name: Option<&str>) -> String {
    match name.map(str::trim).filter(|n| !n.is_empty()) {
        Some(name) => format!("{name} ({id})"),
        None => id.to_string(),
    }
}

fn same_name(a: &str, b: &str) -> bool {
    a == b || a.to_lowercase() == b.to_lowercase()
}

impl AvatarData {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            format_version: FORMAT_VERSION,
            id: id.into(),
            name: None,
            profiles: Vec::new(),
            excluded_parameters: Vec::new(),
            auto_sync_parameters: Vec::new(),
            fixed_parameters: Vec::new(),
        }
    }

    pub fn summary(&self) -> AvatarSummary {
        AvatarSummary {
            id: self.id.clone(),
            name: self.name.clone(),
            profile_count: self.profiles.len(),
        }
    }

    pub fn display(&self) -> String {
        display_avatar(&self.id, self.name.as_deref())
    }

    /// `true` when nothing worth persisting is stored for this avatar.
    pub fn is_empty(&self) -> bool {
        self.profiles.is_empty()
            && self.excluded_parameters.is_empty()
            && self.auto_sync_parameters.is_empty()
            && self.fixed_parameters.is_empty()
    }

    pub fn profile_names(&self) -> Vec<String> {
        self.profiles.iter().map(|p| p.name.clone()).collect()
    }

    /// Exact (case-sensitive) lookup.
    pub fn find_profile(&self, name: &str) -> Option<&Profile> {
        self.profiles.iter().find(|p| p.name == name)
    }

    fn index_of(&self, name: &str) -> Result<usize> {
        self.profiles
            .iter()
            .position(|p| p.name == name)
            .ok_or_else(|| CoreError::ProfileNotFound(name.to_string()))
    }

    /// Whether a profile with this name exists (case-insensitive), optionally
    /// ignoring the profile at `except`.
    fn name_taken(&self, name: &str, except: Option<usize>) -> bool {
        self.profiles
            .iter()
            .enumerate()
            .any(|(i, p)| Some(i) != except && same_name(&p.name, name))
    }

    /// Returns a unique name with the given base, appending ` (2)`, ` (3)`, ... if needed.
    pub fn unique_name(&self, base: &str) -> String {
        if !self.name_taken(base, None) {
            return base.to_string();
        }
        (2..)
            .map(|n| format!("{base} ({n})"))
            .find(|candidate| !self.name_taken(candidate, None))
            .expect("infinite iterator")
    }

    /// Default name for a new profile (`Profile N`).
    pub fn next_default_name(&self) -> String {
        (self.profiles.len() + 1..)
            .map(|n| format!("Profile {n}"))
            .find(|candidate| !self.name_taken(candidate, None))
            .expect("infinite iterator")
    }

    /// Adds a new profile at the end of the list. An empty/missing name uses
    /// [`AvatarData::next_default_name`]. Returns the final name.
    pub fn add_profile(
        &mut self,
        name: Option<&str>,
        parameters: BTreeMap<String, ParamValue>,
    ) -> Result<String> {
        let name = match name.map(str::trim).filter(|n| !n.is_empty()) {
            Some(name) => validate_profile_name(name)?,
            None => self.next_default_name(),
        };
        if self.name_taken(&name, None) {
            return Err(CoreError::ProfileExists(name));
        }
        self.profiles.push(Profile::new(name.clone(), parameters));
        Ok(name)
    }

    /// Replaces the parameters of an existing profile.
    pub fn override_profile(
        &mut self,
        name: &str,
        parameters: BTreeMap<String, ParamValue>,
    ) -> Result<()> {
        let idx = self.index_of(name)?;
        let profile = &mut self.profiles[idx];
        profile.parameters = parameters;
        profile.updated_at = Some(unix_now());
        Ok(())
    }

    pub fn rename_profile(&mut self, name: &str, new_name: &str) -> Result<String> {
        let idx = self.index_of(name)?;
        let new_name = validate_profile_name(new_name)?;
        if self.name_taken(&new_name, Some(idx)) {
            return Err(CoreError::ProfileExists(new_name));
        }
        self.profiles[idx].name = new_name.clone();
        Ok(new_name)
    }

    pub fn delete_profile(&mut self, name: &str) -> Result<Profile> {
        let idx = self.index_of(name)?;
        Ok(self.profiles.remove(idx))
    }

    pub fn move_profile(&mut self, name: &str, direction: MoveDirection) -> Result<()> {
        let idx = self.index_of(name)?;
        let target = match direction {
            MoveDirection::Up if idx > 0 => idx - 1,
            MoveDirection::Down if idx + 1 < self.profiles.len() => idx + 1,
            _ => return Err(CoreError::invalid("Unable to move profile any further")),
        };
        self.profiles.swap(idx, target);
        Ok(())
    }

    /// Copies all profiles of `source` into this avatar. Profiles with the same
    /// name (case-insensitive) are overwritten in place, others are appended.
    /// Returns the number of copied profiles.
    pub fn copy_profiles_from(&mut self, source: &AvatarData) -> usize {
        for profile in &source.profiles {
            match self
                .profiles
                .iter_mut()
                .find(|p| same_name(&p.name, &profile.name))
            {
                Some(existing) => {
                    existing.parameters = profile.parameters.clone();
                    existing.updated_at = Some(unix_now());
                }
                None => self.profiles.push(profile.clone()),
            }
        }
        source.profiles.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> BTreeMap<String, ParamValue> {
        BTreeMap::from([("/avatar/parameters/A".to_string(), ParamValue::Float(0.5))])
    }

    #[test]
    fn add_and_default_names() {
        let mut a = AvatarData::new("avtr_1");
        assert_eq!(a.add_profile(None, params()).unwrap(), "Profile 1");
        assert_eq!(a.add_profile(Some("  "), params()).unwrap(), "Profile 2");
        assert_eq!(a.add_profile(Some("Outfit"), params()).unwrap(), "Outfit");
        assert!(matches!(
            a.add_profile(Some("outfit"), params()),
            Err(CoreError::ProfileExists(_))
        ));
        a.delete_profile("Profile 1").unwrap();
        // len is 2 -> "Profile 3" would be next; "Profile 3" is free.
        assert_eq!(a.next_default_name(), "Profile 3");
    }

    #[test]
    fn rename_collision_is_reported() {
        let mut a = AvatarData::new("avtr_1");
        a.add_profile(Some("A"), params()).unwrap();
        a.add_profile(Some("B"), params()).unwrap();
        assert!(matches!(
            a.rename_profile("A", "b"),
            Err(CoreError::ProfileExists(_))
        ));
        assert!(matches!(
            a.rename_profile("X", "Y"),
            Err(CoreError::ProfileNotFound(_))
        ));
        // Changing only the case of the own name is allowed.
        assert_eq!(a.rename_profile("A", "a").unwrap(), "a");
    }

    #[test]
    fn move_profiles() {
        let mut a = AvatarData::new("avtr_1");
        for n in ["A", "B", "C"] {
            a.add_profile(Some(n), params()).unwrap();
        }
        a.move_profile("C", MoveDirection::Up).unwrap();
        assert_eq!(a.profile_names(), ["A", "C", "B"]);
        assert!(a.move_profile("A", MoveDirection::Up).is_err());
        assert!(a.move_profile("B", MoveDirection::Down).is_err());
    }

    #[test]
    fn copy_overwrites_same_names() {
        let mut src = AvatarData::new("src");
        src.add_profile(Some("A"), params()).unwrap();
        src.add_profile(Some("New"), BTreeMap::new()).unwrap();
        let mut dst = AvatarData::new("dst");
        dst.add_profile(Some("a"), BTreeMap::new()).unwrap();
        assert_eq!(dst.copy_profiles_from(&src), 2);
        assert_eq!(dst.profile_names(), ["a", "New"]);
        assert_eq!(dst.profiles[0].parameters, params());
    }
}
