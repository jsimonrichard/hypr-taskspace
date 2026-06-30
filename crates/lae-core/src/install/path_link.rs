//! Symlink the Rust CLI onto PATH at `~/.local/bin/lae`.

use std::fs;
use std::path::{Path, PathBuf};

use crate::config::LaeConfig;
use crate::error::{LaeError, Result};
use crate::xdg::{ensure_parent, user_bin_dir};

pub fn install_path_symlink(_cfg: &LaeConfig, rust_bin: &Path) -> Result<PathBuf> {
    let link = user_bin_dir().join("lae");
    ensure_parent(&link)?;

    if link.exists() || link.is_symlink() {
        if path_points_at_rust_bin(&link, rust_bin)? {
            return Ok(link);
        }
        fs::remove_file(&link).map_err(|source| LaeError::Write {
            path: link.clone(),
            source,
        })?;
    }

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(rust_bin, &link).map_err(|source| LaeError::Write {
            path: link.clone(),
            source,
        })?;
    }
    #[cfg(not(unix))]
    {
        fs::copy(rust_bin, &link).map_err(|source| LaeError::Write {
            path: link.clone(),
            source,
        })?;
    }

    Ok(link)
}

pub fn remove_path_symlink(rust_bin: &Path) -> Result<()> {
    let link = user_bin_dir().join("lae");
    if !link.is_symlink() {
        return Ok(());
    }
    if path_points_at_rust_bin(&link, rust_bin)? {
        let _ = fs::remove_file(link);
    }
    Ok(())
}

/// First `lae` executable found by walking `PATH`.
pub fn path_lae_binary() -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        for dir in std::env::split_paths(&paths) {
            let candidate = dir.join("lae");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        None
    })
}

pub fn path_lae_is_rust(cfg: &LaeConfig) -> (bool, String) {
    let expected = cfg.install_hypr_share_dir.join("bin/lae");
    let Some(found) = path_lae_binary() else {
        return (
            false,
            format!(
                "no lae on PATH — symlink created at {}; ensure {} is on PATH",
                user_bin_dir().join("lae").display(),
                user_bin_dir().display()
            ),
        );
    };

    if path_points_at_rust_bin(&found, &expected).unwrap_or(false) || found == expected {
        return (true, found.display().to_string());
    }

    let detail = if is_python_script(&found) {
        format!(
            "{} is the legacy Python CLI — run: pip uninstall local-agentic-env, then `lae install hypr`",
            found.display()
        )
    } else {
        format!(
            "PATH lae is {} (expected {})",
            found.display(),
            expected.display()
        )
    };
    (false, detail)
}

fn path_points_at_rust_bin(link: &Path, rust_bin: &Path) -> Result<bool> {
    if link == rust_bin {
        return Ok(true);
    }
    if !link.is_symlink() {
        return Ok(false);
    }
    let target = fs::read_link(link).map_err(|source| LaeError::Read {
        path: link.to_path_buf(),
        source,
    })?;
    Ok(normalize_path(&target) == normalize_path(rust_bin))
}

fn normalize_path(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn is_python_script(path: &Path) -> bool {
    let lossy = path.to_string_lossy();
    if lossy.contains("/python")
        || lossy.contains("site-packages")
        || lossy.contains(".local/share/mise/installs/python")
    {
        return true;
    }
    let Ok(mut file) = fs::File::open(path) else {
        return false;
    };
    use std::io::Read;
    let mut head = [0u8; 128];
    let Ok(n) = file.read(&mut head) else {
        return false;
    };
    let prefix = String::from_utf8_lossy(&head[..n]);
    prefix.starts_with("#!") && prefix.contains("python")
}
