//! Walker / Elephant integration install.

use std::fs;
use std::path::PathBuf;

use chrono::Utc;
use serde_json::{json, Value};

use crate::binary::resolve_tsk_command;
use crate::config::TskConfig;
use crate::error::{Result, TskError};
use crate::install::backup::{self, backup_timestamp};
use crate::install::manifest::{self, Manifest};
use crate::install::profile::{install_metadata_dir, profile_for_config};
use crate::walker::{walker_launch_prefix, walker_terminal_cmd};
use crate::xdg::{ensure_parent, expand};

const MANAGED_MARKER: &str = "tsk-managed";

#[derive(Debug, Clone)]
pub struct InstallWalkerOptions {
    pub dry_run: bool,
    pub quiet: bool,
    /// Skip quietly when Elephant config is absent (dev installs on non-Omarchy hosts).
    pub skip_if_missing: bool,
}

impl Default for InstallWalkerOptions {
    fn default() -> Self {
        Self {
            dry_run: false,
            quiet: false,
            skip_if_missing: false,
        }
    }
}

pub fn elephant_config_path() -> PathBuf {
    expand("~/.config/elephant/elephant.toml")
}

pub fn install_walker_status(cfg: &TskConfig) -> Result<Value> {
    let path = elephant_config_path();
    let metadata_dir = install_metadata_dir(cfg, profile_for_config(cfg));
    let m = manifest::load_manifest(&metadata_dir, "walker")?;
    let content = fs::read_to_string(&path).unwrap_or_default();
    let tsk_cmd = resolve_tsk_command(cfg);
    let expected_prefix = walker_launch_prefix(&tsk_cmd);
    let expected_terminal = walker_terminal_cmd(&tsk_cmd);
    Ok(json!({
        "installed": content.contains(MANAGED_MARKER),
        "manifest": m.is_some(),
        "config_path": path,
        "launch_prefix_set": content.contains(&expected_prefix),
        "terminal_cmd_set": content.contains(&expected_terminal),
    }))
}

pub fn install_walker(cfg: &TskConfig, options: &InstallWalkerOptions) -> Result<Vec<String>> {
    let path = elephant_config_path();
    let profile = profile_for_config(cfg);
    let metadata_dir = install_metadata_dir(cfg, profile);
    let backup_dir = metadata_dir
        .join("install/walker/backups")
        .join(backup_timestamp());
    let tsk_cmd = resolve_tsk_command(cfg);
    let launch_prefix = walker_launch_prefix(&tsk_cmd);
    let terminal_cmd = walker_terminal_cmd(&tsk_cmd);

    if options.dry_run {
        return Ok(vec![
            format!("would patch {} ({MANAGED_MARKER})", path.display()),
            format!("  launch_prefix = \"{launch_prefix}\""),
            format!("  terminal_cmd = \"{terminal_cmd}\""),
            "would restart elephant.service (if active)".into(),
        ]);
    }

    if !path.is_file() {
        if options.skip_if_missing {
            return Ok(vec!["Walker: skipped (Elephant config not found)".into()]);
        }
        return Err(TskError::Other(format!(
            "Elephant config not found at {} — install Walker/Omarchy first",
            path.display()
        )));
    }

    ensure_parent(&path)?;
    if path.is_file() {
        backup::backup_file(&path, &backup_dir)?;
    }

    let original = fs::read_to_string(&path).map_err(|source| TskError::Read {
        path: path.clone(),
        source,
    })?;
    let patched = patch_elephant_toml(&original, &launch_prefix, &terminal_cmd);
    fs::write(&path, &patched).map_err(|source| TskError::Write {
        path: path.clone(),
        source,
    })?;

    let mut actions = vec![
        format!("patched {}", path.display()),
        format!("  launch_prefix = \"{launch_prefix}\""),
        format!("  terminal_cmd = \"{terminal_cmd}\""),
    ];

    restart_elephant_if_active(&mut actions, options.quiet)?;

    let manifest = Manifest {
        version: 1,
        integration: "walker".into(),
        installed_at: Utc::now().to_rfc3339(),
        backup_dir: backup_dir.to_string_lossy().into_owned(),
        templates_installed: vec![json!({
            "launch_prefix": launch_prefix,
            "terminal_cmd": terminal_cmd,
        })],
        user_files_backed_up: vec![json!({"path": path, "backup": "elephant.toml"})],
        user_files_modified: vec![json!({"path": path, "actions": [{"type": "patch", "marker": MANAGED_MARKER}]})],
        module_kind: Some(format!("{profile:?}").to_lowercase()),
    };
    manifest::save_manifest(&metadata_dir, &manifest)?;
    actions.push(format!("saved install manifest ({MANAGED_MARKER})"));
    Ok(actions)
}

