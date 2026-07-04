use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum TskError {
    #[error("XDG_RUNTIME_DIR is not set")]
    NoRuntimeDir,
    #[error("Hyprland is not available")]
    HyprlandUnavailable,
    #[error("hyprctl failed: {0}")]
    Hyprctl(String),
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("failed to read {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to write {path}: {source}")]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to parse {path}: {source}")]
    Parse {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("config error: {0}")]
    Config(String),
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, TskError>;
