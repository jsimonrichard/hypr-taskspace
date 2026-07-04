use std::env;
use std::path::{Path, PathBuf};

use crate::error::{TskError, Result};

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
        .map_err(|_| TskError::NoRuntimeDir)
}

pub fn tsk_config_dir() -> PathBuf {
    config_home().join("tsk")
}

pub fn tsk_config_path() -> PathBuf {
    if crate::dev_session::dev_session_active() {
        if let Ok(path) = env::var("TSK_CONFIG") {
            return expand(path);
        }
        return crate::install::profile::dev_config_path();
    }
    if let Ok(exe) = env::current_exe() {
        let exe = exe.to_string_lossy();
        if exe.contains("/tsk-dev/") {
            return expand("~/.config/tsk-dev/config.toml");
        }
    }
    tsk_config_dir().join("config.toml")
}

pub fn tsk_data_dir() -> PathBuf {
    data_home().join("tsk")
}

/// Resolve `[daemon].socket` from config to an absolute path.
///
/// Absolute paths and `~`-prefixed paths are expanded as-is. Bare filenames and
/// legacy values like `tsk/daemon.sock` resolve under `~/.local/share/tsk/`.
pub fn resolve_daemon_socket_path(configured: &str) -> PathBuf {
    let trimmed = configured.trim();
    if trimmed.starts_with('~') || trimmed.starts_with('/') {
        return expand(trimmed);
    }
    match trimmed {
        "daemon.sock" | "tsk/daemon.sock" => tsk_data_dir().join("daemon.sock"),
        other => tsk_data_dir().join(other),
    }
}

pub fn tsk_state_db() -> PathBuf {
    tsk_data_dir().join("state.db")
}

pub fn tsk_runtime_dir() -> Result<PathBuf> {
    Ok(runtime_dir()?.join("tsk"))
}

/// User-local executables directory (`~/.local/bin`).
pub fn user_bin_dir() -> PathBuf {
    expand("~/.local/bin")
}

pub fn ensure_parent(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| TskError::Write {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tsk_config_path_ignores_tsk_config_env_without_active_session() {
        crate::dev_session::stop_dev_session().ok();
        std::env::set_var("TSK_CONFIG", "/home/u/.config/tsk-dev/config.toml");
        let path = tsk_config_path();
        assert!(
            !path.to_string_lossy().contains("tsk-dev"),
            "expected prod config path, got {}",
            path.display()
        );
        std::env::remove_var("TSK_CONFIG");
    }
}
