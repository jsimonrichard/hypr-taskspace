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
    let has_source = cfg.install_hypr_config_path.is_file()
        && fs::read_to_string(&cfg.install_hypr_config_path)
            .map(|s| s.contains(marker))
            .unwrap_or(false);
    Ok(json!({
        "installed": m.is_some(),
        "profile": format!("{:?}", profile).to_lowercase(),
        "bindings_exist": bindings.is_file(),
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
        backed_up.push(json!({
            "path": config_path,
            "backup": backup_file_name(config_path),
        }));
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

    let mut restored_from_backup = false;
    if let Some(m) = manifest::load_manifest(&metadata_dir, "hypr")? {
        let backup_root = PathBuf::from(&m.backup_dir);
        for entry in &m.user_files_backed_up {
            if let Some(path) = entry.get("path").and_then(|v| v.as_str()) {
                let backup_name = resolve_backup_name(entry.get("backup"), path);
                let src = backup_root.join(&backup_name);
                let dest = crate::xdg::expand(path);
                if src.is_file() {
                    backup::restore_file(&src, &dest)?;
                    restored_from_backup = true;
                }
            }
        }
        manifest::remove_manifest(&metadata_dir, "hypr")?;
    }

    // Legacy: dev manifests were stored under prod data_dir before metadata split.
    if profile == InstallProfile::Dev {
        remove_legacy_dev_manifest(cfg, "hypr")?;
    }

    // Backup restore can miss the dev line (re-install, prod+dev coexistence). Always strip.
    strip_managed_source_lines(&config_path, marker)?;

    // If backup restore failed (e.g. legacy manifest backup name), re-apply prod source line.
    if profile == InstallProfile::Dev && !restored_from_backup {
        let _ = try_restore_prod_hypr(&config_path)?;
    } else if profile == InstallProfile::Dev {
        ensure_prod_source_line_if_installed(&config_path)?;
    }

    if !keep_files && !uses_packaged_share(cfg) {
        let hypr_dir = cfg.install_hypr_share_dir.join("hypr");
        if hypr_dir.is_dir() {
            let _ = fs::remove_dir_all(hypr_dir);
        }
    }

    reload::apply_after_hypr()
}

fn backup_file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "hyprland.conf".into())
}

/// Parse manifest `backup` field — older installs stored `OsStr` as `{"Unix":[...]}`.
fn resolve_backup_name(backup: Option<&Value>, config_path: &str) -> String {
    if let Some(name) = backup.and_then(|v| v.as_str()) {
        return name.to_string();
    }
    if let Some(bytes) = backup
        .and_then(|v| v.get("Unix"))
        .and_then(|v| v.as_array())
    {
        let name: String = bytes
            .iter()
            .filter_map(|b| b.as_u64().and_then(|n| u8::try_from(n).ok()))
            .map(|b| b as char)
            .collect();
        if !name.is_empty() {
            return name;
        }
    }
    Path::new(config_path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "hyprland.conf".into())
}

/// After dev Hypr uninstall, re-apply prod integration when prod was installed first.
fn try_restore_prod_hypr(config_path: &Path) -> Result<bool> {
    let prod_cfg = crate::config::load_prod_config()?;
    let prod_metadata = install_metadata_dir(&prod_cfg, InstallProfile::Prod);
    if manifest::load_manifest(&prod_metadata, "hypr")?.is_none() {
        return Ok(false);
    }

    strip_managed_source_lines(config_path, InstallProfile::Dev.manage_marker())?;
    ensure_prod_source_line(config_path, &prod_cfg)?;
    Ok(true)
}

/// Backup restore can drop the prod source line if the snapshot predates prod install.
fn ensure_prod_source_line_if_installed(config_path: &Path) -> Result<()> {
    let prod_cfg = crate::config::load_prod_config()?;
    let prod_metadata = install_metadata_dir(&prod_cfg, InstallProfile::Prod);
    if manifest::load_manifest(&prod_metadata, "hypr")?.is_some() {
        ensure_prod_source_line(config_path, &prod_cfg)?;
    }
    Ok(())
}

fn ensure_prod_source_line(config_path: &Path, prod_cfg: &TskConfig) -> Result<bool> {
    let marker = InstallProfile::Prod.manage_marker();
    if !config_path.is_file() {
        ensure_parent(config_path)?;
        fs::write(config_path, "").map_err(|source| TskError::Write {
            path: config_path.to_path_buf(),
            source,
        })?;
    }
    let content = fs::read_to_string(config_path).map_err(|source| TskError::Read {
        path: config_path.to_path_buf(),
        source,
    })?;
    if content.contains(marker) {
        return Ok(false);
    }
    let source_line = format!(
        "source = {}  # {marker} (installed {})",
        prod_cfg.install_hypr_source_line,
        Utc::now().date_naive()
    );
    fs::OpenOptions::new()
        .append(true)
        .open(config_path)
        .map_err(|source| TskError::Write {
            path: config_path.to_path_buf(),
            source,
        })?
        .write_all(format!("\n{source_line}\n").as_bytes())
        .map_err(|source| TskError::Write {
            path: config_path.to_path_buf(),
            source,
        })?;
    Ok(true)
}

