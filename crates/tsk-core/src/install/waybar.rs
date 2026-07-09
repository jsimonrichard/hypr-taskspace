use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

use crate::config::TskConfig;
use crate::error::{TskError, Result};
use crate::install::backup::{self, backup_timestamp};
use crate::install::jsonc::{dump_jsonc, parse_jsonc};
use crate::install::manifest::{self, Manifest};
use crate::install::profile::{install_metadata_dir, profile_for_config, InstallProfile};
use crate::install::reload;
use crate::share::{effective_share_dir, uses_packaged_share};
use crate::xdg::{config_home, ensure_parent, expand};

pub const CFFI_MODULE: &str = "cffi/tsk";
const HYPR_WORKSPACES_MODULE: &str = "hyprland/workspaces";
const TSK_TASK_MODULE: &str = "custom/tsk-task";
const STYLE_MARKER: &str = "/* tsk-waybar */";

use crate::binary::waybar_module_path;

#[derive(Debug, Clone)]
pub struct InstallWaybarOptions {
    pub dry_run: bool,
    pub workspace_root: Option<PathBuf>,
    /// Skip `cargo build` for the CFFI module (caller already installed it).
    pub skip_module_build: bool,
    /// Skip Hyprland/Waybar reload (caller will apply once at the end).
    pub skip_reload: bool,
    /// Suppress progress messages (scripts/dev.sh --quiet).
    pub quiet: bool,
}

impl Default for InstallWaybarOptions {
    fn default() -> Self {
        Self {
            dry_run: false,
            workspace_root: None,
            skip_module_build: false,
            skip_reload: false,
            quiet: false,
        }
    }
}

#[derive(Debug)]
pub struct InstallWaybarPlan {
    pub config_path: PathBuf,
    pub backup_dir: PathBuf,
    pub module_path: PathBuf,
    pub modules_left_before: Option<Vec<String>>,
}

pub fn default_waybar_config() -> PathBuf {
    config_home().join("waybar").join("config.jsonc")
}

pub fn install_waybar_status(cfg: &TskConfig) -> Result<Value> {
    let profile = profile_for_config(cfg);
    let metadata_dir = install_metadata_dir(cfg, profile);
    let manifest = manifest::load_manifest(&metadata_dir, "waybar")?;
    let config_path = default_waybar_config();
    let mut has_cffi = false;
    if config_path.is_file() {
        let data = parse_jsonc(&fs::read_to_string(&config_path).map_err(|source| {
            TskError::Read {
                path: config_path.clone(),
                source,
            }
        })?)?;
        has_cffi = modules_left(&data)
            .iter()
            .any(|m| m == CFFI_MODULE);
    }
    Ok(json!({
        "installed": manifest.is_some(),
        "module_kind": manifest.as_ref().and_then(|m| m.module_kind.clone()),
        "cffi_module_present": has_cffi,
        "config_path": config_path,
    }))
}

pub fn plan_install(cfg: &TskConfig) -> Result<InstallWaybarPlan> {
    let profile = profile_for_config(cfg);
    let metadata_dir = install_metadata_dir(cfg, profile);
    let config_path = default_waybar_config();
    let backup_dir = metadata_dir
        .join("install/waybar/backups")
        .join(backup_timestamp());
    let module_path = waybar_module_path(cfg);
    let modules_left_before = if config_path.is_file() {
        let data = parse_jsonc(&fs::read_to_string(&config_path).map_err(|source| {
            TskError::Read {
                path: config_path.clone(),
                source,
            }
        })?)?;
        Some(modules_left(&data))
    } else {
        None
    };
    Ok(InstallWaybarPlan {
        config_path,
        backup_dir,
        module_path,
        modules_left_before,
    })
}

