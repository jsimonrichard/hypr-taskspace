//! Prod Omarchy preset — binaries + Hyprland + Waybar integration.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::config::TskConfig;
use crate::error::{TskError, Result};
use crate::install::backup;
use crate::install::bins::{install_bins, InstallBinsOptions};
use crate::install::hypr::{install_hypr, InstallHyprOptions};
use crate::install::profile::InstallProfile;
use crate::install::waybar::{install_waybar, InstallWaybarOptions};
use crate::install::walker::{install_walker, InstallWalkerOptions};
use crate::xdg::expand;

#[derive(Debug, Clone)]
pub struct OmarchyInstallOptions {
    pub dry_run: bool,
    pub workspace_root: Option<PathBuf>,
}

pub fn install_omarchy_prod(cfg: &TskConfig, options: &OmarchyInstallOptions) -> Result<Vec<String>> {
    let profile = InstallProfile::Prod;
    let mut actions = install_bins(
        cfg,
        &InstallBinsOptions {
            dry_run: options.dry_run,
            workspace_root: options.workspace_root.clone(),
            profile: Some(profile),
            omarchy_integration: true,
            skip_waybar: false,
            skip_reload: false,
            quiet: false,
            bundled_waybar_source: None,
        },
    )?;

    let hypr = install_hypr(
        cfg,
        &InstallHyprOptions {
            dry_run: options.dry_run,
            workspace_root: options.workspace_root.clone(),
            profile: Some(profile),
            omarchy_integration: true,
            skip_bins_install: true,
            skip_reload: true,
            quiet: false,
        },
    )?;
    actions.extend(hypr);

    let waybar = install_waybar(
        cfg,
        &InstallWaybarOptions {
            dry_run: options.dry_run,
            workspace_root: options.workspace_root.clone(),
            skip_module_build: true,
            skip_reload: true,
            quiet: false,
        },
    )?;
    actions.extend(waybar);

    let walker = install_walker(
        cfg,
        &InstallWalkerOptions {
            dry_run: options.dry_run,
            quiet: false,
            skip_if_missing: false,
        },
    )?;
    actions.extend(walker);

    Ok(actions)
}

/// `~/.config/hypr/input.conf` — Omarchy enables native workspace swipes here by default.
pub fn omarchy_input_conf_path(cfg: &TskConfig) -> PathBuf {
    cfg.install_hypr_config_path
        .parent()
        .map(|dir| dir.join("input.conf"))
        .unwrap_or_else(|| expand("~/.config/hypr/input.conf"))
}

/// Comment out Hyprland's native workspace swipe gestures so tsk bindings take over.
pub fn disable_native_workspace_gestures(content: &str) -> (String, bool) {
    let mut changed = false;
    let lines: Vec<String> = content
        .lines()
        .map(|line| {
            if is_active_native_workspace_gesture(line) {
                changed = true;
                format!(
                    "# {line}  # disabled by tsk — use tsk workspace prev/next gestures in bindings.conf"
                )
            } else {
                line.to_string()
            }
        })
        .collect();
    let body = if content.ends_with('\n') || content.is_empty() {
        if lines.is_empty() {
            String::new()
        } else {
            format!("{}\n", lines.join("\n"))
        }
    } else {
        lines.join("\n")
    };
    (body, changed)
}

fn is_active_native_workspace_gesture(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.starts_with('#') {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    lower.starts_with("gesture")
        && lower.contains("workspace")
        && !lower.contains("dispatcher")
}

/// Disable native Omarchy workspace swipe gestures in `input.conf` during install.
pub fn patch_omarchy_input_gestures(
    cfg: &TskConfig,
    backup_dir: &Path,
    dry_run: bool,
) -> Result<Option<Value>> {
    let input_path = omarchy_input_conf_path(cfg);
    if !input_path.is_file() {
        return Ok(None);
    }

    let content = fs::read_to_string(&input_path).map_err(|source| TskError::Read {
        path: input_path.clone(),
        source,
    })?;
    let (patched, changed) = disable_native_workspace_gestures(&content);
    if !changed {
        return Ok(None);
    }

    if dry_run {
        return Ok(Some(json!({
            "path": input_path,
            "backup": "input.conf",
            "action": "would comment native workspace gestures",
        })));
    }

    backup::backup_file(&input_path, backup_dir)?;
    fs::write(&input_path, patched).map_err(|source| TskError::Write {
        path: input_path.clone(),
        source,
    })?;
    Ok(Some(json!({
        "path": input_path,
        "backup": "input.conf",
        "action": "commented native workspace gestures",
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disable_native_workspace_gestures_comments_active_lines() {
        let input = "\
# Enable touchpad gestures for changing workspaces
gesture = 3, horizontal, workspace
gesture = 3, left, dispatcher, exec, tsk workspace prev
";
        let (out, changed) = disable_native_workspace_gestures(input);
        assert!(changed);
        assert!(out.contains("# gesture = 3, horizontal, workspace"));
        assert!(out.contains("gesture = 3, left, dispatcher, exec, tsk workspace prev"));
    }

    #[test]
    fn disable_native_workspace_gestures_skips_already_commented() {
        let input = "# gesture = 3, horizontal, workspace\n";
        let (_, changed) = disable_native_workspace_gestures(input);
        assert!(!changed);
    }
}
