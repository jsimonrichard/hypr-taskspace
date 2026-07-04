//! Active dev session marker — `~/.local/share/tsk/dev-session`.
//!
//! Any `tsk` process (shell, Hyprland exec, helpers) checks this file in the prod
//! data dir. When present, it contains the path to the dev build binary; config and
//! share paths are derived from the dev install tree. No environment variables required.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::error::{Result, TskError};
use crate::install::profile::dev_share_dir;
use crate::xdg::{ensure_parent, expand, tsk_data_dir};

const LEGACY_MARKER_NAME: &str = ".session-active";

pub fn dev_session_marker_path() -> PathBuf {
    tsk_data_dir().join("dev-session")
}

fn legacy_dev_session_marker_path() -> PathBuf {
    dev_share_dir().join(LEGACY_MARKER_NAME)
}

pub fn dev_session_active() -> bool {
    dev_session_marker_path().is_file() || legacy_dev_session_marker_path().is_file()
}

/// Dev binary path recorded at session start (`scripts/dev.sh enter`).
pub fn dev_session_binary() -> Option<PathBuf> {
    let path = dev_session_marker_path();
    if path.is_file() {
        return read_session_binary(&path);
    }
    let legacy = legacy_dev_session_marker_path();
    if legacy.is_file() {
        if let Some(bin) = read_session_binary(&legacy) {
            // Migrate so prod-side code and helpers only need one location.
            start_dev_session(&bin).ok();
            let _ = fs::remove_file(&legacy);
            return Some(bin);
        }
    }
    None
}

fn read_session_binary(path: &Path) -> Option<PathBuf> {
    let raw = fs::read_to_string(path).ok()?;
    let trimmed = raw.lines().next()?.trim();
    if trimmed.is_empty() {
        return None;
    }
    let bin = expand(trimmed);
    if bin.is_file() {
        Some(bin)
    } else {
        None
    }
}

pub fn start_dev_session(binary: &Path) -> Result<()> {
    let marker = dev_session_marker_path();
    ensure_parent(&marker)?;
    let mut file = fs::File::create(&marker).map_err(|source| TskError::Write {
        path: marker.clone(),
        source,
    })?;
    file.write_all(binary.to_string_lossy().as_bytes())
        .map_err(|source| TskError::Write {
            path: marker,
            source,
        })?;
    Ok(())
}

pub fn stop_dev_session() -> Result<()> {
    for path in [dev_session_marker_path(), legacy_dev_session_marker_path()] {
        if path.is_file() {
            fs::remove_file(&path).map_err(|source| TskError::Write { path, source })?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, MutexGuard};

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn env_lock() -> MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn session_marker_roundtrip() {
        let _lock = env_lock();
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let share = home.join(".local/share/tsk");
        fs::create_dir_all(&share).unwrap();
        let bin = share.join("tsk-dev-bin");
        fs::write(&bin, b"").unwrap();

        std::env::set_var("HOME", home);
        stop_dev_session().ok();

        start_dev_session(&bin).unwrap();
        assert!(dev_session_active());
        assert_eq!(dev_session_binary(), Some(bin));
        assert!(dev_session_marker_path().is_file());

        stop_dev_session().unwrap();
        assert!(!dev_session_active());
    }

    #[test]
    fn migrates_legacy_marker_from_tsk_dev_share() {
        let _lock = env_lock();
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let prod_share = home.join(".local/share/tsk");
        let dev_share = home.join(".local/share/tsk-dev");
        fs::create_dir_all(&dev_share).unwrap();
        let bin = prod_share.join("dev-build");
        fs::create_dir_all(&prod_share).unwrap();
        fs::write(&bin, b"").unwrap();
        fs::write(dev_share.join(LEGACY_MARKER_NAME), bin.to_string_lossy().as_bytes()).unwrap();

        std::env::set_var("HOME", home);
        assert_eq!(dev_session_binary(), Some(bin));
        assert!(dev_session_marker_path().is_file());
        assert!(!legacy_dev_session_marker_path().exists());
    }
}