pub fn install_waybar(cfg: &TskConfig, options: &InstallWaybarOptions) -> Result<Vec<String>> {
    let plan = plan_install(cfg)?;
    if options.dry_run {
        let mut lines = vec![format!("would patch {}", plan.config_path.display())];
        let style_path = plan.config_path.parent().unwrap().join("style.css");
        if style_path.is_file() {
            lines.push(format!("would patch {}", style_path.display()));
        }
        lines.push("would restart Waybar".into());
        return Ok(lines);
    }

    copy_share_templates(cfg)?;
    if !options.skip_module_build && !uses_packaged_share(cfg) {
        crate::install::bins::build_and_install_waybar_module(
            cfg,
            options.workspace_root.as_deref(),
            options.quiet,
        )?;
    } else if uses_packaged_share(cfg) {
        crate::install::bins::verify_system_share_for_waybar(cfg)?;
    }

    if !plan.config_path.is_file() {
        return Err(TskError::Other(format!(
            "Waybar config not found: {}",
            plan.config_path.display()
        )));
    }

    let style_path = plan.config_path.parent().unwrap().join("style.css");
    let raw = fs::read_to_string(&plan.config_path).map_err(|source| TskError::Read {
        path: plan.config_path.clone(),
        source,
    })?;
    let data = parse_jsonc(&raw)?;

    // Always snapshot before patching (dev may re-patch an already tsk-integrated config).
    backup::backup_file(&plan.config_path, &plan.backup_dir)?;
    if style_path.is_file() {
        backup::backup_file(&style_path, &plan.backup_dir)?;
    }
    if !config_has_tsk(&data) {
        let metadata_dir = install_metadata_dir(cfg, profile_for_config(cfg));
        ensure_pristine_backup(&metadata_dir, &plan.config_path, &style_path)?;
    }

    patch_config(&plan.config_path, &plan.module_path)?;
    patch_style(&plan.config_path.parent().unwrap(), cfg)?;

    let mut m = Manifest::new_waybar(plan.backup_dir.clone());
    m.templates_installed = vec![json!({"to": cfg.install_hypr_share_dir.join("waybar")})];
    m.user_files_backed_up = vec![
        json!({"path": plan.config_path, "backup": "config.jsonc"}),
        json!({"path": style_path, "backup": "style.css"}),
    ];
    m.user_files_modified = vec![json!({
        "path": plan.config_path,
        "actions": [
            {"type": "remove", "module": HYPR_WORKSPACES_MODULE},
            {"type": "install_cffi_module", "with": CFFI_MODULE},
        ]
    })];
    manifest::save_manifest(&install_metadata_dir(cfg, profile_for_config(cfg)), &m)?;

    if options.skip_reload {
        return Ok(Vec::new());
    }
    Ok(reload::apply_after_waybar())
}

pub fn uninstall_waybar(cfg: &TskConfig) -> Result<Vec<String>> {
    let profile = profile_for_config(cfg);
    let metadata_dir = install_metadata_dir(cfg, profile);
    let config_path = default_waybar_config();

    if profile == InstallProfile::Dev && try_restore_prod_waybar(&config_path)? {
        manifest::remove_manifest(&metadata_dir, "waybar")?;
        remove_legacy_dev_manifest(cfg, "waybar")?;
        return Ok(reload::apply_after_waybar());
    }

    let m = manifest::load_manifest(&metadata_dir, "waybar")?;

    seed_pristine_from_oldest_backup(cfg, &metadata_dir)?;

    let restored = restore_from_pristine(cfg, &config_path, &metadata_dir)?;
    if !restored {
        if let Some(ref manifest) = m {
            let backup_root = PathBuf::from(&manifest.backup_dir);
            for entry in &manifest.user_files_backed_up {
                let backup_name = entry
                    .get("backup")
                    .and_then(|v| v.as_str())
                    .unwrap_or("config.jsonc");
                let dest = entry
                    .get("path")
                    .and_then(|v| v.as_str())
                    .map(expand)
                    .unwrap_or_else(|| config_path.clone());
                let src = backup_root.join(backup_name);
                if src.is_file() {
                    backup::restore_file(&src, &dest)?;
                }
            }
        }
    }

    if config_path.is_file() {
        unpatch_config(&config_path)?;
    }
    if let Some(parent) = config_path.parent() {
        unpatch_style(parent)?;
    }

    manifest::remove_manifest(&metadata_dir, "waybar")?;
    if profile == InstallProfile::Dev {
        remove_legacy_dev_manifest(cfg, "waybar")?;
    }

    if m.is_none() && !restored {
        let has_tsk = config_path
            .is_file()
            .then(|| {
                fs::read_to_string(&config_path)
                    .ok()
                    .and_then(|raw| parse_jsonc(&raw).ok())
                    .is_some_and(|data| config_has_tsk(&data))
            })
            .unwrap_or(false);
        if !has_tsk {
            return Err(TskError::Other(
                "No tsk Waybar installation found".into(),
            ));
        }
    }

    Ok(reload::apply_after_waybar())
}

