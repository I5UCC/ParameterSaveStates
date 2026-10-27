use std::path::PathBuf;

/// Environment variable overriding the data directory.
pub const DATA_DIR_ENV: &str = "PSSTATES_DATA_DIR";

/// Platform data directory.
///
/// * Windows: `%APPDATA%\I5UCC\ParameterSaveStates\data`
/// * Linux: `$XDG_DATA_HOME/parametersavestates` (usually `~/.local/share/parametersavestates`)
pub fn default_data_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os(DATA_DIR_ENV).filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    directories::ProjectDirs::from("", "I5UCC", "ParameterSaveStates")
        .map(|d| d.data_dir().to_path_buf())
}

/// Location of the legacy (Unity) `Profiles` folder, if it exists.
///
/// Unity's `persistentDataPath` on Windows is
/// `%USERPROFILE%\AppData\LocalLow\<company>\<product>`.
pub fn legacy_profiles_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        let profile = std::env::var_os("USERPROFILE")?;
        let dir = PathBuf::from(profile)
            .join("AppData")
            .join("LocalLow")
            .join("I5UCC")
            .join("ParameterSaveStates")
            .join("Profiles");
        dir.is_dir().then_some(dir)
    }
    #[cfg(not(windows))]
    {
        // The Unity build only ever shipped for Windows; on Linux it could only
        // have run through Wine/Proton, so we cannot guess the prefix reliably.
        None
    }
}
