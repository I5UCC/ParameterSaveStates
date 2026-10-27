//! Backup export / import as zip archives.
//!
//! Native archives contain `settings.json` and `avatars/*.json`. Archives
//! produced by the legacy Unity version (zip of the `Profiles` folder) are
//! detected automatically and imported through [`crate::legacy`].

use std::fs;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::error::{CoreError, Result};
use crate::legacy::{migrate_legacy_dir, MigrationReport};
use crate::model::{AvatarData, Settings};
use crate::store::{Store, AVATARS_DIR, SETTINGS_FILE};
use crate::util::{compact_utc_timestamp, unix_now};

/// Maximum size of a single extracted entry (protects against zip bombs).
const MAX_ENTRY_SIZE: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportReport {
    /// A native backup was restored; number of restored files.
    Native { files: usize },
    /// A legacy backup was migrated.
    Legacy(MigrationReport),
}

impl ImportReport {
    pub fn summary(&self) -> String {
        match self {
            ImportReport::Native { files } => format!("Restored {files} file(s) from backup"),
            ImportReport::Legacy(r) => format!("Legacy backup: {}", r.summary()),
        }
    }
}

/// Suggested file name for a new backup.
pub fn default_backup_file_name() -> String {
    format!("profiles-export-{}.zip", compact_utc_timestamp(unix_now()))
}

/// Writes all store files into a zip archive. Returns the number of files.
pub fn export_backup(store: &Store, dest: &Path) -> Result<usize> {
    let _guard = store.write_lock();
    let mut files = Vec::new();
    let settings = store.root().join(SETTINGS_FILE);
    if settings.is_file() {
        files.push((SETTINGS_FILE.to_string(), settings));
    }
    let avatars = store.root().join(AVATARS_DIR);
    if let Ok(entries) = fs::read_dir(&avatars) {
        let mut entries: Vec<_> = entries.flatten().map(|e| e.path()).collect();
        entries.sort();
        for path in entries {
            if path.is_file() && path.extension().and_then(|e| e.to_str()) == Some("json") {
                let name = path.file_name().unwrap().to_string_lossy();
                files.push((format!("{AVATARS_DIR}/{name}"), path));
            }
        }
    }
    if files.is_empty() {
        return Err(CoreError::invalid("There is nothing to export yet"));
    }

    let file = fs::File::create(dest).map_err(|e| CoreError::io(dest, e))?;
    let mut zip = ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for (name, path) in &files {
        let bytes = fs::read(path).map_err(|e| CoreError::io(path, e))?;
        zip.start_file(name.as_str(), options)?;
        zip.write_all(&bytes).map_err(|e| CoreError::io(dest, e))?;
    }
    zip.finish()?;
    Ok(files.len())
}

/// Safe relative path of an archive entry (no absolute paths, no `..`).
fn safe_entry_path(name: &str) -> Option<PathBuf> {
    let path = Path::new(name);
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(c) => out.push(c),
            Component::CurDir => {}
            _ => return None,
        }
    }
    (!out.as_os_str().is_empty()).then_some(out)
}

fn read_entry<R: Read>(entry: &mut zip::read::ZipFile<'_, R>) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    entry
        .take(MAX_ENTRY_SIZE + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| CoreError::io(entry.name().to_string(), e))?;
    if bytes.len() as u64 > MAX_ENTRY_SIZE {
        return Err(CoreError::invalid(format!(
            "Archive entry '{}' is too large",
            entry.name()
        )));
    }
    Ok(bytes)
}

/// Imports a backup archive (native or legacy).
pub fn import_backup(store: &Store, src: &Path) -> Result<ImportReport> {
    let file = fs::File::open(src).map_err(|e| CoreError::io(src, e))?;
    let mut archive =
        ZipArchive::new(file).map_err(|_| CoreError::invalid("Invalid zip archive"))?;
    if archive.is_empty() {
        return Err(CoreError::invalid("Archive has no files"));
    }

    let native = archive
        .file_names()
        .any(|n| n == SETTINGS_FILE || n.starts_with(&format!("{AVATARS_DIR}/")));
    if native {
        import_native(store, &mut archive).map(|files| ImportReport::Native { files })
    } else {
        import_legacy(store, &mut archive).map(ImportReport::Legacy)
    }
}

