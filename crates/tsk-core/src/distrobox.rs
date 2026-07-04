//! Distrobox container helpers — stop/remove only; create remains in Python for now.

use std::process::Command;

use crate::error::{TskError, Result};

pub fn available() -> bool {
    which::which("distrobox").is_ok()
}

pub fn container_exists(name: &str) -> bool {
    if !available() {
        return false;
    }
    let Ok(output) = Command::new("distrobox")
        .args(["list", "--no-color"])
        .output()
    else {
        return false;
    };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .any(|line| line.split_whitespace().next() == Some(name))
}

pub fn stop_container(name: &str) -> Result<()> {
    if !available() || !container_exists(name) {
        return Ok(());
    }
    let output = Command::new("distrobox")
        .args(["stop", "--name", name])
        .output()
        .map_err(|e| TskError::Other(format!("distrobox stop failed: {e}")))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(TskError::Other(format!(
            "distrobox stop {} failed: {}",
            name,
            String::from_utf8_lossy(&output.stderr).trim()
        )))
    }
}

pub fn remove_container(name: &str) -> Result<()> {
    if !available() {
        return Ok(());
    }
    if !container_exists(name) {
        return Ok(());
    }
    // Stop first — `distrobox rm` fails on a running container.
    let _ = stop_container(name);
    let output = Command::new("distrobox")
        .args(["rm", "--name", name, "-Y"])
        .output()
        .map_err(|e| TskError::Other(format!("distrobox rm failed: {e}")))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(TskError::Other(format!(
            "distrobox rm {} failed: {}",
            name,
            String::from_utf8_lossy(&output.stderr).trim()
        )))
    }
}

mod which {
    use std::path::PathBuf;

    pub fn which(name: &str) -> Result<PathBuf, ()> {
        std::env::split_paths(&std::env::var_os("PATH").ok_or(())?)
            .find_map(|dir| {
                let path = dir.join(name);
                path.is_file().then_some(path)
            })
            .ok_or(())
    }
}
