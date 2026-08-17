//! Installation health checks for `tsk doctor`.

use std::fs;
use std::path::Path;

use crate::config::TskConfig;
use crate::daemon_socket_path;
use crate::error::Result;
use crate::hyprland;
use crate::hyprland_events::diagnose_socket2;
use crate::install::detect::chromium_present;
use crate::install::waybar::CFFI_MODULE;
use crate::install::{
    install_chromium_status, install_hypr_status, install_systemd_status, install_walker_status,
    install_waybar_status, manifest,
};
use crate::is_daemon_running;
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
    let quattro = crate::install::detect::quattro_hypr_present()
        || crate::install::plugin::omarchy_shell_present();
    checks.push(DoctorCheck {
        label: if quattro {
            "Hyprland Lua bindings installed".into()
        } else {
            "Hyprland bindings installed".into()
        },
        passed: hypr
            .get("bindings_exist")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        detail: hypr
            .get("bindings_path")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
    });

    checks.push(DoctorCheck {
        label: if quattro {
            "bindings.lua contains tsk dofile".into()
        } else {
            "hyprland.conf contains tsk source line".into()
        },
        passed: hypr
            .get("source_line_present")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        detail: hypr
            .get("config_path")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
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

    let opener = crate::task_env::url_opener_path(cfg);
    checks.push(DoctorCheck {
        label: "Taskspace URL opener (tsk-open)".into(),
        passed: opener.is_file(),
        detail: if opener.is_file() {
            opener.display().to_string()
        } else {
            format!(
                "{} missing — run `tsk install all` (or `tsk install omarchy`)",
                opener.display()
            )
        },
    });

    if let Some(check) = editor_external_browser_check(&opener) {
        checks.push(check);
    }

    if quattro {
        push_omarchy_shell_checks(&mut checks, cfg);
    } else {
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
    }

    checks.push(DoctorCheck {
        label: "Runtime data directory".into(),
        passed: true,
        detail: cfg.data_dir.display().to_string(),
    });

    if uses_packaged_share(cfg) {
        let bindings_ok = if quattro {
            share.join("hypr/omarchy.lua").is_file()
        } else {
            share.join("hypr/bindings.conf").is_file()
        };
        checks.push(DoctorCheck {
            label: "System share (package)".into(),
            passed: bindings_ok,
            detail: share.display().to_string(),
        });
    }

    if !quattro {
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
        let walker_ok = walker
            .get("installed")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
            && walker
                .get("launch_prefix_set")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
        let walker_path = walker
            .get("config_path")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        checks.push(DoctorCheck {
            label: "Walker Elephant launch_prefix".into(),
            passed: walker_ok,
            detail: if walker_ok {
                walker_path.to_string()
            } else {
                let expected = walker
                    .get("expected_launch_prefix")
                    .and_then(|v| v.as_str())
                    .unwrap_or("/usr/bin/tsk walker exec --");
                format!("{walker_path} — expected launch_prefix = \"{expected}\"")
            },
        });
    }

    if chromium_present() {
        let chromium = install_chromium_status(cfg)?;
        let ok = chromium
            .get("installed")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        checks.push(DoctorCheck {
            label: "Chromium helper extension".into(),
            passed: ok,
            detail: if ok {
                chromium
                    .get("external_json")
                    .and_then(|v| v.as_str())
                    .unwrap_or("installed")
                    .to_string()
            } else {
                "Chromium detected — run `tsk install chromium` (or `tsk install all`)".into()
            },
        });
    }

    checks.push(DoctorCheck {
        label: "SUPER+1 runs tsk workspace switch (not Omarchy)".into(),
        passed: super_one_is_tsk(),
        detail: super_one_detail(),
    });

    if quattro {
        checks.push(DoctorCheck {
            label: "Browser keys run tsk launch (not omarchy-launch-browser)".into(),
            passed: browser_launch_is_tsk(),
            detail: browser_launch_detail(),
        });
    }

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

    let daemons = crate::running_daemon_processes();
    checks.push(DoctorCheck {
        label: "Single tsk daemon process".into(),
        passed: daemons.len() <= 1,
        detail: if daemons.len() <= 1 {
            daemons
                .first()
                .map(|p| format!("pid {} ({})", p.pid, p.cmdline))
                .unwrap_or_else(|| "none".into())
        } else {
            format!(
                "{} processes: {}",
                daemons.len(),
                daemons
                    .iter()
                    .map(|p| format!("pid {} ({})", p.pid, p.cmdline))
                    .collect::<Vec<_>>()
                    .join("; ")
            )
        },
    });

    let stale_daemon = daemons.iter().any(|p| p.deleted_exe);
    checks.push(DoctorCheck {
        label: "Daemon binary matches /usr/bin/tsk".into(),
        passed: !stale_daemon,
        detail: if stale_daemon {
            "running daemon is a replaced binary — run: systemctl --user restart tskd.service"
                .into()
        } else {
            daemons
                .first()
                .map(|p| format!("pid {}", p.pid))
                .unwrap_or_else(|| "no daemon".into())
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

    if quattro {
        let runtime = crate::xdg::tsk_runtime_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "XDG_RUNTIME_DIR/tsk".into());
        checks.push(DoctorCheck {
            label: "Bar refresh (state.rev / tsk bar status --json)".into(),
            passed: crate::xdg::tsk_runtime_dir()
                .map(|p| p.is_dir())
                .unwrap_or(false),
            detail: runtime,
        });
    } else {
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
    }

    Ok(checks)
}

fn editor_external_browser_check(opener: &Path) -> Option<DoctorCheck> {
    let path = crate::xdg::config_home().join("Cursor/User/settings.json");
    if !path.is_file() {
        return None;
    }
    let raw = fs::read_to_string(&path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let current = value
        .get("workbench.externalBrowser")
        .and_then(|v| v.as_str());
    let expected = opener.to_string_lossy();
    let passed = opener.is_file() && current == Some(expected.as_ref());
    Some(DoctorCheck {
        label: "Cursor opens taskspace links via tsk-open".into(),
        passed,
        detail: match current {
            Some(v) if passed => v.to_string(),
            Some(v) => format!("{v} — run `tsk install all` to use {expected}"),
            None => format!("unset — run `tsk install all` to set workbench.externalBrowser"),
        },
    })
}

fn push_omarchy_shell_checks(checks: &mut Vec<DoctorCheck>, cfg: &TskConfig) {
    let plugin_dir = crate::install::plugin::plugin_install_dir();
    let control_ui = crate::install::plugin::load_control_ui(cfg)
        .unwrap_or(crate::install::plugin::ControlUi::Shell);
    checks.push(DoctorCheck {
        label: "Omarchy plugin tsk.taskspace".into(),
        passed: plugin_dir.is_dir() && crate::install::plugin::plugin_enabled_in_shell_json(),
        detail: plugin_dir.display().to_string(),
    });
    checks.push(DoctorCheck {
        label: "Omarchy control UI".into(),
        passed: true,
        detail: match control_ui {
            crate::install::plugin::ControlUi::Tui => {
                "tui (SUPER+Tab / bar open tsk task tui-launch)".into()
            }
            crate::install::plugin::ControlUi::Shell => "shell overlay (tsk.taskspace)".into(),
        },
    });
    if control_ui.includes_overlay() {
        checks.push(DoctorCheck {
            label: "Omarchy overlay Taskspace.qml".into(),
            passed: crate::install::plugin::overlay_installed(),
            detail: crate::install::plugin::overlay_qml_path()
                .display()
                .to_string(),
        });
    }
    checks.push(DoctorCheck {
        label: "omarchy.workspaces not in left bar".into(),
        passed: !crate::install::plugin::workspaces_in_left_layout(),
        detail: crate::install::plugin::shell_json_path()
            .display()
            .to_string(),
    });
    checks.push(DoctorCheck {
        label: "Cloned menu uses tsk launch".into(),
        passed: crate::install::plugin::menu_launch_patched(),
        detail: crate::install::plugin::cloned_menu_dir()
            .join("Menu.qml")
            .display()
            .to_string(),
    });
}

/// Default doctor output is failures only. Verbose prints every check.
/// All-pass (non-verbose) is a single `ok` line.
pub fn format_doctor_report(checks: &[DoctorCheck], verbose: bool) -> String {
    let mut lines = Vec::new();
    for check in checks {
        if check.passed && !verbose {
            continue;
        }
        let mark = if check.passed { "ok" } else { "FAIL" };
        lines.push(format!("[{mark}] {}: {}", check.label, check.detail));
    }
    if lines.is_empty() {
        "ok".into()
    } else {
        lines.join("\n")
    }
}

fn install_backup_status(cfg: &TskConfig) -> (bool, String) {
    let Ok(Some(m)) = manifest::load_manifest(&cfg.data_dir, "hypr") else {
        return (false, "no manifest".into());
    };
    let backup_dir = Path::new(&m.backup_dir);
    let ok = backup_dir.is_dir()
        && fs::read_dir(backup_dir)
            .ok()
            .is_some_and(|mut d| d.next().is_some());
    (ok, backup_dir.display().to_string())
}

fn super_one_is_tsk() -> bool {
    super_one_counts().is_some_and(|(tsk, omarchy)| tsk > 0 && omarchy == 0)
}

fn super_one_detail() -> String {
    match super_one_counts() {
        None if !hyprland::available() => "hyprctl unavailable".into(),
        None => "hyprctl binds failed".into(),
        Some((tsk_binds, omarchy_binds)) if tsk_binds > 0 && omarchy_binds == 0 => {
            "hyprctl binds".into()
        }
        Some((tsk_binds, omarchy_binds)) => format!(
            "tsk workspace binds: {tsk_binds}, Omarchy workspace binds still active: {omarchy_binds} — unbind in omarchy.lua before tsk binds"
        ),
    }
}

fn super_one_counts() -> Option<(usize, usize)> {
    if !hyprland::available() {
        return None;
    }
    let text = hyprland::hyprctl_output(&["binds"]).ok()?;
    let binds = parse_hyprctl_binds(&text);
    if binds.is_empty() {
        return None;
    }
    let tsk_binds = binds
        .iter()
        .filter(|b| bind_runs_tsk_workspace_switch(b))
        .count();
    let omarchy_binds = binds
        .iter()
        .filter(|b| bind_is_omarchy_workspace_digit(b))
        .count();
    Some((tsk_binds, omarchy_binds))
}

#[derive(Debug, Default, Clone)]
struct HyprBind {
    modmask: i64,
    keycode: i64,
    key: String,
    description: String,
    dispatcher: String,
    arg: String,
}

fn parse_hyprctl_binds(text: &str) -> Vec<HyprBind> {
    let mut binds = Vec::new();
    let mut current: Option<HyprBind> = None;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("bind") && !trimmed.contains(':') {
            if let Some(bind) = current.take() {
                binds.push(bind);
            }
            current = Some(HyprBind::default());
            continue;
        }
        let Some(bind) = current.as_mut() else {
            continue;
        };
        if let Some(value) = trimmed.strip_prefix("modmask: ") {
            bind.modmask = value.parse().unwrap_or(-1);
        } else if let Some(value) = trimmed.strip_prefix("keycode: ") {
            bind.keycode = value.parse().unwrap_or(-1);
        } else if let Some(value) = trimmed.strip_prefix("key: ") {
            bind.key = value.to_string();
        } else if let Some(value) = trimmed.strip_prefix("description: ") {
            bind.description = value.to_string();
        } else if let Some(value) = trimmed.strip_prefix("dispatcher: ") {
            bind.dispatcher = value.to_string();
        } else if let Some(value) = trimmed.strip_prefix("arg: ") {
            bind.arg = value.to_string();
        }
    }
    if let Some(bind) = current {
        binds.push(bind);
    }
    binds
}

fn is_super_one(bind: &HyprBind) -> bool {
    bind.modmask == 64
        && (bind.keycode == 10
            || bind.key.ends_with("code:10")
            || bind.key == "1"
            || bind.key.ends_with(" + 1"))
}

fn bind_runs_tsk_workspace_switch(bind: &HyprBind) -> bool {
    is_super_one(bind)
        && (bind.arg.contains("workspace switch")
            || bind.description.contains("Taskspace workspace 1"))
}

fn bind_runs_tsk_launch_chromium(bind: &HyprBind) -> bool {
    (bind.arg.contains("launch") && bind.arg.contains("chromium.desktop"))
        || bind.description.contains("Taskspace launch browser")
}

fn bind_is_omarchy_launch_browser(bind: &HyprBind) -> bool {
    bind.arg.contains("omarchy-launch-browser")
}

fn is_stock_browser_description(bind: &HyprBind) -> bool {
    bind.description == "Browser" || bind.description == "Browser (private)"
}

fn count_browser_binds(binds: &[HyprBind]) -> (usize, usize) {
    let tsk_ws = binds.iter().any(bind_runs_tsk_workspace_switch);
    let omarchy_ws = binds.iter().any(bind_is_omarchy_workspace_digit);
    let tsk = binds
        .iter()
        .filter(|b| {
            bind_runs_tsk_launch_chromium(b)
                || (tsk_ws && !omarchy_ws && is_stock_browser_description(b))
        })
        .count();
    let omarchy = binds
        .iter()
        .filter(|b| {
            bind_is_omarchy_launch_browser(b)
                || ((!tsk_ws || omarchy_ws) && is_stock_browser_description(b))
        })
        .count();
    (tsk, omarchy)
}

fn browser_launch_is_tsk() -> bool {
    browser_launch_counts().is_some_and(|(tsk, omarchy)| tsk > 0 && omarchy == 0)
}

fn browser_launch_detail() -> String {
    match browser_launch_counts() {
        None if !hyprland::available() => "hyprctl unavailable".into(),
        None => "hyprctl binds failed".into(),
        Some((tsk, omarchy)) if tsk > 0 && omarchy == 0 => "hyprctl binds".into(),
        Some((tsk, omarchy)) => format!(
            "tsk launch browser binds: {tsk}, Omarchy browser binds still active: {omarchy}"
        ),
    }
}

fn browser_launch_counts() -> Option<(usize, usize)> {
    if !hyprland::available() {
        return None;
    }
    let text = hyprland::hyprctl_output(&["binds"]).ok()?;
    let binds = parse_hyprctl_binds(&text);
    if binds.is_empty() {
        return None;
    }
    Some(count_browser_binds(&binds))
}

fn bind_is_omarchy_workspace_digit(bind: &HyprBind) -> bool {
    bind.modmask == 64
        && !bind.description.contains("Taskspace workspace")
        && (((10..=19).contains(&bind.keycode) && bind.dispatcher == "workspace")
            || bind.description.starts_with("Switch to workspace "))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_BINDS: &str = r#"
bind
	modmask: 64
	keycode: 10
	dispatcher: exec
	arg: /usr/bin/tsk workspace switch 1

bindd
	modmask: 64
	keycode: 10
	dispatcher: workspace
	arg: 1
"#;

    #[test]
    fn parse_hyprctl_binds_reads_tsk_and_omarchy_super_one() {
        let binds = parse_hyprctl_binds(SAMPLE_BINDS);
        assert_eq!(binds.len(), 2);
        assert!(bind_runs_tsk_workspace_switch(&binds[0]));
        assert!(bind_is_omarchy_workspace_digit(&binds[1]));
    }

    #[test]
    fn parse_hyprctl_binds_ignores_shift_move_and_group() {
        let text = r#"
bind
	modmask: 65
	keycode: 10
	dispatcher: exec
	arg: /usr/bin/tsk workspace move-dispatch 1

bindd
	modmask: 64
	keycode: 10
	dispatcher: changegroupactive
	arg: 1

bind
	modmask: 64
	keycode: 10
	dispatcher: exec
	arg: /usr/bin/tsk workspace switch 1
"#;
        let binds = parse_hyprctl_binds(text);
        assert_eq!(
            binds
                .iter()
                .filter(|b| bind_runs_tsk_workspace_switch(b))
                .count(),
            1
        );
        assert_eq!(
            binds
                .iter()
                .filter(|b| bind_is_omarchy_workspace_digit(b))
                .count(),
            0
        );
    }

    #[test]
    fn parse_lua_binds_by_description_when_keycode_is_zero() {
        let text = r#"
bindd
	modmask: 64
	key: SUPER + code:10
	keycode: 0
	description: Taskspace workspace 1
	dispatcher: __lua
	arg: 33

bindd
	modmask: 64
	key: SUPER + code:10
	keycode: 0
	description: Switch to workspace 1
	dispatcher: __lua
	arg: 1

bindd
	modmask: 65
	key: B
	keycode: 0
	description: Taskspace launch browser
	dispatcher: __lua
	arg: 335

bindd
	modmask: 65
	key: B
	keycode: 0
	description: Browser
	dispatcher: __lua
	arg: 12
"#;
        let binds = parse_hyprctl_binds(text);
        assert!(bind_runs_tsk_workspace_switch(&binds[0]));
        assert!(!bind_is_omarchy_workspace_digit(&binds[0]));
        assert!(bind_is_omarchy_workspace_digit(&binds[1]));
        assert!(bind_runs_tsk_launch_chromium(&binds[2]));
        assert!(!bind_is_omarchy_launch_browser(&binds[2]));
        assert!(!bind_runs_tsk_launch_chromium(&binds[3]));
        assert_eq!(count_browser_binds(&binds), (1, 1));
    }

    #[test]
    fn stock_browser_description_counts_as_tsk_when_omarchy_workspace_unbound() {
        let text = r#"
bindd
	modmask: 64
	key: SUPER + code:10
	keycode: 0
	description: Taskspace workspace 1
	dispatcher: __lua
	arg: 33

bindd
	modmask: 65
	key: B
	keycode: 0
	description: Browser
	dispatcher: __lua
	arg: 335
"#;
        let binds = parse_hyprctl_binds(text);
        assert_eq!(count_browser_binds(&binds), (1, 0));
    }

    fn check(passed: bool, label: &str) -> DoctorCheck {
        DoctorCheck {
            label: label.into(),
            passed,
            detail: "detail".into(),
        }
    }

    #[test]
    fn format_doctor_report_default_is_failures_only() {
        let checks = [check(true, "a"), check(false, "b"), check(true, "c")];
        assert_eq!(format_doctor_report(&checks, false), "[FAIL] b: detail");
    }

    #[test]
    fn format_doctor_report_all_pass_is_ok() {
        let checks = [check(true, "a"), check(true, "b")];
        assert_eq!(format_doctor_report(&checks, false), "ok");
    }

    #[test]
    fn format_doctor_report_verbose_prints_all() {
        let checks = [check(true, "a"), check(false, "b")];
        assert_eq!(
            format_doctor_report(&checks, true),
            "[ok] a: detail\n[FAIL] b: detail"
        );
    }
}
