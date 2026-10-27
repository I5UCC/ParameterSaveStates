use std::path::PathBuf;

/// Errors produced by the core crate. Display strings are user facing.
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    /// Validation / user input error.
    #[error("{0}")]
    Invalid(String),

    #[error("Profile '{0}' not found")]
    ProfileNotFound(String),

    #[error("A profile named '{0}' already exists")]
    ProfileExists(String),

    #[error("I/O error at '{path}': {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("Invalid JSON in '{path}': {source}")]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error("Zip archive error: {0}")]
    Zip(#[from] zip::result::ZipError),
}

impl CoreError {
    pub fn invalid(msg: impl Into<String>) -> Self {
        Self::Invalid(msg.into())
    }

    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }

    pub(crate) fn json(path: impl Into<PathBuf>, source: serde_json::Error) -> Self {
        Self::Json {
            path: path.into(),
            source,
        }
    }
}

pub type Result<T, E = CoreError> = std::result::Result<T, E>;
