//! Symlink the Rust CLI onto PATH at `~/.local/bin/tsk`.

use std::fs;
use std::path::{Path, PathBuf};

use crate::config::TskConfig;
use crate::error::{TskError, Result};
use crate::xdg::{ensure_parent, user_bin_dir};

pub fn install_path_symlink(_cfg: &TskConfig, rust_bin: &Path) -> Result<PathBuf> {
    let link = user_bin_dir().join("tsk");
    ensure_parent(&link)?;

    if link.exists() || link.is_symlink() {
        if path_points_at_rust_bin(&link, rust_bin)? {
            return Ok(link);
        }
        fs::remove_file(&link).map_err(|source| TskError::Write {
            path: link.clone(),
            source,
        })?;
    }

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(rust_bin, &link).map_err(|source| TskError::Write {
            path: link.clone(),
            source,
        })?;
    }
    #[cfg(not(unix))]
    {
        fs::copy(rust_bin, &link).map_err(|source| TskError::Write {
            path: link.clone(),
            source,
        })?;
    }

    Ok(link)
}

pub fn remove_path_symlink(rust_bin: &Path) -> Result<()> {
    let link = user_bin_dir().join("tsk");
    if !link.is_symlink() {
        return Ok(());
    }
    if path_points_at_rust_bin(&link, rust_bin)? {
        let _ = fs::remove_file(link);
    }
    Ok(())
}

/// First `tsk` executable found by walking `PATH`.
pub fn path_tsk_binary() -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        for dir in std::env::split_paths(&paths) {
            let candidate = dir.join("tsk");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        None
    })
}

pub fn path_tsk_is_rust(cfg: &TskConfig) -> (bool, String) {
    let expected = cfg.install_hypr_share_dir.join("bin/tsk");
    let Some(found) = path_tsk_binary() else {
        return (
            false,
            format!(
                "no tsk on PATH — symlink created at {}; ensure {} is on PATH",
                user_bin_dir().join("tsk").display(),
                user_bin_dir().display()
            ),
        );
    };

    if path_points_at_rust_bin(&found, &expected).unwrap_or(false) || found == expected {
        return (true, found.display().to_string());
    }

    let detail = format!(
        "PATH tsk is {} (expected {})",
        found.display(),
        expected.display()
    );
    (false, detail)
}

fn path_points_at_rust_bin(link: &Path, rust_bin: &Path) -> Result<bool> {
    if link == rust_bin {
        return Ok(true);
    }
    if !link.is_symlink() {
        return Ok(false);
    }
    let target = fs::read_link(link).map_err(|source| TskError::Read {
        path: link.to_path_buf(),
        source,
    })?;
    Ok(normalize_path(&target) == normalize_path(rust_bin))
}

fn normalize_path(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}