fn import_native(store: &Store, archive: &mut ZipArchive<fs::File>) -> Result<usize> {
    // Parse everything first so a broken archive does not half-apply.
    let mut settings: Option<Settings> = None;
    let mut avatars: Vec<AvatarData> = Vec::new();
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().to_string();
        let Some(path) = safe_entry_path(&name) else {
            return Err(CoreError::invalid("Archive contains invalid file path"));
        };
        let bytes = read_entry(&mut entry)?;
        let bytes = bytes
            .strip_prefix(b"\xEF\xBB\xBF")
            .unwrap_or(&bytes)
            .to_vec();
        if path == Path::new(SETTINGS_FILE) {
            settings = Some(serde_json::from_slice(&bytes).map_err(|e| CoreError::json(&path, e))?);
        } else if path.parent() == Some(Path::new(AVATARS_DIR))
            && path.extension().and_then(|e| e.to_str()) == Some("json")
        {
            let data: AvatarData =
                serde_json::from_slice(&bytes).map_err(|e| CoreError::json(&path, e))?;
            if !data.id.is_empty() {
                avatars.push(data);
            }
        } else {
            tracing::warn!("Ignoring unexpected archive entry '{name}'");
        }
    }

    let _guard = store.write_lock();
    let mut files = 0;
    if let Some(settings) = settings {
        store.write_settings_locked(&settings)?;
        files += 1;
    }
    for avatar in &avatars {
        store.write_avatar_locked(avatar)?;
        files += 1;
    }
    Ok(files)
}

fn import_legacy(store: &Store, archive: &mut ZipArchive<fs::File>) -> Result<MigrationReport> {
    let temp = tempfile::tempdir().map_err(|e| CoreError::io(std::env::temp_dir(), e))?;
    let mut extracted = 0;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        if entry.is_dir() {
            continue;
        }
        let Some(rel) = safe_entry_path(entry.name()) else {
            return Err(CoreError::invalid("Archive contains invalid file path"));
        };
        let bytes = read_entry(&mut entry)?;
        let dest = temp.path().join(rel);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).map_err(|e| CoreError::io(parent, e))?;
        }
        fs::write(&dest, bytes).map_err(|e| CoreError::io(&dest, e))?;
        extracted += 1;
    }
    if extracted == 0 {
        return Err(CoreError::invalid("Archive has no files"));
    }
    // Some tools wrap everything in a top level "Profiles/" folder.
    let root = temp.path().join("Profiles");
    let root = if root.is_dir() {
        root
    } else {
        temp.path().to_path_buf()
    };
    migrate_legacy_dir(store, &root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ParamValue;
    use std::collections::BTreeMap;

    #[test]
    fn export_import_native() {
        let a = tempfile::tempdir().unwrap();
        let store = Store::open(a.path()).unwrap();
        store
            .update_avatar("avtr_x", |av| {
                av.add_profile(
                    Some("P"),
                    BTreeMap::from([("/avatar/parameters/a".into(), ParamValue::Int(2))]),
                )
            })
            .unwrap();
        store.update_settings(|_| Ok(())).unwrap();

        let out = tempfile::tempdir().unwrap();
        let zip_path = out.path().join(default_backup_file_name());
        assert_eq!(export_backup(&store, &zip_path).unwrap(), 2);

        let b = tempfile::tempdir().unwrap();
        let other = Store::open(b.path()).unwrap();
        let report = import_backup(&other, &zip_path).unwrap();
        assert_eq!(report, ImportReport::Native { files: 2 });
        assert_eq!(other.avatar("avtr_x").unwrap().profile_names(), ["P"]);
    }

    #[test]
    fn import_legacy_zip_and_reject_traversal() {
        let out = tempfile::tempdir().unwrap();
        let legacy_zip = out.path().join("legacy.zip");
        {
            let mut zip = ZipWriter::new(fs::File::create(&legacy_zip).unwrap());
            let opts = SimpleFileOptions::default();
            zip.start_file("avtr_1/1_A", opts).unwrap();
            zip.write_all(br#"{"/avatar/parameters/x":{"Item1":"1","Item2":"i"}}"#)
                .unwrap();
            zip.start_file("avtr_1/Name", opts).unwrap();
            zip.write_all(b"Legacy").unwrap();
            zip.finish().unwrap();
        }
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        match import_backup(&store, &legacy_zip).unwrap() {
            ImportReport::Legacy(r) => assert_eq!(r.profiles, 1),
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(
            store.avatar("avtr_1").unwrap().name.as_deref(),
            Some("Legacy")
        );

        let evil = out.path().join("evil.zip");
        {
            let mut zip = ZipWriter::new(fs::File::create(&evil).unwrap());
            zip.start_file("../evil", SimpleFileOptions::default())
                .unwrap();
            zip.write_all(b"x").unwrap();
            zip.finish().unwrap();
        }
        assert!(import_backup(&store, &evil).is_err());
        assert!(safe_entry_path("/abs").is_none());
        assert!(safe_entry_path("a/../../b").is_none());
    }
}
