//! Import of data written by the legacy Unity version of ParameterSaveStates.
//!
//! Legacy layout (`%USERPROFILE%\AppData\LocalLow\I5UCC\ParameterSaveStates\Profiles`):
//!
//! ```text
//! ApplyProfileSettings.json   {"excludedParameterNames":[..]}
//! AutoSync.json               {"parameterNames":[..]}
//! FixedParameters.json        {"parameters":[{"name","type","value"}]}
//! WebUiSettings.json          {"theme":"dark"|"light"}
//! <avtr_id>/Name              plain text display name
//! <avtr_id>/{index}_{name}    profile: {"/avatar/parameters/X":{"Item1":"0.5","Item2":"f"}}
//! <avtr_id>/ApplyProfileSettings.json, AutoSync.json, FixedParameters.json
//! ```

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::Path;

use serde::Deserialize;

use crate::error::{CoreError, Result};
use crate::model::{
    validate_profile_name, AvatarData, FixedParam, ParamType, ParamValue, Profile, Theme,
};
use crate::rules::{normalize_fixed_parameters, normalize_name_list};
use crate::store::Store;

const NAME_FILE: &str = "Name";
const APPLY_SETTINGS_FILE: &str = "ApplyProfileSettings.json";
const AUTO_SYNC_FILE: &str = "AutoSync.json";
const FIXED_FILE: &str = "FixedParameters.json";
const WEB_UI_SETTINGS_FILE: &str = "WebUiSettings.json";
const REORDER_TMP_PREFIX: &str = "__reorder_tmp_";

/// Summary of a legacy import.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MigrationReport {
    pub avatars: usize,
    pub profiles: usize,
    /// Profiles that were renamed because the name already existed.
    pub renamed_profiles: usize,
    /// Parameter values that could not be converted.
    pub skipped_values: usize,
    pub warnings: Vec<String>,
}

impl MigrationReport {
    pub fn summary(&self) -> String {
        let mut s = format!(
            "Imported {} profile(s) for {} avatar(s)",
            self.profiles, self.avatars
        );
        if self.renamed_profiles > 0 {
            s.push_str(&format!(
                ", {} renamed to avoid name clashes",
                self.renamed_profiles
            ));
        }
        if self.skipped_values > 0 {
            s.push_str(&format!(
                ", {} unsupported value(s) skipped",
                self.skipped_values
            ));
        }
        if !self.warnings.is_empty() {
            s.push_str(&format!(", {} warning(s)", self.warnings.len()));
        }
        s
    }
}

