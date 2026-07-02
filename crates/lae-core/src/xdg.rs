use std::env;
use std::path::{Path, PathBuf};

use crate::error::{LaeError, Result};

pub fn expand(path: impl AsRef<Path>) -> PathBuf {
    let s = path.as_ref().to_string_lossy();
    if s.starts_with('~') {
        if let Ok(home) = env::var("HOME") {
            return PathBuf::from(s.replacen('~', &home, 1));
        }
    }
    PathBuf::from(s.into_owned())
}

pub fn config_home() -> PathBuf {
    expand(env::var("XDG_CONFIG_HOME").unwrap_or_else(|_| "~/.config".into()))
}

pub fn data_home() -> PathBuf {
    expand(env::var("XDG_DATA_HOME").unwrap_or_else(|_| "~/.local/share".into()))
}

pub fn runtime_dir() -> Result<PathBuf> {
    env::var("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .map_err(|_| LaeError::NoRuntimeDir)
}

pub fn lae_config_dir() -> PathBuf {
    config_home().join("lae")
}

pub fn lae_config_path() -> PathBuf {
    lae_config_dir().join("config.toml")
}

pub fn lae_data_dir() -> PathBuf {
    data_home().join("lae")
}

/// Resolve `[daemon].socket` from config to an absolute path.
///
/// Absolute paths and `~`-prefixed paths are expanded as-is. Bare filenames and
/// legacy values like `lae/daemon.sock` resolve under `~/.local/share/lae/`.
pub fn resolve_daemon_socket_path(configured: &str) -> PathBuf {
    let trimmed = configured.trim();
    if trimmed.starts_with('~') || trimmed.starts_with('/') {
        return expand(trimmed);
    }
    match trimmed {
        "daemon.sock" | "lae/daemon.sock" => lae_data_dir().join("daemon.sock"),
        other => lae_data_dir().join(other),
    }
}

pub fn lae_state_db() -> PathBuf {
    lae_data_dir().join("state.db")
}

pub fn lae_runtime_dir() -> Result<PathBuf> {
    Ok(runtime_dir()?.join("lae"))
}

/// User-local executables directory (`~/.local/bin`).
pub fn user_bin_dir() -> PathBuf {
    expand("~/.local/bin")
}

pub fn lae_waybar_file() -> Result<PathBuf> {
    Ok(lae_runtime_dir()?.join("waybar.json"))
}

pub fn lae_waybar_modules_cache() -> Result<PathBuf> {
    Ok(lae_runtime_dir()?.join("waybar-modules.json"))
}

pub fn ensure_parent(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| LaeError::Write {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    Ok(())
}
