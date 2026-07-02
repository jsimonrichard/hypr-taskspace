//! Hyprland integration install / uninstall.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use chrono::Utc;
use serde_json::{json, Value};

use crate::config::LaeConfig;
use crate::error::{LaeError, Result};
use crate::install::backup::{self, backup_timestamp};
use crate::install::manifest::{self, Manifest};
use crate::install::path_link;
use crate::install::reload;
use crate::install::wrapper;
use crate::xdg::{config_home, ensure_parent, expand, user_bin_dir};

#[derive(Debug, Clone)]
pub struct InstallHyprOptions {
    pub dry_run: bool,
    pub workspace_root: Option<PathBuf>,
}

impl Default for InstallHyprOptions {
    fn default() -> Self {
        Self {
            dry_run: false,
            workspace_root: None,
        }
    }
}

pub fn install_hypr_status(cfg: &LaeConfig) -> Result<Value> {
    let m = manifest::load_manifest(&cfg.install_hypr_share_dir, "hypr")?;
    let bindings = cfg.install_hypr_share_dir.join("hypr/bindings.conf");
    let tui_helper = cfg.install_hypr_share_dir.join("bin/lae-task-tui");
    let has_source = cfg.install_hypr_config_path.is_file()
        && fs::read_to_string(&cfg.install_hypr_config_path)
            .map(|s| s.contains("lae-managed"))
            .unwrap_or(false);
    Ok(json!({
        "installed": m.is_some(),
        "bindings_exist": bindings.is_file(),
        "tui_helper_exist": tui_helper.is_file(),
        "source_line_present": has_source,
        "config_path": cfg.install_hypr_config_path,
        "bindings_path": bindings,
    }))
}

pub fn install_hypr(cfg: &LaeConfig, options: &InstallHyprOptions) -> Result<Vec<String>> {
    let share_src = find_share_root(options.workspace_root.as_deref())?.join("hypr");
    let share_dest = cfg.install_hypr_share_dir.join("hypr");
    let backup_dir = cfg
        .install_hypr_share_dir
        .join("install/hypr/backups")
        .join(backup_timestamp());
    let source_line = format!(
        "source = {}  # lae-managed (installed {})",
        cfg.install_hypr_source_line,
        Utc::now().date_naive()
    );

    if options.dry_run {
        return Ok(vec![
            format!("would copy {} → {}", share_src.display(), share_dest.display()),
            format!("would append to {}", cfg.install_hypr_config_path.display()),
            format!("would install {}", cfg.install_hypr_share_dir.join("bin/lae").display()),
            format!(
                "would symlink {} → {}",
                user_bin_dir().join("lae").display(),
                cfg.install_hypr_share_dir.join("bin/lae").display()
            ),
        ]);
    }

    ensure_parent(&share_dest.join("_"))?;
    if share_src.is_dir() {
        for entry in fs::read_dir(&share_src).map_err(|source| LaeError::Read {
            path: share_src.clone(),
            source,
        })? {
            let entry = entry.map_err(|source| LaeError::Read {
                path: share_src.clone(),
                source,
            })?;
            if entry.file_type().map_err(|source| LaeError::Read {
                path: entry.path(),
                source,
            })?.is_file()
            {
                let dest = share_dest.join(entry.file_name());
                ensure_parent(&dest)?;
                fs::copy(entry.path(), &dest).map_err(|source| LaeError::Write {
                    path: dest,
                    source,
                })?;
            }
        }
    }

    cleanup_legacy_menu(cfg)?;
    let rust_bin = build_and_install_cli(cfg, options.workspace_root.as_deref())?;
    path_link::install_path_symlink(cfg, &rust_bin)?;
    wrapper::write_install_helpers(cfg)?;

    let config_path = &cfg.install_hypr_config_path;
    let mut backed_up = Vec::new();
    let mut modified = false;
    if config_path.is_file() {
        backup::backup_file(config_path, &backup_dir)?;
        backed_up.push(json!({"path": config_path, "backup": config_path.file_name()}));
        let content = fs::read_to_string(config_path).map_err(|source| LaeError::Read {
            path: config_path.clone(),
            source,
        })?;
        if !content.contains("lae-managed") {
            fs::OpenOptions::new()
                .append(true)
                .open(config_path)
                .map_err(|source| LaeError::Write {
                    path: config_path.clone(),
                    source,
                })?
                .write_all(format!("\n{source_line}\n").as_bytes())
                .map_err(|source| LaeError::Write {
                    path: config_path.clone(),
                    source,
                })?;
            modified = true;
        }
    } else {
        ensure_parent(config_path)?;
        fs::write(config_path, format!("{source_line}\n")).map_err(|source| LaeError::Write {
            path: config_path.clone(),
            source,
        })?;
        modified = true;
    }

    let m = Manifest {
        version: 1,
        integration: "hypr".into(),
        installed_at: Utc::now().to_rfc3339(),
        backup_dir: backup_dir.to_string_lossy().into_owned(),
        templates_installed: vec![json!({"from": share_src, "to": share_dest})],
        user_files_backed_up: backed_up,
        user_files_modified: if modified {
            vec![json!({"path": config_path, "actions": [{"type": "append", "line": source_line}]})]
        } else {
            vec![]
        },
        module_kind: None,
    };
    manifest::save_manifest(&cfg.install_hypr_share_dir, &m)?;

    reload::apply_after_hypr()
}

