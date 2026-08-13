//! Installation health checks for `tsk doctor`.

use std::fs;
use std::path::Path;

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

    checks.push(DoctorCheck {
        label: "SUPER+1 runs tsk workspace switch (not Omarchy)".into(),
        passed: super_one_is_tsk(),
        detail: super_one_detail(),
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
    let ok = backup_dir.is_dir() && fs::read_dir(backup_dir).ok().is_some_and(|mut d| d.next().is_some());
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
            "tsk workspace binds: {tsk_binds}, Omarchy workspace binds still active: {omarchy_binds} — source omarchy-unbind.conf before tsk bindings"
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
    let tsk_binds = binds.iter().filter(|b| bind_runs_tsk_workspace_switch(b)).count();
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

fn bind_runs_tsk_workspace_switch(bind: &HyprBind) -> bool {
    bind.keycode == 10
        && bind.modmask == 64
        && bind.arg.contains("workspace switch")
}

fn bind_is_omarchy_workspace_digit(bind: &HyprBind) -> bool {
    (10..=19).contains(&bind.keycode)
        && bind.modmask == 64
        && bind.dispatcher == "workspace"
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
        assert_eq!(binds.iter().filter(|b| bind_runs_tsk_workspace_switch(b)).count(), 1);
        assert_eq!(binds.iter().filter(|b| bind_is_omarchy_workspace_digit(b)).count(), 0);
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
        assert_eq!(
            format_doctor_report(&checks, false),
            "[FAIL] b: detail"
        );
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
