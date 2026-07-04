//! Hyprland integration install / uninstall.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde_json::{json, Value};

use crate::config::TskConfig;
use crate::error::{TskError, Result};
use crate::install::backup::{self, backup_timestamp};
use crate::install::bins::{self, InstallBinsOptions};
use crate::install::manifest::{self, Manifest};
use crate::install::profile::{InstallProfile, install_metadata_dir, profile_for_config};
use crate::share::{effective_share_dir, uses_packaged_share};
use crate::install::reload;
use crate::xdg::ensure_parent;

#[derive(Debug, Clone)]
pub struct InstallHyprOptions {
    pub dry_run: bool,
    pub workspace_root: Option<PathBuf>,
    pub profile: Option<InstallProfile>,
    pub omarchy_integration: bool,
    /// Skip binary install (caller already ran `install_bins`).
    pub skip_bins_install: bool,
}

impl Default for InstallHyprOptions {
    fn default() -> Self {
        Self {
            dry_run: false,
            workspace_root: None,
            profile: None,
            omarchy_integration: false,
            skip_bins_install: false,
        }
    }
}

pub fn install_hypr_status(cfg: &TskConfig) -> Result<Value> {
    let profile = profile_for_config(cfg);
    let marker = profile.manage_marker();
    let metadata_dir = install_metadata_dir(cfg, profile);
    let m = manifest::load_manifest(&metadata_dir, "hypr")?;
    let share = effective_share_dir(cfg);
    let bindings = share.join("hypr/bindings.conf");
    let tui_helper = share.join("bin/tsk-task-tui");
    let has_source = cfg.install_hypr_config_path.is_file()
        && fs::read_to_string(&cfg.install_hypr_config_path)
            .map(|s| s.contains(marker))
            .unwrap_or(false);
    Ok(json!({
        "installed": m.is_some(),
        "profile": format!("{:?}", profile).to_lowercase(),
        "bindings_exist": bindings.is_file(),
        "tui_helper_exist": tui_helper.is_file(),
        "source_line_present": has_source,
        "config_path": cfg.install_hypr_config_path,
        "bindings_path": bindings,
    }))
}

pub fn install_hypr(cfg: &TskConfig, options: &InstallHyprOptions) -> Result<Vec<String>> {
    let profile = options.profile.unwrap_or_else(|| profile_for_config(cfg));
    let marker = profile.manage_marker();
    let metadata_dir = install_metadata_dir(cfg, profile);
    let backup_dir = metadata_dir
        .join("install/hypr/backups")
        .join(backup_timestamp());
    let source_line = format!(
        "source = {}  # {marker} (installed {})",
        cfg.install_hypr_source_line,
        Utc::now().date_naive()
    );

    if options.dry_run {
        let mut lines = Vec::new();
        if !options.skip_bins_install {
            lines.extend(bins::install_bins(
                cfg,
                &InstallBinsOptions {
                    dry_run: true,
                    workspace_root: options.workspace_root.clone(),
                    profile: Some(profile),
                    omarchy_integration: options.omarchy_integration,
                    skip_waybar: true,
                    bundled_waybar_source: None,
                },
            )?);
        }
        lines.push(format!(
            "would append to {}",
            cfg.install_hypr_config_path.display()
        ));
        return Ok(lines);
    }

    if !options.skip_bins_install {
        bins::install_bins(
            cfg,
            &InstallBinsOptions {
                dry_run: false,
                workspace_root: options.workspace_root.clone(),
                profile: Some(profile),
                omarchy_integration: options.omarchy_integration,
                skip_waybar: true,
                bundled_waybar_source: None,
            },
        )?;
    }

    let config_path = &cfg.install_hypr_config_path;
    let mut backed_up = Vec::new();
    if config_path.is_file() {
        backup::backup_file(config_path, &backup_dir)?;
        backed_up.push(json!({"path": config_path, "backup": config_path.file_name()}));
    } else {
        ensure_parent(config_path)?;
        fs::write(config_path, "").map_err(|source| TskError::Write {
            path: config_path.clone(),
            source,
        })?;
    }

    // Replace stale managed source lines (wrong share path after config migration).
    strip_managed_source_lines(config_path, marker)?;
    // Dev and prod bindings conflict — only one profile may be sourced at a time.
    if profile == InstallProfile::Dev {
        strip_managed_source_lines(config_path, InstallProfile::Prod.manage_marker())?;
    }
    fs::OpenOptions::new()
        .append(true)
        .open(config_path)
        .map_err(|source| TskError::Write {
            path: config_path.clone(),
            source,
        })?
        .write_all(format!("\n{source_line}\n").as_bytes())
        .map_err(|source| TskError::Write {
            path: config_path.clone(),
            source,
        })?;
    let modified = true;

    let share_src = bins::find_share_root(options.workspace_root.as_deref())?;
    let m = Manifest {
        version: 1,
        integration: "hypr".into(),
        installed_at: Utc::now().to_rfc3339(),
        backup_dir: backup_dir.to_string_lossy().into_owned(),
        templates_installed: vec![json!({"from": share_src.join("hypr"), "to": cfg.install_hypr_share_dir.join("hypr")})],
        user_files_backed_up: backed_up,
        user_files_modified: if modified {
            vec![json!({"path": config_path, "actions": [{"type": "append", "line": source_line}]})]
        } else {
            vec![]
        },
        module_kind: Some(format!("{:?}", profile).to_lowercase()),
    };
    manifest::save_manifest(&metadata_dir, &m)?;

    if profile.install_systemd() && crate::install::systemd::is_systemd_unit_installed() {
        let _ = crate::install::systemd::install_systemd(
            cfg,
            &crate::install::systemd::InstallSystemdOptions {
                dry_run: false,
                enable: false,
                start: false,
            },
        );
    }

    reload::apply_after_hypr()
}

