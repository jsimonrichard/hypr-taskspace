//! Installation health checks for `tsk doctor`.

use std::fs;
use std::path::Path;

use serde_json::Value;

use crate::config::TskConfig;
use crate::daemon_socket_path;
use crate::error::Result;
use crate::is_daemon_running;
use crate::hyprland;
use crate::hyprland_events::diagnose_socket2;
use crate::install::{
    install_hypr_status, install_systemd_status, install_walker_status, install_waybar_status, manifest,
};
use crate::install::waybar::CFFI_MODULE;
use crate::share::{effective_share_dir, uses_packaged_share};

#[derive(Debug, Clone)]
pub struct DoctorCheck {
    pub label: String,
    pub passed: bool,
    pub detail: String,
}

pub fn run_doctor_checks(cfg: &TskConfig) -> Result<Vec<DoctorCheck>> {
    let mut checks = Vec::new();
    let hypr = install_hypr_status(cfg)?;
    let waybar = install_waybar_status(cfg)?;

    let share = effective_share_dir(cfg);
    checks.push(DoctorCheck {
        label: "Hyprland bindings installed".into(),
        passed: hypr
            .get("bindings_exist")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        detail: share.join("hypr/bindings.conf").display().to_string(),
    });

    checks.push(DoctorCheck {
        label: "hyprland.conf contains tsk source line".into(),
        passed: hypr
            .get("source_line_present")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        detail: cfg.install_hypr_config_path.display().to_string(),
    });

    let (backup_ok, backup_msg) = install_backup_status(cfg);
    checks.push(DoctorCheck {
        label: "Install backup exists".into(),
        passed: backup_ok,
        detail: backup_msg,
    });

    let (path_ok, path_detail) = crate::binary::path_tsk_is_usable(cfg);
    checks.push(DoctorCheck {
        label: "tsk on PATH".into(),
        passed: path_ok,
        detail: path_detail,
    });

    checks.push(DoctorCheck {
        label: "Waybar CFFI module configured".into(),
        passed: waybar
            .get("cffi_module_present")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        detail: waybar
            .get("config_path")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
    });

    checks.push(DoctorCheck {
        label: "Runtime data directory".into(),
        passed: true,
        detail: cfg.data_dir.display().to_string(),
    });

    if uses_packaged_share(cfg) {
        checks.push(DoctorCheck {
            label: "System share (package)".into(),
            passed: share.join("hypr/bindings.conf").is_file(),
            detail: share.display().to_string(),
        });
    }

    let module_path = share.join("lib/libtsk_waybar.so");
    let module_ok = crate::binary::is_usable_cdylib(&module_path);
    checks.push(DoctorCheck {
        label: format!("Waybar module ({CFFI_MODULE}) installed"),
        passed: waybar
            .get("installed")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
            && module_ok,
        detail: if module_ok {
            module_path.display().to_string()
        } else if module_path.is_file() {
            format!(
                "{} (empty or corrupt — run: scripts/install-user-share.sh or reinstall the package)",
                module_path.display()
            )
        } else {
            format!(
                "{} (missing — run: scripts/install-user-share.sh or reinstall the package)",
                module_path.display()
            )
        },
    });

    let walker = install_walker_status(cfg)?;
    checks.push(DoctorCheck {
        label: "Walker Elephant launch_prefix".into(),
        passed: walker
            .get("installed")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
            && walker
                .get("launch_prefix_set")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        detail: walker
            .get("config_path")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
    });

    checks.push(DoctorCheck {
        label: "SUPER+1 runs tsk workspace switch (not Omarchy)".into(),
        passed: super_one_is_tsk(),
        detail: "hyprctl binds -j".into(),
    });

    let socket2 = diagnose_socket2();
    checks.push(DoctorCheck {
        label: "Hyprland socket2 event socket".into(),
        passed: socket2.available,
        detail: socket2
            .path
            .map(|p| p.display().to_string())
            .unwrap_or(socket2.reason),
    });

    let daemon_running = is_daemon_running();
    let systemd = install_systemd_status(cfg).ok();
    let systemd_installed = systemd
        .as_ref()
        .and_then(|s| s.get("installed"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    checks.push(DoctorCheck {
        label: "TSK daemon running".into(),
        passed: daemon_running,
        detail: if daemon_running {
            daemon_socket_path()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| "ok".into())
        } else if systemd_installed {
            "run: systemctl --user start tskd.service (or log into your graphical session)".into()
        } else {
            "run: scripts/install-systemd.sh (recommended) or tsk daemon start".into()
        },
    });

    checks.push(DoctorCheck {
        label: "TSK daemon systemd unit".into(),
        passed: systemd_installed
            && systemd
                .as_ref()
                .and_then(|s| s.get("enabled"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        detail: if systemd_installed {
            let enabled = systemd
                .as_ref()
                .and_then(|s| s.get("enabled"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let active = systemd
                .as_ref()
                .and_then(|s| s.get("active"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            format!(
                "{} (enabled: {enabled}, active: {active})",
                systemd
                    .as_ref()
                    .and_then(|s| s.get("unit_path"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
            )
        } else {
            "run: scripts/install-systemd.sh".into()
        },
    });

    let state_events = crate::state_notify::state_events_socket_path()
        .map(|p| p.exists())
        .unwrap_or(false);
    checks.push(DoctorCheck {
        label: "State-events socket (Waybar bar updates)".into(),
        passed: state_events,
        detail: crate::state_notify::state_events_socket_path()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "XDG_RUNTIME_DIR/tsk/state-events.sock".into()),
    });

    Ok(checks)
}

fn install_backup_status(cfg: &TskConfig) -> (bool, String) {
    let Ok(Some(m)) = manifest::load_manifest(&cfg.data_dir, "hypr") else {
        return (false, "no manifest".into());
    };
    let backup_dir = Path::new(&m.backup_dir);
    let ok = backup_dir.is_dir() && fs::read_dir(backup_dir).ok().is_some_and(|mut d| d.next().is_some());
    (ok, backup_dir.display().to_string())
}

fn super_one_is_tsk() -> bool {
    if !hyprland::available() {
        return false;
    }
    let Ok(binds) = hyprland::hyprctl_json(&["binds"]) else {
        return false;
    };
    let Some(items) = binds.as_array() else {
        return false;
    };
    let tsk_binds = items.iter().filter(|b| bind_runs_tsk_workspace_go(b)).count();
    let omarchy_binds = items
        .iter()
        .filter(|b| bind_is_omarchy_workspace_digit(b))
        .count();
    tsk_binds > 0 && omarchy_binds == 0
}

fn bind_runs_tsk_workspace_go(bind: &Value) -> bool {
    bind.get("keycode").and_then(|v| v.as_i64()) == Some(10)
        && bind.get("modmask").and_then(|v| v.as_i64()) == Some(64)
        && bind
            .get("arg")
            .and_then(|v| v.as_str())
            .is_some_and(|arg| {
                arg.contains("workspace switch")
                    || arg.contains("tsk-workspace-switch")
                    || arg.contains("tsk workspace go")
            })
}

fn bind_is_omarchy_workspace_digit(bind: &Value) -> bool {
    let keycode = bind.get("keycode").and_then(|v| v.as_i64()).unwrap_or(-1);
    (10..=19).contains(&keycode)
        && bind.get("modmask").and_then(|v| v.as_i64()) == Some(64)
        && bind
            .get("dispatcher")
            .and_then(|v| v.as_str())
            .is_some_and(|d| d == "workspace")
}