#[derive(Deserialize)]
struct LegacyEntry {
    #[serde(rename = "Item1")]
    value: Option<String>,
    #[serde(rename = "Item2")]
    ty: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct LegacyApplySettings {
    #[serde(rename = "excludedParameterNames")]
    excluded_parameter_names: Vec<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct LegacyAutoSync {
    #[serde(rename = "parameterNames")]
    parameter_names: Vec<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct LegacyFixedParameters {
    parameters: Vec<LegacyFixedParameter>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct LegacyFixedParameter {
    name: String,
    #[serde(rename = "type")]
    ty: String,
    value: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct LegacyWebUiSettings {
    theme: String,
}

/// Heuristic: does this directory look like a legacy `Profiles` folder?
pub fn looks_like_legacy_dir(dir: &Path) -> bool {
    if [
        APPLY_SETTINGS_FILE,
        AUTO_SYNC_FILE,
        FIXED_FILE,
        WEB_UI_SETTINGS_FILE,
    ]
    .iter()
    .any(|f| dir.join(f).is_file())
    {
        return true;
    }
    fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .any(|e| e.path().is_dir() && e.file_name().to_string_lossy().starts_with("avtr_"))
        })
        .unwrap_or(false)
}

fn is_reserved_file(name: &str) -> bool {
    matches!(
        name,
        NAME_FILE | APPLY_SETTINGS_FILE | AUTO_SYNC_FILE | FIXED_FILE
    ) || name.starts_with(REORDER_TMP_PREFIX)
        || name.ends_with(".tmp")
        || name.starts_with('.')
}

/// Parses `{index}_{displayName}`.
fn parse_profile_file_name(file_name: &str) -> (Option<i64>, String) {
    if let Some((prefix, rest)) = file_name.split_once('_') {
        if let Ok(idx) = prefix.trim().parse::<i64>() {
            return (Some(idx), rest.to_string());
        }
    }
    (None, file_name.to_string())
}

/// Converts one legacy `{Item1, Item2}` entry.
fn convert_value(entry: &LegacyEntry) -> Option<ParamValue> {
    let raw = entry.value.as_deref()?.trim();
    let ty = ParamType::from_osc_tag(entry.ty.as_deref()?.trim())?;
    ParamValue::parse(ty, raw).ok().or_else(|| match ty {
        // Older builds formatted floats with the current culture ("0,5").
        ParamType::Float => ParamValue::parse(ty, &raw.replace(',', ".")).ok(),
        _ => None,
    })
}

fn read_text(path: &Path) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
    Some(String::from_utf8_lossy(bytes).into_owned())
}

fn read_legacy_json<T: for<'de> Deserialize<'de> + Default>(
    path: &Path,
    report: &mut MigrationReport,
) -> T {
    let Some(text) = read_text(path) else {
        return T::default();
    };
    serde_json::from_str(&text).unwrap_or_else(|e| {
        report
            .warnings
            .push(format!("Could not read '{}': {e}", path.display()));
        T::default()
    })
}

fn convert_fixed(legacy: LegacyFixedParameters, report: &mut MigrationReport) -> Vec<FixedParam> {
    legacy
        .parameters
        .into_iter()
        .filter_map(|p| {
            let name = p.name.trim().to_string();
            let ty = ParamType::from_osc_tag(p.ty.trim());
            match (name.is_empty(), ty) {
                (false, Some(ty)) => match ParamValue::parse(ty, &p.value) {
                    Ok(v) => Some(FixedParam::new(name, v)),
                    Err(_) => {
                        report.skipped_values += 1;
                        None
                    }
                },
                _ => {
                    report.skipped_values += 1;
                    None
                }
            }
        })
        .collect()
}

fn merge_fixed(existing: &[FixedParam], imported: Vec<FixedParam>) -> Vec<FixedParam> {
    // Existing entries win, so they go last.
    let merged: Vec<FixedParam> = imported
        .into_iter()
        .chain(existing.iter().cloned())
        .collect();
    normalize_fixed_parameters(merged).unwrap_or_else(|_| existing.to_vec())
}

fn merge_names(existing: &[String], imported: Vec<String>) -> Vec<String> {
    normalize_name_list(existing.iter().cloned().chain(imported))
}

struct LegacyProfileFile {
    index: Option<i64>,
    display_name: String,
    file_name: String,
    parameters: BTreeMap<String, ParamValue>,
}

fn read_legacy_avatar(
    dir: &Path,
    report: &mut MigrationReport,
) -> Result<(Option<String>, Vec<LegacyProfileFile>)> {
    let name = read_text(&dir.join(NAME_FILE))
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty());

    let mut profiles = Vec::new();
    let entries = fs::read_dir(dir).map_err(|e| CoreError::io(dir, e))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let file_name = entry.file_name().to_string_lossy().into_owned();
        if is_reserved_file(&file_name) {
            continue;
        }
        let Some(text) = read_text(&path) else {
            continue;
        };
        let raw: HashMap<String, LegacyEntry> = match serde_json::from_str(&text) {
            Ok(raw) => raw,
            Err(e) => {
                report
                    .warnings
                    .push(format!("Skipped profile '{}': {e}", path.display()));
                continue;
            }
        };
        let mut parameters = BTreeMap::new();
        for (addr, entry) in &raw {
            match convert_value(entry) {
                Some(v) => {
                    parameters.insert(addr.clone(), v);
                }
                None => report.skipped_values += 1,
            }
        }
        let (index, display_name) = parse_profile_file_name(&file_name);
        profiles.push(LegacyProfileFile {
            index,
            display_name,
            file_name,
            parameters,
        });
    }

