//! Active dev session marker — `~/.local/share/tsk/dev-session`.
//!
//! Any `tsk` process (shell, Hyprland exec, helpers) checks this file in the prod
//! data dir. When present, it contains the path to the dev build binary; config and
//! share paths are derived from the dev install tree. No environment variables required.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::config::load_dev_config;
use crate::config::load_prod_config;
use crate::daemon::ping_daemon_at;
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

/// Drop a leftover `dev-session` marker when prod `tskd` is up but the dev socket is not.
///
/// This happens when `scripts/dev.sh enter` is interrupted after the session file is written
/// (e.g. Ctrl+C during install) but before teardown runs — clients would otherwise keep
/// re-execing the dev build and pinging `~/.local/share/tsk-dev/daemon.sock` while systemd
/// serves prod on `~/.local/share/tsk/daemon.sock`.
pub fn reconcile_stale_dev_session() -> Result<()> {
    if !dev_session_active() {
        return Ok(());
    }

    let dev_cfg = load_dev_config()?;
    let dev_socket = dev_cfg.daemon_socket_path();
    if ping_daemon_at(&dev_socket)? {
        return Ok(());
    }

    let prod_cfg = load_prod_config()?;
    let prod_socket = prod_cfg.daemon_socket_path();
    if !ping_daemon_at(&prod_socket)? {
        remove_stale_daemon_runtime(&prod_socket);
    }

    if ping_daemon_at(&prod_socket)? {
        eprintln!(
            "note: clearing stale dev session — prod daemon is running at {}",
            prod_socket.display()
        );
        stop_dev_session()
    } else {
        Ok(())
    }
}

fn remove_stale_daemon_runtime(socket: &Path) {
    if socket.exists() {
        let _ = fs::remove_file(socket);
    }
    let pid = socket.with_file_name("daemon.pid");
    if pid.is_file() {
        let _ = fs::remove_file(pid);
    }
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
    fn reconcile_keeps_session_without_reachable_daemons() {
        let _lock = env_lock();
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let share = home.join(".local/share/tsk");
        fs::create_dir_all(&share).unwrap();
        let bin = share.join("dev-build");
        fs::write(&bin, b"").unwrap();

        std::env::set_var("HOME", home);
        stop_dev_session().ok();
        start_dev_session(&bin).unwrap();

        reconcile_stale_dev_session().unwrap();
        assert!(dev_session_active());

        stop_dev_session().ok();
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
