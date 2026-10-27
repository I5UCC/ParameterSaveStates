//! File based persistence.
//!
//! Layout below the data root:
//!
//! ```text
//! settings.json            global settings (Settings)
//! avatars/<avatar>.json    one file per avatar (AvatarData)
//! ```
//!
//! All writes are atomic (write to a temporary file and rename) and all
//! read-modify-write cycles are serialised through a process wide lock.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::error::{CoreError, Result};
use crate::model::{AvatarData, AvatarSummary, Settings, FORMAT_VERSION};

pub const SETTINGS_FILE: &str = "settings.json";
pub const AVATARS_DIR: &str = "avatars";

#[derive(Debug, Clone)]
pub struct Store {
    root: PathBuf,
    lock: Arc<Mutex<()>>,
}

impl Store {
    /// Opens (and creates if necessary) a store rooted at `root`.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        let avatars = root.join(AVATARS_DIR);
        fs::create_dir_all(&avatars).map_err(|e| CoreError::io(&avatars, e))?;
        Ok(Self {
            root,
            lock: Arc::new(Mutex::new(())),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn write_lock(&self) -> MutexGuard<'_, ()> {
        self.lock.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn settings_path(&self) -> PathBuf {
        self.root.join(SETTINGS_FILE)
    }

    fn avatars_dir(&self) -> PathBuf {
        self.root.join(AVATARS_DIR)
    }

    /// Path of the file storing the given avatar.
    pub fn avatar_path(&self, avatar_id: &str) -> PathBuf {
        self.avatars_dir()
            .join(format!("{}.json", avatar_file_stem(avatar_id)))
    }

    // ---------------------------------------------------------------- settings

    pub fn settings(&self) -> Result<Settings> {
        Ok(read_json(&self.settings_path())?.unwrap_or_default())
    }

    /// Read-modify-write of the global settings. Nothing is written if `f` fails.
    pub fn update_settings<R>(&self, f: impl FnOnce(&mut Settings) -> Result<R>) -> Result<R> {
        let _guard = self.write_lock();
        let path = self.settings_path();
        let mut settings: Settings = read_json(&path)?.unwrap_or_default();
        let result = f(&mut settings)?;
        settings.format_version = FORMAT_VERSION;
        write_json_atomic(&path, &settings)?;
        Ok(result)
    }

    // ----------------------------------------------------------------- avatars

    pub fn avatar_exists(&self, avatar_id: &str) -> bool {
        self.avatar_path(avatar_id).is_file()
    }

    /// Loads an avatar; returns an empty [`AvatarData`] if nothing is stored.
    pub fn avatar(&self, avatar_id: &str) -> Result<AvatarData> {
        validate_avatar_id(avatar_id)?;
        let mut data: AvatarData =
            read_json(&self.avatar_path(avatar_id))?.unwrap_or_else(|| AvatarData::new(avatar_id));
        if data.id.is_empty() {
            data.id = avatar_id.to_string();
        }
        Ok(data)
    }

    /// Read-modify-write of one avatar. Nothing is written if `f` fails.
    pub fn update_avatar<R>(
        &self,
        avatar_id: &str,
        f: impl FnOnce(&mut AvatarData) -> Result<R>,
    ) -> Result<R> {
        validate_avatar_id(avatar_id)?;
        let _guard = self.write_lock();
        self.update_avatar_locked(avatar_id, f)
    }

    pub(crate) fn update_avatar_locked<R>(
        &self,
        avatar_id: &str,
        f: impl FnOnce(&mut AvatarData) -> Result<R>,
    ) -> Result<R> {
        let path = self.avatar_path(avatar_id);
        let mut data: AvatarData = read_json(&path)?.unwrap_or_else(|| AvatarData::new(avatar_id));
        data.id = avatar_id.to_string();
        let before = data.clone();
        let result = f(&mut data)?;
        if data != before || !path.exists() {
            data.format_version = FORMAT_VERSION;
            write_json_atomic(&path, &data)?;
        }
        Ok(result)
    }

    /// Updates the cached display name of an avatar (only writes on change).
    pub fn set_avatar_name(&self, avatar_id: &str, name: &str) -> Result<()> {
        let name = name.trim();
        if name.is_empty() {
            return Ok(());
        }
        if self.avatar(avatar_id)?.name.as_deref() == Some(name) {
            return Ok(());
        }
        self.update_avatar(avatar_id, |a| {
            a.name = Some(name.to_string());
            Ok(())
        })
    }

    /// Deletes all stored data of an avatar. Returns whether anything was deleted.
    pub fn delete_avatar(&self, avatar_id: &str) -> Result<bool> {
        validate_avatar_id(avatar_id)?;
        let _guard = self.write_lock();
        let path = self.avatar_path(avatar_id);
        match fs::remove_file(&path) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(CoreError::io(path, e)),
        }
    }

    /// All stored avatars, sorted by display name (case-insensitive).
    /// Unreadable files are skipped with a warning.
    pub fn list_avatars(&self) -> Result<Vec<AvatarSummary>> {
        let dir = self.avatars_dir();
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(CoreError::io(dir, e)),
        };
        let mut out = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            match read_json::<AvatarData>(&path) {
                Ok(Some(data)) if !data.id.is_empty() => out.push(data.summary()),
                Ok(_) => {}
                Err(e) => tracing::warn!("Skipping unreadable avatar file: {e}"),
            }
        }
        out.sort_by(|a, b| {
            let ka = a.name.as_deref().unwrap_or(&a.id).to_lowercase();
            let kb = b.name.as_deref().unwrap_or(&b.id).to_lowercase();
            ka.cmp(&kb).then_with(|| a.id.cmp(&b.id))
        });
        Ok(out)
    }

    /// Avatars that have at least one profile.
    pub fn avatars_with_profiles(&self) -> Result<Vec<AvatarSummary>> {
        Ok(self
            .list_avatars()?
            .into_iter()
            .filter(|a| a.profile_count > 0)
            .collect())
    }

    /// Writes raw, already validated avatar data (used by backup restore).
    pub(crate) fn write_avatar_locked(&self, data: &AvatarData) -> Result<()> {
        validate_avatar_id(&data.id)?;
        write_json_atomic(&self.avatar_path(&data.id), data)
    }

    pub(crate) fn write_settings_locked(&self, settings: &Settings) -> Result<()> {
        write_json_atomic(&self.settings_path(), settings)
    }
}