    profiles.sort_by(|a, b| {
        a.index
            .is_none()
            .cmp(&b.index.is_none())
            .then(a.index.cmp(&b.index))
            .then_with(|| {
                a.display_name
                    .to_lowercase()
                    .cmp(&b.display_name.to_lowercase())
            })
            .then_with(|| a.file_name.cmp(&b.file_name))
    });
    Ok((name, profiles))
}

/// Imports a legacy `Profiles` directory into `store`, merging with existing
/// data. Existing profiles are never overwritten: clashing names get a
/// ` (legacy)` suffix.
pub fn migrate_legacy_dir(store: &Store, dir: &Path) -> Result<MigrationReport> {
    if !dir.is_dir() {
        return Err(CoreError::invalid(format!(
            "'{}' is not a directory",
            dir.display()
        )));
    }
    let mut report = MigrationReport::default();

    // Global settings.
    let apply: LegacyApplySettings = read_legacy_json(&dir.join(APPLY_SETTINGS_FILE), &mut report);
    let auto: LegacyAutoSync = read_legacy_json(&dir.join(AUTO_SYNC_FILE), &mut report);
    let fixed_raw: LegacyFixedParameters = read_legacy_json(&dir.join(FIXED_FILE), &mut report);
    let fixed = convert_fixed(fixed_raw, &mut report);
    let web: LegacyWebUiSettings = read_legacy_json(&dir.join(WEB_UI_SETTINGS_FILE), &mut report);
    store.update_settings(|s| {
        s.excluded_parameters = merge_names(&s.excluded_parameters, apply.excluded_parameter_names);
        s.auto_sync_parameters = merge_names(&s.auto_sync_parameters, auto.parameter_names);
        s.fixed_parameters = merge_fixed(&s.fixed_parameters, fixed);
        match web.theme.to_ascii_lowercase().as_str() {
            "light" => s.theme = Theme::Light,
            "dark" => s.theme = Theme::Dark,
            _ => {}
        }
        Ok(())
    })?;

    // Avatars.
    let mut avatar_dirs: Vec<_> = fs::read_dir(dir)
        .map_err(|e| CoreError::io(dir, e))?
        .flatten()
        .filter(|e| e.path().is_dir())
        .collect();
    avatar_dirs.sort_by_key(|e| e.file_name());

    for entry in avatar_dirs {
        let avatar_id = entry.file_name().to_string_lossy().into_owned();
        let path = entry.path();
        let (name, profiles) = read_legacy_avatar(&path, &mut report)?;
        let apply: LegacyApplySettings =
            read_legacy_json(&path.join(APPLY_SETTINGS_FILE), &mut report);
        let auto: LegacyAutoSync = read_legacy_json(&path.join(AUTO_SYNC_FILE), &mut report);
        let fixed_raw: LegacyFixedParameters =
            read_legacy_json(&path.join(FIXED_FILE), &mut report);
        let fixed = convert_fixed(fixed_raw, &mut report);

        let has_content = !profiles.is_empty()
            || !apply.excluded_parameter_names.is_empty()
            || !auto.parameter_names.is_empty()
            || !fixed.is_empty();
        if !has_content {
            continue;
        }

        let (imported, renamed) = store.update_avatar(&avatar_id, |a: &mut AvatarData| {
            if a.name.is_none() {
                a.name = name.clone();
            }
            a.excluded_parameters =
                merge_names(&a.excluded_parameters, apply.excluded_parameter_names);
            a.auto_sync_parameters = merge_names(&a.auto_sync_parameters, auto.parameter_names);
            a.fixed_parameters = merge_fixed(&a.fixed_parameters, fixed);

            let mut imported = 0;
            let mut renamed = 0;
            for p in profiles {
                let base = validate_profile_name(&p.display_name)
                    .unwrap_or_else(|_| a.next_default_name());
                let mut final_name = a.unique_name(&base);
                if final_name != base {
                    final_name = a.unique_name(&format!("{base} (legacy)"));
                    renamed += 1;
                }
                let mut profile = Profile::new(final_name, p.parameters);
                profile.created_at = None;
                a.profiles.push(profile);
                imported += 1;
            }
            Ok((imported, renamed))
        })?;
        report.avatars += 1;
        report.profiles += imported;
        report.renamed_profiles += renamed;
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn legacy_fixture(root: &Path) {
        write(
            &root.join("ApplyProfileSettings.json"),
            r#"{ "excludedParameterNames": ["VRCEmote"] }"#,
        );
        write(
            &root.join("AutoSync.json"),
            r#"{ "parameterNames": ["Hue"] }"#,
        );
        write(
            &root.join("FixedParameters.json"),
            r#"{ "parameters": [ { "name": "Glasses", "type": "T", "value": "True" },
                                 { "name": "Bad", "type": "x", "value": "1" } ] }"#,
        );
        write(&root.join("WebUiSettings.json"), r#"{"theme":"light"}"#);
        write(&root.join("WebUiWindowSize"), "1200,500");

        let av = root.join("avtr_1111");
        write(&av.join("Name"), "\u{feff}My Avatar\n");
        write(
            &av.join("2_Second"),
            r#"{ "/avatar/parameters/Mode": { "Item1": "3", "Item2": "i" } }"#,
        );
        write(
            &av.join("1_First_Outfit"),
            r#"{
              "/avatar/parameters/Foo/Bar": { "Item1": "0,5", "Item2": "f" },
              "/avatar/parameters/Toggle": { "Item1": "True", "Item2": "T" },
              "/avatar/parameters/Str": { "Item1": "x", "Item2": "s" }
            }"#,
        );
        write(&av.join("Loose"), r#"{}"#);
        write(&av.join("__reorder_tmp_abc"), r#"{}"#);
        write(
            &av.join("AutoSync.json"),
            r#"{ "parameterNames": ["Avatar/Only"] }"#,
        );
        // Empty avatar folder is ignored.
        fs::create_dir_all(root.join("avtr_empty")).unwrap();
    }

    #[test]
    fn migrates_fixture() {
        let legacy = tempfile::tempdir().unwrap();
        legacy_fixture(legacy.path());
        assert!(looks_like_legacy_dir(legacy.path()));

        let data = tempfile::tempdir().unwrap();
        let store = Store::open(data.path()).unwrap();
        let report = migrate_legacy_dir(&store, legacy.path()).unwrap();
        assert_eq!(report.avatars, 1);
        assert_eq!(report.profiles, 3);
        assert_eq!(report.skipped_values, 2); // "s" value + bad fixed type

        let settings = store.settings().unwrap();
        assert_eq!(settings.excluded_parameters, ["VRCEmote"]);
        assert_eq!(settings.auto_sync_parameters, ["Hue"]);
        assert_eq!(
            settings.fixed_parameters,
            vec![FixedParam::new("Glasses", ParamValue::Bool(true))]
        );
        assert_eq!(settings.theme, Theme::Light);

        let avatar = store.avatar("avtr_1111").unwrap();
        assert_eq!(avatar.name.as_deref(), Some("My Avatar"));
        assert_eq!(avatar.profile_names(), ["First_Outfit", "Second", "Loose"]);
        assert_eq!(
            avatar.profiles[0]
                .parameters
                .get("/avatar/parameters/Foo/Bar"),
            Some(&ParamValue::Float(0.5))
        );
        assert_eq!(
            avatar.profiles[0]
                .parameters
                .get("/avatar/parameters/Toggle"),
            Some(&ParamValue::Bool(true))
        );
        assert_eq!(avatar.auto_sync_parameters, ["Avatar/Only"]);
        assert!(!store.avatar_exists("avtr_empty"));

        // Importing twice renames instead of overwriting.
        let again = migrate_legacy_dir(&store, legacy.path()).unwrap();
        assert_eq!(again.renamed_profiles, 3);
        let avatar = store.avatar("avtr_1111").unwrap();
        assert_eq!(avatar.profiles.len(), 6);
        assert_eq!(avatar.profiles[3].name, "First_Outfit (legacy)");
    }

    #[test]
    fn file_name_parsing() {
        assert_eq!(parse_profile_file_name("3_A_B"), (Some(3), "A_B".into()));
        assert_eq!(parse_profile_file_name("x_A"), (None, "x_A".into()));
        assert_eq!(parse_profile_file_name("Plain"), (None, "Plain".into()));
    }
}
