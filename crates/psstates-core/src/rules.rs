//! Parameter matching, normalisation and "what to send" rules.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::error::{CoreError, Result};
use crate::model::{FixedParam, ParamValue, Profile};

pub const AVATAR_PARAMETERS_PREFIX: &str = "/avatar/parameters/";

/// Case-insensitive set of configured parameter names.
#[derive(Debug, Clone, Default)]
pub struct NameSet(HashSet<String>);

impl NameSet {
    pub fn new<I, S>(names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        Self(
            names
                .into_iter()
                .map(|n| n.as_ref().trim().to_lowercase())
                .filter(|n| !n.is_empty())
                .collect(),
        )
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether `address` (a full OSC path) matches any configured name. A
    /// name matches if it equals the full path, the path relative to
    /// `/avatar/parameters/` or the last path segment.
    pub fn matches(&self, address: &str) -> bool {
        if self.0.is_empty() {
            return false;
        }
        let full = address.to_lowercase();
        if self.0.contains(&full) {
            return true;
        }
        if let Some(rel) = full.strip_prefix(AVATAR_PARAMETERS_PREFIX) {
            if self.0.contains(rel) {
                return true;
            }
        }
        full.rsplit('/')
            .next()
            .is_some_and(|last| self.0.contains(last))
    }
}

/// Trims, removes empties, de-duplicates (case-insensitive, first wins) and
/// sorts (case-insensitive) a list of parameter names.
pub fn normalize_name_list<I, S>(names: I) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut seen = HashSet::new();
    let mut out: Vec<String> = names
        .into_iter()
        .map(|n| n.as_ref().trim().to_string())
        .filter(|n| !n.is_empty() && seen.insert(n.to_lowercase()))
        .collect();
    out.sort_by_key(|n| n.to_lowercase());
    out
}

/// Normalises fixed parameters: trims names, rejects empty names,
/// de-duplicates case-insensitively (last wins) and sorts by name.
pub fn normalize_fixed_parameters(params: Vec<FixedParam>) -> Result<Vec<FixedParam>> {
    let mut map: BTreeMap<String, FixedParam> = BTreeMap::new();
    for p in params {
        let name = p.name.trim().to_string();
        if name.is_empty() {
            return Err(CoreError::invalid("Fixed parameter name is required"));
        }
        if let ParamValue::Float(v) = p.value {
            if !v.is_finite() {
                return Err(CoreError::invalid(format!(
                    "'{v}' is not a valid float for '{name}'"
                )));
            }
        }
        map.insert(name.to_lowercase(), FixedParam::new(name, p.value));
    }
    Ok(map.into_values().collect())
}

/// Global fixed parameters merged with per-avatar ones; the avatar entry wins
/// on a (case-insensitive) name clash. Result is sorted by name.
pub fn effective_fixed_parameters(global: &[FixedParam], avatar: &[FixedParam]) -> Vec<FixedParam> {
    let mut map: BTreeMap<String, FixedParam> = BTreeMap::new();
    for p in global.iter().chain(avatar) {
        let name = p.name.trim();
        if !name.is_empty() {
            map.insert(name.to_lowercase(), FixedParam::new(name, p.value));
        }
    }
    map.into_values().collect()
}

/// OSC address for a fixed parameter name.
pub fn fixed_parameter_address(name: &str) -> String {
    let name = name.trim();
    if name.starts_with('/') {
        name.to_string()
    } else {
        format!("{AVATAR_PARAMETERS_PREFIX}{name}")
    }
}

/// Messages to send when applying a profile, honouring the exclusion filter.
pub fn profile_messages(profile: &Profile, excluded: &NameSet) -> Vec<(String, ParamValue)> {
    profile
        .parameters
        .iter()
        .filter(|(addr, _)| !excluded.matches(addr))
        .map(|(addr, v)| (addr.clone(), *v))
        .collect()
}

/// Messages to send for fixed parameters.
pub fn fixed_messages(fixed: &[FixedParam]) -> Vec<(String, ParamValue)> {
    fixed
        .iter()
        .map(|p| (fixed_parameter_address(&p.name), p.value))
        .collect()
}

