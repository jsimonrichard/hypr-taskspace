//! Linux boot identity — used to archive active tasks after reboot/crash.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{TskError, Result};
use crate::xdg::ensure_parent;

const BOOT_ID_PROC: &str = "/proc/sys/kernel/random/boot_id";
const LAST_BOOT_ID_FILE: &str = "last_boot_id";

/// Current kernel boot id (`/proc/sys/kernel/random/boot_id`), trimmed.
pub fn current_boot_id() -> Result<String> {
    read_boot_id_from(Path::new(BOOT_ID_PROC))
}

pub fn last_boot_id_path(data_dir: &Path) -> PathBuf {
    data_dir.join(LAST_BOOT_ID_FILE)
}

pub fn read_stored_boot_id(data_dir: &Path) -> Result<Option<String>> {
    let path = last_boot_id_path(data_dir);
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(&path).map_err(|source| TskError::Read {
        path: path.clone(),
        source,
    })?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        Ok(None)
    } else {
        Ok(Some(trimmed.to_string()))
    }
}

pub fn write_stored_boot_id(data_dir: &Path, boot_id: &str) -> Result<()> {
    let path = last_boot_id_path(data_dir);
    ensure_parent(&path)?;
    fs::write(&path, format!("{boot_id}\n")).map_err(|source| TskError::Write {
        path,
        source,
    })
}

fn read_boot_id_from(path: &Path) -> Result<String> {
    let raw = fs::read_to_string(path).map_err(|source| TskError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(TskError::Other(format!(
            "empty boot id at {}",
            path.display()
        )));
    }
    Ok(trimmed.to_string())
}

/// Whether this daemon start should run reboot archive recovery.
///
/// Returns `true` when there is no prior boot id (first run / upgrade after reboot —
/// treat as unclean so leftover active tasks are archived) or when the id changed.
pub fn is_new_boot(data_dir: &Path, current: &str) -> Result<bool> {
    match read_stored_boot_id(data_dir)? {
        None => Ok(true),
        Some(previous) => Ok(previous != current),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn new_boot_true_when_no_stored_id() {
        let dir = tempdir().unwrap();
        assert!(is_new_boot(dir.path(), "boot-a").unwrap());
    }

    #[test]
    fn new_boot_false_when_same_id() {
        let dir = tempdir().unwrap();
        write_stored_boot_id(dir.path(), "boot-a").unwrap();
        assert!(!is_new_boot(dir.path(), "boot-a").unwrap());
    }

    #[test]
    fn new_boot_true_when_id_changed() {
        let dir = tempdir().unwrap();
        write_stored_boot_id(dir.path(), "boot-a").unwrap();
        assert!(is_new_boot(dir.path(), "boot-b").unwrap());
    }

    #[test]
    fn roundtrip_stored_boot_id() {
        let dir = tempdir().unwrap();
        write_stored_boot_id(dir.path(), "abc-123").unwrap();
        assert_eq!(
            read_stored_boot_id(dir.path()).unwrap().as_deref(),
            Some("abc-123")
        );
    }
}
