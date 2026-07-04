//! Install and manage the tsk daemon as a user systemd service.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use serde_json::{json, Value};

use crate::config::TskConfig;
use crate::error::{Result, TskError};
use crate::share::packaged_systemd_unit_installed;
use crate::share::packaged_systemd_unit_path;
use crate::xdg::{config_home, ensure_parent};

pub const SERVICE_NAME: &str = "tskd.service";

#[derive(Debug, Clone, Default)]
pub struct InstallSystemdOptions {
    pub dry_run: bool,
    /// Run `systemctl --user enable` so the unit starts with the graphical session.
    pub enable: bool,
    /// Run `systemctl --user start` after install when possible.
    pub start: bool,
}

pub fn user_service_unit_path() -> PathBuf {
    config_home().join("systemd/user").join(SERVICE_NAME)
}

/// Active unit file — user override, else packaged `/usr/lib/systemd/user/tskd.service`.
pub fn service_unit_path() -> PathBuf {
    let user = user_service_unit_path();
    if user.is_file() {
        user
    } else {
        packaged_systemd_unit_path()
    }
}

pub fn is_systemd_unit_installed() -> bool {
    user_service_unit_path().is_file() || packaged_systemd_unit_installed()
}

pub fn render_service_unit(_cfg: &TskConfig) -> String {
    include_str!("../../../../share/systemd/tskd.service").replace("@TSK_CMD@", "tsk")
}

pub fn install_systemd(cfg: &TskConfig, options: &InstallSystemdOptions) -> Result<Vec<String>> {
    if packaged_systemd_unit_installed() && !user_service_unit_path().is_file() {
        let unit_path = packaged_systemd_unit_path();
        if options.dry_run {
            return Ok(vec![
                format!("would use packaged unit at {}", unit_path.display()),
                if options.enable {
                    format!("would run: systemctl --user enable {SERVICE_NAME}")
                } else {
                    format!("would skip: systemctl --user enable {SERVICE_NAME}")
                },
                if options.start {
                    format!("would run: systemctl --user start {SERVICE_NAME}")
                } else {
                    format!("would skip: systemctl --user start {SERVICE_NAME}")
                },
            ]);
        }

        systemctl(&["daemon-reload"])?;
        if options.enable {
            systemctl(&["enable", SERVICE_NAME])?;
        }
        if options.start {
            let _ = systemctl(&["start", SERVICE_NAME]);
        }
        return Ok(vec![format!("using packaged unit at {}", unit_path.display())]);
    }

    let unit_path = user_service_unit_path();
    let unit_body = render_service_unit(cfg);

    if options.dry_run {
        return Ok(vec![
            format!("would write {}", unit_path.display()),
            if options.enable {
                format!("would run: systemctl --user enable {SERVICE_NAME}")
            } else {
                format!("would skip: systemctl --user enable {SERVICE_NAME}")
            },
            if options.start {
                format!("would run: systemctl --user start {SERVICE_NAME}")
            } else {
                format!("would skip: systemctl --user start {SERVICE_NAME}")
            },
        ]);
    }

    ensure_parent(&unit_path)?;
    fs::write(&unit_path, unit_body).map_err(|source| TskError::Write {
        path: unit_path.clone(),
        source,
    })?;

    systemctl(&["daemon-reload"])?;

    if options.enable {
        systemctl(&["enable", SERVICE_NAME])?;
    }

    if options.start {
        let _ = systemctl(&["start", SERVICE_NAME]);
    }

    Ok(vec![format!("installed {}", unit_path.display())])
}

pub fn uninstall_systemd(_cfg: &TskConfig) -> Result<Vec<String>> {
    if packaged_systemd_unit_installed() && !user_service_unit_path().is_file() {
        let mut actions = Vec::new();
        let _ = systemctl(&["stop", SERVICE_NAME]);
        let _ = systemctl(&["disable", SERVICE_NAME]);
        actions.push(format!(
            "disabled packaged unit ({}) — remove the hypr-taskspace package to uninstall the unit file",
            packaged_systemd_unit_path().display()
        ));
        return Ok(actions);
    }

    let unit_path = user_service_unit_path();
    let mut actions = Vec::new();

    if unit_path.is_file() {
        let _ = systemctl(&["stop", SERVICE_NAME]);
        let _ = systemctl(&["disable", SERVICE_NAME]);
        fs::remove_file(&unit_path).map_err(|source| TskError::Write {
            path: unit_path.clone(),
            source,
        })?;
        let _ = systemctl(&["daemon-reload"]);
        actions.push(format!("removed {}", unit_path.display()));
    }

    Ok(actions)
}

pub fn install_systemd_status(_cfg: &TskConfig) -> Result<Value> {
    let unit_path = service_unit_path();
    let installed = is_systemd_unit_installed();
    let packaged = packaged_systemd_unit_installed() && !user_service_unit_path().is_file();

    Ok(json!({
        "installed": installed,
        "unit_path": unit_path.display().to_string(),
        "packaged": packaged,
        "enabled": installed && systemctl_is_enabled().unwrap_or(false),
        "active": installed && systemctl_is_active().unwrap_or(false),
    }))
}

pub fn systemd_start() -> Result<()> {
    systemctl(&["start", SERVICE_NAME])
}

pub fn systemd_stop() -> Result<()> {
    systemctl(&["stop", SERVICE_NAME])
}

pub fn systemd_restart() -> Result<()> {
    systemctl(&["restart", SERVICE_NAME])
}

pub fn systemctl_is_active() -> Result<bool> {
    Ok(systemctl_output(&["is-active", SERVICE_NAME])?.trim() == "active")
}

pub fn systemctl_is_enabled() -> Result<bool> {
    let state = systemctl_output(&["is-enabled", SERVICE_NAME])?;
    Ok(matches!(state.trim(), "enabled" | "static" | "linked"))
}

fn systemctl(args: &[&str]) -> Result<()> {
    let status = Command::new("systemctl")
        .arg("--user")
        .args(args)
        .status()
        .map_err(|e| TskError::Other(format!("failed to run systemctl: {e}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(TskError::Other(format!(
            "systemctl --user {} failed (exit {status})",
            args.join(" ")
        )))
    }
}

fn systemctl_output(args: &[&str]) -> Result<String> {
    let output = Command::new("systemctl")
        .arg("--user")
        .args(args)
        .output()
        .map_err(|e| TskError::Other(format!("failed to run systemctl: {e}")))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(TskError::Other(format!(
            "systemctl --user {} failed (exit {})",
            args.join(" "),
            output.status
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_service_unit_uses_tsk_on_path() {
        let body = render_service_unit(&TskConfig::default());
        assert!(body.contains("ExecStart=tsk daemon run"));
        assert!(body.contains("WantedBy=graphical-session.target"));
        assert!(body.contains("PassEnvironment=WAYLAND_DISPLAY"));
    }
}