/// Values from the previous avatar's parameter snapshot that should be
/// carried over to the new avatar. Sorted by address.
pub fn select_auto_sync(
    snapshot: &HashMap<String, ParamValue>,
    names: &NameSet,
) -> Vec<(String, ParamValue)> {
    let mut out: Vec<_> = snapshot
        .iter()
        .filter(|(addr, _)| names.matches(addr))
        .map(|(addr, v)| (addr.clone(), *v))
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Looks up the given names in a profile and returns them as fixed
/// parameters with the profile's values. Unknown names are omitted.
pub fn resolve_fixed_from_profile(profile: &Profile, names: &[String]) -> Vec<FixedParam> {
    names
        .iter()
        .filter_map(|name| {
            let set = NameSet::new([name]);
            profile
                .parameters
                .iter()
                .find(|(addr, _)| set.matches(addr))
                .map(|(_, v)| FixedParam::new(name.trim(), *v))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_matching() {
        let set = NameSet::new(["FT/EyeSync", "toggle", "/avatar/parameters/Full"]);
        assert!(set.matches("/avatar/parameters/FT/EyeSync"));
        assert!(set.matches("/avatar/parameters/Some/Toggle"));
        assert!(set.matches("/avatar/parameters/full"));
        assert!(!set.matches("/avatar/parameters/EyeSyncX"));
        assert!(!NameSet::default().matches("/avatar/parameters/x"));
    }

    #[test]
    fn normalize_names() {
        assert_eq!(
            normalize_name_list([" b", "A", "a", "", "B "]),
            vec!["A".to_string(), "b".to_string()]
        );
    }

    #[test]
    fn fixed_params_rules() {
        let global = vec![
            FixedParam::new("A", ParamValue::Int(1)),
            FixedParam::new("B", ParamValue::Int(1)),
        ];
        let avatar = vec![FixedParam::new("a", ParamValue::Int(2))];
        let eff = effective_fixed_parameters(&global, &avatar);
        assert_eq!(
            eff,
            vec![
                FixedParam::new("a", ParamValue::Int(2)),
                FixedParam::new("B", ParamValue::Int(1)),
            ]
        );
        assert_eq!(fixed_parameter_address("X"), "/avatar/parameters/X");
        assert_eq!(fixed_parameter_address("/custom/X"), "/custom/X");
        assert!(
            normalize_fixed_parameters(vec![FixedParam::new(" ", ParamValue::Int(0))]).is_err()
        );
        let norm = normalize_fixed_parameters(vec![
            FixedParam::new("z", ParamValue::Int(0)),
            FixedParam::new(" Z ", ParamValue::Int(5)),
            FixedParam::new("a", ParamValue::Bool(true)),
        ])
        .unwrap();
        assert_eq!(
            norm,
            vec![
                FixedParam::new("a", ParamValue::Bool(true)),
                FixedParam::new("Z", ParamValue::Int(5)),
            ]
        );
    }

    #[test]
    fn profile_filtering_and_resolve() {
        let profile = Profile::new(
            "p",
            BTreeMap::from([
                ("/avatar/parameters/Keep".into(), ParamValue::Int(1)),
                ("/avatar/parameters/FT/Drop".into(), ParamValue::Int(2)),
            ]),
        );
        let msgs = profile_messages(&profile, &NameSet::new(["Drop"]));
        assert_eq!(
            msgs,
            vec![("/avatar/parameters/Keep".into(), ParamValue::Int(1))]
        );

        let fixed = resolve_fixed_from_profile(&profile, &["ft/drop".into(), "Missing".into()]);
        assert_eq!(fixed, vec![FixedParam::new("ft/drop", ParamValue::Int(2))]);
    }

    #[test]
    fn auto_sync_selection() {
        let snapshot = HashMap::from([
            ("/avatar/parameters/Hue".to_string(), ParamValue::Float(0.3)),
            (
                "/avatar/parameters/Other".to_string(),
                ParamValue::Float(0.1),
            ),
        ]);
        let sel = select_auto_sync(&snapshot, &NameSet::new(["hue"]));
        assert_eq!(
            sel,
            vec![("/avatar/parameters/Hue".into(), ParamValue::Float(0.3))]
        );
    }
}