pub fn uninstall_hypr(cfg: &TskConfig, keep_files: bool) -> Result<Vec<String>> {
    let profile = profile_for_config(cfg);
    let marker = profile.manage_marker();
    let metadata_dir = install_metadata_dir(cfg, profile);
    let config_path = cfg.install_hypr_config_path.clone();

    if let Some(m) = manifest::load_manifest(&metadata_dir, "hypr")? {
        let backup_root = PathBuf::from(&m.backup_dir);
        for entry in &m.user_files_backed_up {
            if let (Some(path), Some(backup)) = (entry.get("path"), entry.get("backup")) {
                let src = backup_root.join(backup.as_str().unwrap_or(""));
                let dest = crate::xdg::expand(path.as_str().unwrap_or(""));
                if src.is_file() {
                    backup::restore_file(&src, &dest)?;
                }
            }
        }
        manifest::remove_manifest(&metadata_dir, "hypr")?;
    }

    // Legacy: dev manifests were stored under prod data_dir before metadata split.
    if profile == InstallProfile::Dev {
        if let Some(m) = manifest::load_manifest(&cfg.data_dir, "hypr")? {
            if m.module_kind.as_deref() == Some("dev") {
                manifest::remove_manifest(&cfg.data_dir, "hypr")?;
            }
        }
    }

    // Backup restore can miss the dev line (re-install, prod+dev coexistence). Always strip.
    strip_managed_source_lines(&config_path, marker)?;

    if !keep_files && !uses_packaged_share(cfg) {
        let hypr_dir = cfg.install_hypr_share_dir.join("hypr");
        if hypr_dir.is_dir() {
            let _ = fs::remove_dir_all(hypr_dir);
        }
    }

    reload::apply_after_hypr()
}

/// Remove hyprland.conf lines appended by this profile's installer.
pub fn strip_managed_source_lines(config_path: &Path, marker: &str) -> Result<bool> {
    if !config_path.is_file() {
        return Ok(false);
    }
    let content = fs::read_to_string(config_path).map_err(|source| TskError::Read {
        path: config_path.to_path_buf(),
        source,
    })?;
    if !content.contains(marker) {
        return Ok(false);
    }
    let trimmed: String = content
        .lines()
        .filter(|line| !line.contains(marker))
        .collect::<Vec<_>>()
        .join("\n");
    let body = if trimmed.is_empty() {
        String::new()
    } else if content.ends_with('\n') {
        format!("{trimmed}\n")
    } else {
        trimmed
    };
    fs::write(config_path, body).map_err(|source| TskError::Write {
        path: config_path.to_path_buf(),
        source,
    })?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dev_install_strips_prod_source_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hyprland.conf");
        fs::write(
            &path,
            "source = ~/.config/hypr/base.conf\nsource = ~/.local/share/tsk/hypr/bindings.conf  # tsk-managed (installed 2026-07-04)\n",
        )
        .unwrap();
        assert!(strip_managed_source_lines(&path, InstallProfile::Prod.manage_marker()).unwrap());
        let body = fs::read_to_string(&path).unwrap();
        assert!(body.contains("base.conf"));
        assert!(!body.contains("tsk-managed"));
    }

    #[test]
    fn strip_managed_source_lines_removes_profile_marker() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hyprland.conf");
        fs::write(
            &path,
            "source = ~/.config/hypr/base.conf\nsource = ~/.local/share/tsk-dev/hypr/bindings.conf  # tsk-dev-managed (installed 2026-07-04)\n",
        )
        .unwrap();
        assert!(strip_managed_source_lines(&path, "tsk-dev-managed").unwrap());
        let body = fs::read_to_string(&path).unwrap();
        assert!(body.contains("base.conf"));
        assert!(!body.contains("tsk-dev-managed"));
        assert!(!body.contains("tsk-dev"));
    }
}