pub fn uninstall_walker(cfg: &TskConfig, restore_backup: bool) -> Result<Vec<String>> {
    let path = elephant_config_path();
    let metadata_dir = install_metadata_dir(cfg, profile_for_config(cfg));
    let mut actions = Vec::new();

    if restore_backup {
        if let Some(m) = manifest::load_manifest(&metadata_dir, "walker")? {
            let backup_root = PathBuf::from(&m.backup_dir);
            let backup_file = backup_root.join("elephant.toml");
            if backup_file.is_file() {
                fs::copy(&backup_file, &path).map_err(|source| TskError::Write {
                    path: path.clone(),
                    source,
                })?;
                actions.push(format!("restored {} from backup", path.display()));
            }
        }
    } else if path.is_file() {
        let content = fs::read_to_string(&path).unwrap_or_default();
        if content.contains(MANAGED_MARKER) {
            let stripped = strip_managed_lines(&content);
            fs::write(&path, stripped).map_err(|source| TskError::Write {
                path: path.clone(),
                source,
            })?;
            actions.push(format!("removed tsk walker settings from {}", path.display()));
        }
    }

    manifest::remove_manifest(&metadata_dir, "walker")?;
    restart_elephant_if_active(&mut actions, false)?;
    Ok(actions)
}

fn patch_elephant_toml(original: &str, launch_prefix: &str, terminal_cmd: &str) -> String {
    let mut lines: Vec<String> = original
        .lines()
        .filter(|line| !is_managed_key_line(line))
        .map(str::to_string)
        .collect();

    while lines.last().is_some_and(|l| l.trim().is_empty()) {
        lines.pop();
    }

    let managed = vec![
        String::new(),
        format!("# {MANAGED_MARKER} (installed {})", Utc::now().date_naive()),
        "auto_detect_launch_prefix = false".into(),
        format!("launch_prefix = \"{launch_prefix}\""),
        format!("terminal_cmd = \"{terminal_cmd}\""),
        String::new(),
    ];

    // Keys must stay at the root table — appending after `[provider_hosts]` nests them
    // under provider_hosts and Elephant ignores launch_prefix (falls back to uwsm-app).
    let insert_at = lines
        .iter()
        .position(|line| line.trim().starts_with('[') && line.trim().ends_with(']'))
        .unwrap_or(lines.len());

    for (offset, line) in managed.into_iter().enumerate() {
        lines.insert(insert_at + offset, line);
    }

    let body = lines.join("\n");
    if body.ends_with('\n') {
        body
    } else {
        format!("{body}\n")
    }
}

fn is_managed_key_line(line: &str) -> bool {
    if line.contains(MANAGED_MARKER) {
        return true;
    }
    let trimmed = line.trim();
    trimmed.starts_with("auto_detect_launch_prefix")
        || trimmed.starts_with("launch_prefix")
        || trimmed.starts_with("terminal_cmd")
}

fn strip_managed_lines(content: &str) -> String {
    let lines: Vec<String> = content
        .lines()
        .filter(|line| !is_managed_key_line(line))
        .map(str::to_string)
        .collect();
    let body = lines.join("\n");
    if body.is_empty() {
        String::new()
    } else if body.ends_with('\n') {
        body
    } else {
        format!("{body}\n")
    }
}

fn restart_elephant_if_active(actions: &mut Vec<String>, quiet: bool) -> Result<()> {
    if systemctl_user_active("elephant.service")? {
        std::process::Command::new("systemctl")
            .args(["--user", "restart", "elephant.service"])
            .status()
            .map_err(|e| TskError::Other(format!("systemctl restart elephant: {e}")))?;
        actions.push("restarted elephant.service".into());
    } else if !quiet {
        actions.push("elephant.service not active — restart Walker/Elephant manually".into());
    }
    Ok(())
}

fn systemctl_user_active(unit: &str) -> Result<bool> {
    let status = std::process::Command::new("systemctl")
        .args(["--user", "is-active", unit])
        .status()
        .map_err(|e| TskError::Other(format!("systemctl is-active {unit}: {e}")))?;
    Ok(status.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patch_inserts_managed_keys() {
        let original = "git_on_demand = true\nlaunch_prefix = ''\n";
        let out = patch_elephant_toml(original, "/usr/bin/tsk walker exec --", "/usr/bin/tsk walker terminal --");
        assert!(out.contains(MANAGED_MARKER));
        assert!(out.contains("auto_detect_launch_prefix = false"));
        assert!(out.contains("launch_prefix = \"/usr/bin/tsk walker exec --\""));
        assert!(out.contains("terminal_cmd = \"/usr/bin/tsk walker terminal --\""));
        assert!(!out.contains("launch_prefix = ''"));
    }

    #[test]
    fn patch_keeps_launch_prefix_at_root_not_under_provider_hosts() {
        let original = "git_on_demand = true\n\n[provider_hosts]\n";
        let out = patch_elephant_toml(original, "/usr/bin/tsk walker exec --", "/usr/bin/tsk walker terminal --");
        let launch = out.find("launch_prefix = ").unwrap();
        let provider_hosts = out.find("[provider_hosts]").unwrap();
        assert!(
            launch < provider_hosts,
            "launch_prefix must be root-level, before [provider_hosts]\n{out}"
        );
    }
}
