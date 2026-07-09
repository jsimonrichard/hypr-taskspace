use std::env;
use std::path::{Path, PathBuf};

use crate::error::{TskError, Result};

/// Hyprland `exec` bindings may run without `XDG_RUNTIME_DIR` or `HOME`.
/// Fill sensible defaults before any subcommand touches runtime paths or config.
pub fn normalize_desktop_env() {
    if env::var_os("XDG_RUNTIME_DIR").is_none() {
        let uid = unsafe { libc::getuid() };
        let runtime = format!("/run/user/{uid}");
        if Path::new(&runtime).is_dir() {
            env::set_var("XDG_RUNTIME_DIR", &runtime);
        }
    }
    if env::var_os("HOME").is_none() {
        if let Some(home) = passwd_home_for_uid(unsafe { libc::getuid() }) {
            env::set_var("HOME", home);
        }
    }
}

fn passwd_home_for_uid(uid: u32) -> Option<String> {
    let mut pwd: libc::passwd = unsafe { std::mem::zeroed() };
    let mut result: *mut libc::passwd = std::ptr::null_mut();
    let mut buf = vec![0u8; 16_384];
    let rc = unsafe {
        libc::getpwuid_r(
            uid as libc::uid_t,
            &mut pwd,
            buf.as_mut_ptr().cast(),
            buf.len(),
            &mut result,
        )
    };
    if rc != 0 || result.is_null() {
        return None;
    }
    let dir = unsafe { std::ffi::CStr::from_ptr(pwd.pw_dir) };
    dir.to_str().ok().map(str::to_owned)
}

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
    fn normalize_desktop_env_sets_runtime_dir_when_missing() {
        let uid = unsafe { libc::getuid() };
        let expected = format!("/run/user/{uid}");
        if !Path::new(&expected).is_dir() {
            return;
        }
        let saved = env::var_os("XDG_RUNTIME_DIR");
        env::remove_var("XDG_RUNTIME_DIR");
        normalize_desktop_env();
        assert_eq!(env::var_os("XDG_RUNTIME_DIR").as_deref(), Some(expected.as_ref()));
        match saved {
            Some(v) => env::set_var("XDG_RUNTIME_DIR", v),
            None => env::remove_var("XDG_RUNTIME_DIR"),
        }
    }

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
