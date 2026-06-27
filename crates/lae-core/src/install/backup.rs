use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::{LaeError, Result};
use crate::xdg::ensure_parent;

pub fn backup_timestamp() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // UTC-ish timestamp compatible with Python backup dirs
    chrono::DateTime::from_timestamp(secs as i64, 0)
        .map(|dt| dt.format("%Y-%m-%dT%H%M%S").to_string())
        .unwrap_or_else(|| format!("{secs}"))
}

pub fn backup_file(source: &Path, backup_root: &Path) -> Result<PathBuf> {
    ensure_parent(&backup_root.join("_"))?;
    fs::create_dir_all(backup_root).map_err(|source| LaeError::Write {
        path: backup_root.to_path_buf(),
        source,
    })?;
    let dest = backup_root.join(source.file_name().ok_or_else(|| {
        LaeError::Other("backup source has no file name".into())
    })?);
    fs::copy(source, &dest).map_err(|source| LaeError::Write {
        path: dest.clone(),
        source,
    })?;
    Ok(dest)
}

pub fn restore_file(backup: &Path, destination: &Path) -> Result<()> {
    ensure_parent(destination)?;
    fs::copy(backup, destination).map_err(|source| LaeError::Write {
        path: destination.to_path_buf(),
        source,
    })?;
    Ok(())
}
