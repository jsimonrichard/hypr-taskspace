//! Install and manage the tsk daemon as a user systemd service.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use serde_json::{json, Value};

use crate::config::TskConfig;
use crate::error::{Result, TskError};
use crate::xdg::{config_home, ensure_parent};

pub const SERVICE_NAME: &str = "tskd.service";

const HYPR_EXEC_ONCE: &str = r#"exec-once = systemctl --user import-environment WAYLAND_DISPLAY XDG_CURRENT_DESKTOP HYPRLAND_INSTANCE_SIGNATURE; systemctl --user start --no-block tskd.service"#;

const HYPR_STUB: &str = r#"# Hypr Taskspace — systemd daemon hook (activated by `tsk install systemd`)
# exec-once = systemctl --user import-environment WAYLAND_DISPLAY XDG_CURRENT_DESKTOP HYPRLAND_INSTANCE_SIGNATURE; systemctl --user start --no-block tskd.service
"#;

#[derive(Debug, Clone, Default)]
pub struct InstallSystemdOptions {
    pub dry_run: bool,
    /// Run `systemctl --user enable` so the unit starts with the graphical session.
    pub enable: bool,
    /// Run `systemctl --user start` after install when possible.
    pub start: bool,
}

pub fn service_unit_path() -> PathBuf {
    config_home().join("systemd/user").join(SERVICE_NAME)
}

pub fn hypr_daemon_hook_path(cfg: &TskConfig) -> PathBuf {
    cfg.install_hypr_share_dir.join("hypr/daemon-systemd.conf")
}

pub fn is_systemd_unit_installed() -> bool {
    service_unit_path().is_file()
}

pub fn render_service_unit(cfg: &TskConfig) -> String {
    let tsk_bin = cfg.install_hypr_share_dir.join("bin/tsk");
    let template = include_str!("../../../../share/systemd/tskd.service");
    template.replace("@TSK_BIN@", &tsk_bin.display().to_string())
}

pub fn install_systemd(cfg: &TskConfig, options: &InstallSystemdOptions) -> Result<Vec<String>> {
    let unit_path = service_unit_path();
    let hook_path = hypr_daemon_hook_path(cfg);
    let unit_body = render_service_unit(cfg);

    if options.dry_run {
        return Ok(vec![
            format!("would write {}", unit_path.display()),
            format!("would write {}", hook_path.display()),
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

    ensure_parent(&hook_path)?;
    fs::write(&hook_path, format!("{HYPR_EXEC_ONCE}\n")).map_err(|source| TskError::Write {
        path: hook_path.clone(),
        source,
    })?;

    systemctl(&["daemon-reload"])?;

    if options.enable {
        systemctl(&["enable", SERVICE_NAME])?;
    }

    if options.start {
        let _ = systemctl(&["start", SERVICE_NAME]);
    }

    Ok(vec![
        format!("installed {}", unit_path.display()),
        format!("installed {}", hook_path.display()),
    ])
}

pub fn uninstall_systemd(cfg: &TskConfig) -> Result<Vec<String>> {
    let unit_path = service_unit_path();
    let hook_path = hypr_daemon_hook_path(cfg);
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

    if hook_path.is_file() {
        fs::write(&hook_path, HYPR_STUB).map_err(|source| TskError::Write {
            path: hook_path.clone(),
            source,
        })?;
        actions.push(format!("reset {}", hook_path.display()));
    }

    Ok(actions)
}

pub fn install_systemd_status(cfg: &TskConfig) -> Result<Value> {
    let unit_path = service_unit_path();
    let installed = unit_path.is_file();
    let hook_active = hypr_daemon_hook_path(cfg)
        .is_file()
        .then(|| fs::read_to_string(hypr_daemon_hook_path(cfg)).ok())
        .flatten()
        .is_some_and(|body| body.contains("exec-once = systemctl"));

    Ok(json!({
        "installed": installed,
        "unit_path": unit_path.display().to_string(),
        "hypr_hook_active": hook_active,
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
    fn render_service_unit_uses_configured_binary() {
        let mut cfg = TskConfig::default();
        cfg.install_hypr_share_dir = expand("~/.local/share/tsk");
        let body = render_service_unit(&cfg);
        assert!(body.contains("ExecStart=~/.local/share/tsk/bin/tsk daemon run")
            || body.contains(".local/share/tsk/bin/tsk daemon run"));
        assert!(body.contains("WantedBy=graphical-session.target"));
        assert!(body.contains("PassEnvironment=WAYLAND_DISPLAY"));
    }
}