fn copy_share_templates(cfg: &TskConfig) -> Result<()> {
    if uses_packaged_share(cfg) {
        return Ok(());
    }
    let share_src = crate::install::bins::resolve_share_templates(
        None,
        profile_for_config(cfg),
    )?
    .join("waybar");
    let share_dest = cfg.install_hypr_share_dir.join("waybar");
    let share_str = cfg.install_hypr_share_dir.to_string_lossy();
    ensure_parent(&share_dest.join("_"))?;
    fs::create_dir_all(&share_dest).map_err(|source| TskError::Write {
        path: share_dest.clone(),
        source,
    })?;
    for entry in fs::read_dir(&share_src).map_err(|source| TskError::Read {
        path: share_src.clone(),
        source,
    })? {
        let entry = entry.map_err(|source| TskError::Read {
            path: share_src.clone(),
            source,
        })?;
        let path = entry.path();
        if path.is_file() {
            let dest = share_dest.join(path.file_name().unwrap());
            let raw = fs::read_to_string(&path).map_err(|source| TskError::Read {
                path: path.clone(),
                source,
            })?;
            let body = raw.replace("@TSK_SHARE@", &share_str);
            fs::write(&dest, body).map_err(|source| TskError::Write { path: dest, source })?;
        }
    }
    Ok(())
}


fn patch_config(config_path: &Path, module_path: &Path) -> Result<()> {
    let raw = fs::read_to_string(config_path).map_err(|source| TskError::Read {
        path: config_path.to_path_buf(),
        source,
    })?;
    let mut data = parse_jsonc(&raw)?.as_object().cloned().ok_or_else(|| {
        TskError::Other("waybar config root must be a JSON object".into())
    })?;

    remove_tsk_keys(&mut data);
    data.remove(HYPR_WORKSPACES_MODULE);

    let mut left = modules_left_map(&data)
        .into_iter()
        .filter(|m| !is_tsk_module(m) && !is_replaced_workspace_module(m))
        .collect::<Vec<_>>();

    let insert_at = if left.first().is_some_and(|m| m.starts_with("custom/omarchy")) {
        1
    } else {
        0
    };
    if !left.iter().any(|m| m == CFFI_MODULE) {
        left.insert(insert_at, CFFI_MODULE.into());
    }
    data.insert("modules-left".into(), Value::Array(left.into_iter().map(Value::String).collect()));

    let module_path_str = module_path.to_string_lossy().replace('\\', "/");
    data.insert(
        CFFI_MODULE.into(),
        json!({ "module_path": module_path_str }),
    );

    fs::write(config_path, dump_jsonc(&Value::Object(data))).map_err(|source| {
        TskError::Write {
            path: config_path.to_path_buf(),
            source,
        }
    })?;
    Ok(())
}

fn unpatch_config(config_path: &Path) -> Result<bool> {
    if !config_path.is_file() {
        return Ok(false);
    }
    let raw = fs::read_to_string(config_path).map_err(|source| TskError::Read {
        path: config_path.to_path_buf(),
        source,
    })?;
    let mut data = parse_jsonc(&raw)?
        .as_object()
        .cloned()
        .ok_or_else(|| TskError::Other("waybar config root must be a JSON object".into()))?;

    let mut changed = remove_tsk_keys(&mut data);

    let mut left = modules_left_map(&data)
        .into_iter()
        .filter(|m| !is_tsk_module(m))
        .collect::<Vec<_>>();

    if !left.iter().any(|m| m == HYPR_WORKSPACES_MODULE) {
        let insert_at = if left.first().is_some_and(|m| m.starts_with("custom/omarchy")) {
            1
        } else {
            0
        };
        left.insert(insert_at, HYPR_WORKSPACES_MODULE.into());
        changed = true;
    }

    if left != modules_left_map(&data) {
        data.insert(
            "modules-left".into(),
            Value::Array(left.into_iter().map(Value::String).collect()),
        );
        changed = true;
    }

    if changed {
        fs::write(config_path, dump_jsonc(&Value::Object(data))).map_err(|source| {
            TskError::Write {
                path: config_path.to_path_buf(),
                source,
            }
        })?;
    }
    Ok(changed)
}