pub fn uninstall_hypr(cfg: &LaeConfig, keep_files: bool) -> Result<Vec<String>> {
    let m = manifest::load_manifest(&cfg.install_hypr_share_dir, "hypr")?
        .ok_or_else(|| LaeError::Other("No lae Hyprland installation found".into()))?;

    let backup_root = PathBuf::from(&m.backup_dir);
    for entry in &m.user_files_backed_up {
        if let (Some(path), Some(backup)) = (entry.get("path"), entry.get("backup")) {
            let src = backup_root.join(backup.as_str().unwrap_or(""));
            let dest = expand(path.as_str().unwrap_or(""));
            if src.is_file() {
                backup::restore_file(&src, &dest)?;
            }
        }
    }

    cleanup_legacy_menu(cfg)?;

    let rust_bin = cfg.install_hypr_share_dir.join("bin/lae");
    let _ = path_link::remove_path_symlink(&rust_bin);

    if !keep_files {
        let hypr_dir = cfg.install_hypr_share_dir.join("hypr");
        if hypr_dir.is_dir() {
            let _ = fs::remove_dir_all(hypr_dir);
        }
    }

    manifest::remove_manifest(&cfg.install_hypr_share_dir, "hypr")?;
    reload::apply_after_hypr()
}

fn cleanup_legacy_menu(cfg: &LaeConfig) -> Result<()> {
    let elephant_link = config_home().join("elephant/menus/lae_tasks.lua");
    if elephant_link.is_symlink() {
        let _ = fs::remove_file(elephant_link);
    }
    let elephant_dir = cfg.install_hypr_share_dir.join("elephant");
    if elephant_dir.is_dir() {
        let _ = fs::remove_dir_all(elephant_dir);
    }
    let menu_json = cfg.install_hypr_share_dir.join("bin/lae-task-menu-json");
    if menu_json.is_file() {
        let _ = fs::remove_file(menu_json);
    }
    Ok(())
}

fn build_and_install_cli(cfg: &LaeConfig, workspace_root: Option<&Path>) -> Result<PathBuf> {
    let bin_dir = cfg.install_hypr_share_dir.join("bin");
    let dest = bin_dir.join("lae");
    ensure_parent(&dest)?;

    let workspace = workspace_root
        .map(Path::to_path_buf)
        .or_else(find_workspace_root)
        .ok_or_else(|| {
            LaeError::Other(
                "could not find Cargo workspace — set LAE_WORKSPACE or run from the repo".into(),
            )
        })?;

    let target_dir = workspace.join("target");
    let release_bin = target_dir.join("release/lae");
    eprintln!("building lae CLI (release)...");
    let status = Command::new("cargo")
        .args([
            "build",
            "-p",
            "lae-cli",
            "--release",
            "--target-dir",
        ])
        .arg(&target_dir)
        .current_dir(&workspace)
        .status()
        .map_err(|e| LaeError::Other(format!("failed to run cargo: {e}")))?;
    if !status.success() {
        return Err(LaeError::Other("cargo build -p lae-cli failed".into()));
    }

    let staging = bin_dir.join("lae.new");
    if staging.exists() {
        let _ = fs::remove_file(&staging);
    }
    fs::copy(&release_bin, &staging).map_err(|source| LaeError::Write {
        path: staging.clone(),
        source,
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&staging)
            .map_err(|source| LaeError::Read {
                path: staging.clone(),
                source,
            })?
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&staging, perms).map_err(|source| LaeError::Write {
            path: staging.clone(),
            source,
        })?;
    }
    fs::rename(&staging, &dest).map_err(|source| LaeError::Write {
        path: dest.clone(),
        source,
    })?;
    Ok(dest)
}

fn find_share_root(workspace_root: Option<&Path>) -> Result<PathBuf> {
    if let Some(root) = workspace_root {
        return Ok(root.join("share"));
    }
    find_workspace_root()
        .map(|w| w.join("share"))
        .ok_or_else(|| LaeError::Other("could not find share/ templates".into()))
}

fn find_workspace_root() -> Option<PathBuf> {
    if let Ok(env) = std::env::var("LAE_WORKSPACE") {
        let p = PathBuf::from(env);
        if p.join("Cargo.toml").is_file() {
            return Some(p);
        }
    }
    let mut dir = std::env::current_dir().ok()?;
    loop {
        if dir.join("Cargo.toml").is_file() && dir.join("share/hypr").is_dir() {
            return Some(dir);
        }
        if !dir.pop() {
            break;
        }
    }
    None
}

use std::io::Write;