fn remove_legacy_dev_manifest(cfg: &TskConfig, integration: &str) -> Result<()> {
    if let Some(m) = manifest::load_manifest(&cfg.data_dir, integration)? {
        if m.module_kind.as_deref() == Some("dev") {
            manifest::remove_manifest(&cfg.data_dir, integration)?;
        }
    }
    Ok(())
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
    use crate::config::TskConfig;
    use crate::xdg::ensure_parent;

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

    #[test]
    fn ensure_prod_source_line_appends_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hyprland.conf");
        fs::write(&path, "source = ~/.config/hypr/base.conf\n").unwrap();
        let mut cfg = TskConfig::default();
        cfg.install_hypr_source_line = "~/.local/share/tsk/hypr/bindings.conf".into();
        assert!(ensure_prod_source_line(&path, &cfg).unwrap());
        let body = fs::read_to_string(&path).unwrap();
        assert!(body.contains("tsk-managed"));
        assert!(body.contains("~/.local/share/tsk/hypr/bindings.conf"));
        assert!(!ensure_prod_source_line(&path, &cfg).unwrap());
    }

    #[test]
    fn resolve_backup_name_parses_legacy_unix_osstr() {
        let legacy = json!({"Unix": [104, 121, 112, 114, 108, 97, 110, 100, 46, 99, 111, 110, 102]});
        assert_eq!(
            resolve_backup_name(Some(&legacy), "/home/u/.config/hypr/hyprland.conf"),
            "hyprland.conf"
        );
        assert_eq!(
            resolve_backup_name(Some(&json!("hyprland.conf")), "/ignored"),
            "hyprland.conf"
        );
    }

    #[test]
    fn dev_uninstall_restores_pre_dev_hyprland_conf_from_backup() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let prod_share = home.join(".local/share/tsk");
        let dev_share = home.join(".local/share/tsk-dev");
        fs::create_dir_all(prod_share.join("install/hypr")).unwrap();
        fs::create_dir_all(dev_share.join("install/hypr")).unwrap();

        let config_path = home.join(".config/hypr/hyprland.conf");
        ensure_parent(&config_path).unwrap();

        let pre_dev = "source = ~/.config/hypr/base.conf\nsource = ~/.local/share/tsk/hypr/bindings.conf  # tsk-managed (installed 2026-07-04)\n";
        let during_dev = "source = ~/.config/hypr/base.conf\nsource = ~/.local/share/tsk-dev/hypr/bindings.conf  # tsk-dev-managed (installed 2026-07-04)\n";
        fs::write(&config_path, during_dev).unwrap();

        let prod_manifest = Manifest {
            version: 1,
            integration: "hypr".into(),
            installed_at: Utc::now().to_rfc3339(),
            backup_dir: prod_share
                .join("install/hypr/backups/old")
                .to_string_lossy()
                .into_owned(),
            templates_installed: vec![],
            user_files_backed_up: vec![],
            user_files_modified: vec![],
            module_kind: Some("prod".into()),
        };
        manifest::save_manifest(&prod_share, &prod_manifest).unwrap();

        let dev_backup_dir = dev_share.join("install/hypr/backups/dev-session");
        ensure_parent(&dev_backup_dir.join("hyprland.conf")).unwrap();
        fs::write(dev_backup_dir.join("hyprland.conf"), pre_dev).unwrap();
        let dev_manifest = Manifest {
            version: 1,
            integration: "hypr".into(),
            installed_at: Utc::now().to_rfc3339(),
            backup_dir: dev_backup_dir.to_string_lossy().into_owned(),
            templates_installed: vec![],
            user_files_backed_up: vec![json!({
                "path": config_path,
                "backup": {"Unix": [104, 121, 112, 114, 108, 97, 110, 100, 46, 99, 111, 110, 102]}
            })],
            user_files_modified: vec![],
            module_kind: Some("dev".into()),
        };
        manifest::save_manifest(&dev_share, &dev_manifest).unwrap();

        std::env::set_var("HOME", home);
        let mut cfg = TskConfig::default();
        cfg.install_hypr_share_dir = dev_share;
        cfg.install_hypr_config_path = config_path.clone();
        cfg.data_dir = prod_share.clone();

        uninstall_hypr(&cfg, true).unwrap();
        let body = fs::read_to_string(&config_path).unwrap();
        assert_eq!(body, pre_dev);
        std::env::remove_var("HOME");
    }
}