fn patch_style(config_dir: &Path, cfg: &TskConfig) -> Result<()> {
    let style_path = config_dir.join("style.css");
    let snippet_path = effective_share_dir(cfg).join("waybar/tsk-style.css");
    if !snippet_path.is_file() || !style_path.is_file() {
        return Ok(());
    }
    let mut content = fs::read_to_string(&style_path).map_err(|source| TskError::Read {
        path: style_path.clone(),
        source,
    })?;
    let snippet = fs::read_to_string(&snippet_path).map_err(|source| TskError::Read {
        path: snippet_path,
        source,
    })?;
    if let Some(idx) = content.find(STYLE_MARKER) {
        content = content[..idx].trim_end().to_string();
    }
    content.push_str(&format!("\n\n{STYLE_MARKER}\n{}\n", snippet.trim()));
    fs::write(&style_path, content).map_err(|source| TskError::Write { path: style_path, source })
}

fn unpatch_style(config_dir: &Path) -> Result<bool> {
    let style_path = config_dir.join("style.css");
    if !style_path.is_file() {
        return Ok(false);
    }
    let content = fs::read_to_string(&style_path).map_err(|source| TskError::Read {
        path: style_path.clone(),
        source,
    })?;
    let Some(idx) = content.find(STYLE_MARKER) else {
        return Ok(false);
    };
    fs::write(&style_path, format!("{}\n", content[..idx].trim_end())).map_err(|source| {
        TskError::Write {
            path: style_path,
            source,
        }
    })?;
    Ok(true)
}

fn pristine_backup_dir(metadata_dir: &Path) -> PathBuf {
    metadata_dir.join("install/waybar/backups/pristine")
}

fn ensure_pristine_backup(
    metadata_dir: &Path,
    config_path: &Path,
    style_path: &Path,
) -> Result<()> {
    let pristine = pristine_backup_dir(metadata_dir);
    if pristine.join("config.jsonc").is_file() {
        return Ok(());
    }
    ensure_parent(&pristine.join("_"))?;
    backup::backup_file(config_path, &pristine)?;
    if style_path.is_file() {
        backup::backup_file(style_path, &pristine)?;
    }
    Ok(())
}

fn seed_pristine_from_oldest_backup(_cfg: &TskConfig, metadata_dir: &Path) -> Result<bool> {
    let pristine = pristine_backup_dir(metadata_dir);
    if pristine.join("config.jsonc").is_file() {
        return Ok(false);
    }
    let backups_root = metadata_dir.join("install/waybar/backups");
    if !backups_root.is_dir() {
        return Ok(false);
    }
    let mut candidates = fs::read_dir(&backups_root)
        .map_err(|source| TskError::Read {
            path: backups_root.clone(),
            source,
        })?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n != "pristine" && n != "original")
        })
        .collect::<Vec<_>>();
    candidates.sort();
    for backup_dir in candidates {
        let config_backup = backup_dir.join("config.jsonc");
        if !config_backup.is_file() {
            continue;
        }
        let raw = fs::read_to_string(&config_backup).map_err(|source| TskError::Read {
            path: config_backup.clone(),
            source,
        })?;
        let data = parse_jsonc(&raw)?;
        if config_has_tsk(&data) {
            continue;
        }
        ensure_parent(&pristine.join("_"))?;
        backup::backup_file(&config_backup, &pristine)?;
        let style_backup = backup_dir.join("style.css");
        if style_backup.is_file() {
            backup::backup_file(&style_backup, &pristine)?;
        }
        return Ok(true);
    }
    Ok(false)
}

fn restore_from_pristine(
    _cfg: &TskConfig,
    config_path: &Path,
    metadata_dir: &Path,
) -> Result<bool> {
    let pristine = pristine_backup_dir(metadata_dir);
    let config_backup = pristine.join("config.jsonc");
    if !config_backup.is_file() {
        return Ok(false);
    }
    backup::restore_file(&config_backup, config_path)?;
    let style_backup = pristine.join("style.css");
    let style_path = config_path.parent().unwrap().join("style.css");
    if style_backup.is_file() {
        backup::restore_file(&style_backup, &style_path)?;
    }
    Ok(true)
}

