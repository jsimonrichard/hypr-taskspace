//! Installation health checks for `lae doctor`.

use std::fs;
use std::path::Path;
use std::process::Command;

use serde_json::Value;

use crate::config::LaeConfig;
use crate::error::Result;
use crate::hyprland;
use crate::hyprland_events::diagnose_socket2;
use crate::install::{install_hypr_status, install_waybar_status, manifest};
use crate::install::waybar::CFFI_MODULE;

#[derive(Debug, Clone)]
pub struct DoctorCheck {
    pub label: String,
    pub passed: bool,
    pub detail: String,
}

pub fn run_doctor_checks(cfg: &LaeConfig) -> Result<Vec<DoctorCheck>> {
    let mut checks = Vec::new();
    let hypr = install_hypr_status(cfg)?;
    let waybar = install_waybar_status(cfg)?;

    checks.push(DoctorCheck {
        label: "Hyprland bindings installed".into(),
        passed: hypr
            .get("bindings_exist")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        detail: cfg
            .install_hypr_share_dir
            .join("hypr/bindings.conf")
            .display()
            .to_string(),
    });

    checks.push(DoctorCheck {
        label: "Walker task menu (elephant) installed".into(),
        passed: hypr
            .get("elephant_menu_exist")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
            && hypr
                .get("elephant_symlink")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        detail: crate::xdg::config_home()
            .join("elephant/menus/lae_tasks.lua")
            .display()
            .to_string(),
    });

    checks.push(DoctorCheck {
        label: "hyprland.conf contains lae source line".into(),
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

    let lae_bin = cfg.install_hypr_share_dir.join("bin/lae");
    checks.push(DoctorCheck {
        label: "Rust CLI installed".into(),
        passed: lae_bin.is_file(),
        detail: lae_bin.display().to_string(),
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
        label: format!("Waybar module ({CFFI_MODULE}) installed"),
        passed: waybar
            .get("installed")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        detail: cfg
            .install_hypr_share_dir
            .join("lib/liblae_waybar.so")
            .display()
            .to_string(),
    });

    checks.push(DoctorCheck {
        label: "SUPER+1 runs lae workspace go (not Omarchy)".into(),
        passed: super_one_is_lae(),
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

    let legacy_daemon = legacy_python_daemon_running();
    checks.push(DoctorCheck {
        label: "No legacy Python lae daemon".into(),
        passed: !legacy_daemon,
        detail: if legacy_daemon {
            "run: pkill -f 'lae.cli.daemon' — stale daemon overwrites Rust CLI state".into()
        } else {
            "ok".into()
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
            .unwrap_or_else(|_| "XDG_RUNTIME_DIR/lae/state-events.sock".into()),
    });

    Ok(checks)
}

fn install_backup_status(cfg: &LaeConfig) -> (bool, String) {
    let Ok(Some(m)) = manifest::load_manifest(&cfg.install_hypr_share_dir, "hypr") else {
        return (false, "no manifest".into());
    };
    let backup_dir = Path::new(&m.backup_dir);
    let ok = backup_dir.is_dir() && fs::read_dir(backup_dir).ok().is_some_and(|mut d| d.next().is_some());
    (ok, backup_dir.display().to_string())
}

fn super_one_is_lae() -> bool {
    if !hyprland::available() {
        return false;
    }
    let Ok(binds) = hyprland::hyprctl_json(&["binds"]) else {
        return false;
    };
    let Some(items) = binds.as_array() else {
        return false;
    };
    let lae_binds = items.iter().filter(|b| bind_runs_lae_workspace_go(b)).count();
    let omarchy_binds = items
        .iter()
        .filter(|b| bind_is_omarchy_workspace_digit(b))
        .count();
    lae_binds > 0 && omarchy_binds == 0
}

fn bind_runs_lae_workspace_go(bind: &Value) -> bool {
    bind.get("keycode").and_then(|v| v.as_i64()) == Some(10)
        && bind.get("modmask").and_then(|v| v.as_i64()) == Some(64)
        && bind
            .get("arg")
            .and_then(|v| v.as_str())
            .is_some_and(|arg| arg.contains("lae workspace go") || arg.contains("lae desktop go"))
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

fn legacy_python_daemon_running() -> bool {
    Command::new("pgrep")
        .args(["-f", "lae.cli.daemon"])
        .output()
        .is_ok_and(|o| o.status.success())
}