/// Rejects ids that cannot possibly be VRChat avatar ids.
pub fn validate_avatar_id(avatar_id: &str) -> Result<()> {
    if avatar_id.trim().is_empty() {
        return Err(CoreError::invalid("avatarId is required"));
    }
    if avatar_id.len() > 200 || avatar_id.chars().any(char::is_control) {
        return Err(CoreError::invalid("Invalid avatar id"));
    }
    Ok(())
}

/// Maps an avatar id to a safe file stem. Regular ids (`avtr_<uuid>`) are kept
/// verbatim; any other character is replaced by `_`.
fn avatar_file_stem(avatar_id: &str) -> String {
    avatar_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

pub(crate) fn read_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(CoreError::io(path, e)),
    };
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
    serde_json::from_slice(bytes)
        .map(Some)
        .map_err(|e| CoreError::json(path, e))
}

pub(crate) fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let json = serde_json::to_vec_pretty(value).map_err(|e| CoreError::json(path, e))?;
    write_atomic(path, &json)
}

pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| CoreError::io(parent, e))?;
    }
    let mut tmp_name = path.as_os_str().to_owned();
    tmp_name.push(".tmp");
    let tmp = PathBuf::from(tmp_name);
    {
        let mut file = fs::File::create(&tmp).map_err(|e| CoreError::io(&tmp, e))?;
        file.write_all(bytes).map_err(|e| CoreError::io(&tmp, e))?;
        file.sync_all().map_err(|e| CoreError::io(&tmp, e))?;
    }
    fs::rename(&tmp, path).map_err(|e| CoreError::io(path, e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ParamValue;
    use std::collections::BTreeMap;

    #[test]
    fn roundtrip_and_listing() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();

        assert!(store.list_avatars().unwrap().is_empty());
        assert!(store.avatar("avtr_a").unwrap().profiles.is_empty());

        store
            .update_avatar("avtr_b", |a| {
                a.name = Some("Zeta".into());
                a.add_profile(
                    Some("P"),
                    BTreeMap::from([("/avatar/parameters/x".into(), ParamValue::Int(1))]),
                )
            })
            .unwrap();
        store.set_avatar_name("avtr_a", "alpha").unwrap();

        let list = store.list_avatars().unwrap();
        assert_eq!(
            list.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(),
            ["avtr_a", "avtr_b"]
        );
        assert_eq!(store.avatars_with_profiles().unwrap().len(), 1);

        // Failing closures do not write.
        let err = store.update_avatar("avtr_b", |a| a.rename_profile("missing", "x"));
        assert!(err.is_err());
        assert_eq!(store.avatar("avtr_b").unwrap().profile_names(), ["P"]);

        assert!(store.delete_avatar("avtr_b").unwrap());
        assert!(!store.delete_avatar("avtr_b").unwrap());
    }

    #[test]
    fn odd_avatar_ids_are_sanitised() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let path = store.avatar_path("local:../../x");
        assert_eq!(path.parent().unwrap(), dir.path().join(AVATARS_DIR));
        assert!(store.avatar("").is_err());
    }

    #[test]
    fn settings_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        store
            .update_settings(|s| {
                s.auto_sync_parameters = vec!["Hue".into()];
                Ok(())
            })
            .unwrap();
        assert_eq!(store.settings().unwrap().auto_sync_parameters, ["Hue"]);
    }
}