/// After dev Waybar uninstall, re-apply prod integration when prod was installed first.
fn try_restore_prod_waybar(config_path: &Path) -> Result<bool> {
    let prod_cfg = crate::config::load_prod_config()?;
    let prod_lib = if crate::share::system_share_available() {
        crate::share::system_waybar_module_path()
    } else {
        waybar_module_path(&prod_cfg)
    };
    if !crate::binary::is_usable_cdylib(&prod_lib) {
        return Ok(false);
    }

    let prod_metadata = install_metadata_dir(&prod_cfg, InstallProfile::Prod);
    let prod_manifest = manifest::load_manifest(&prod_metadata, "waybar")?;
    let prod_pristine = pristine_backup_dir(&prod_metadata).join("config.jsonc");
    if prod_manifest.is_none() && !prod_pristine.is_file() {
        return Ok(false);
    }

    if restore_from_pristine(&prod_cfg, config_path, &prod_metadata)? {
        // Restored pre-tsk baseline; patch prod module below.
    } else if let Some(ref m) = prod_manifest {
        let backup_root = PathBuf::from(&m.backup_dir);
        let src = backup_root.join("config.jsonc");
        if src.is_file() {
            backup::restore_file(&src, config_path)?;
        }
        let style_src = backup_root.join("style.css");
        let style_path = config_path.parent().unwrap().join("style.css");
        if style_src.is_file() {
            backup::restore_file(&style_src, &style_path)?;
        }
    }

    patch_config(config_path, &prod_lib)?;
    if let Some(parent) = config_path.parent() {
        let _ = patch_style(parent, &prod_cfg);
    }
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

fn modules_left(data: &Value) -> Vec<String> {
    data.as_object()
        .map(modules_left_map)
        .unwrap_or_default()
}

fn modules_left_map(data: &Map<String, Value>) -> Vec<String> {
    data.get("modules-left")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn is_tsk_module(name: &str) -> bool {
    name == CFFI_MODULE
        || name == TSK_TASK_MODULE
        || name.starts_with("custom/tsk-workspace-")
}

fn is_replaced_workspace_module(name: &str) -> bool {
    name == HYPR_WORKSPACES_MODULE
}

fn config_has_tsk(data: &Value) -> bool {
    if modules_left(data).iter().any(|m| is_tsk_module(m)) {
        return true;
    }
    data.as_object().is_some_and(|obj| {
        obj.keys().any(|key| {
            key == TSK_TASK_MODULE
                || key.starts_with("custom/tsk-workspace-")
                || key == CFFI_MODULE
        })
    })
}

fn remove_tsk_keys(data: &mut Map<String, Value>) -> bool {
    let keys = data
        .keys()
        .filter(|k| {
            *k == CFFI_MODULE
                || *k == TSK_TASK_MODULE
                || k.starts_with("custom/tsk-workspace-")
        })
        .cloned()
        .collect::<Vec<_>>();
    let changed = !keys.is_empty();
    for key in keys {
        data.remove(&key);
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_tsk_module_includes_cffi() {
        assert!(is_tsk_module(CFFI_MODULE));
        assert!(is_tsk_module("custom/tsk-workspace-1"));
        assert!(!is_tsk_module("hyprland/workspaces"));
    }

    #[test]
    fn patch_config_removes_hyprland_workspaces() {
        let mut data = Map::new();
        data.insert(
            "modules-left".into(),
            json!(["custom/omarchy", HYPR_WORKSPACES_MODULE, "clock"]),
        );
        data.insert(HYPR_WORKSPACES_MODULE.into(), json!({"format": "{name}"}));

        remove_tsk_keys(&mut data);
        data.remove(HYPR_WORKSPACES_MODULE);

        let left: Vec<String> = modules_left_map(&data)
            .into_iter()
            .filter(|m| !is_tsk_module(m) && !is_replaced_workspace_module(m))
            .collect();

        assert_eq!(left, vec!["custom/omarchy".to_string(), "clock".to_string()]);
        assert!(data.get(HYPR_WORKSPACES_MODULE).is_none());
        assert!(left.iter().all(|m| !is_replaced_workspace_module(m)));
    }
}
