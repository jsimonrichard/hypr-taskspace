//! Omarchy Quattro shell plugin + cloned-menu launch prefix.

use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::binary::{command_v_login, resolve_tsk_command};
use crate::config::TskConfig;
use crate::error::{Result, TskError};
use crate::install::profile::{install_metadata_dir, profile_for_config, InstallProfile};
use crate::share::effective_share_dir;
use crate::xdg::{ensure_parent, expand};

pub const PLUGIN_ID: &str = "tsk.taskspace";
pub const WORKSPACES_ID: &str = "omarchy.workspaces";
pub const TSK_MANAGED_LAUNCH: &str = "tsk-managed-launch";

const STOCK_LAUNCH: &str = "if (root.appLibrary) root.appLibrary.launch(appId, label)";
const OVERLAY_FILES: &[&str] = &["Taskspace.qml", "TaskspaceModel.js"];

/// Which Omarchy control surface SUPER+Tab and the bar task label open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ControlUi {
    /// Modal `tsk.taskspace` overlay inside omarchy-shell.
    #[default]
    Shell,
    /// Floating ratatui window (`tsk task tui-launch`).
    Tui,
}

impl ControlUi {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Shell => "shell",
            Self::Tui => "tui",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "shell" | "overlay" => Some(Self::Shell),
            "tui" | "terminal" => Some(Self::Tui),
            _ => None,
        }
    }

    pub fn includes_overlay(self) -> bool {
        matches!(self, Self::Shell)
    }
}

#[derive(Debug, Clone, Default)]
pub struct InstallPluginOptions {
    pub dry_run: bool,
    pub quiet: bool,
    pub control_ui: ControlUi,
}

pub fn plugins_dir() -> PathBuf {
    expand("~/.config/omarchy/plugins")
}

pub fn plugin_install_dir() -> PathBuf {
    plugins_dir().join(PLUGIN_ID)
}

pub fn shell_json_path() -> PathBuf {
    expand("~/.config/omarchy/shell.json")
}

pub fn cloned_menu_dir() -> PathBuf {
    plugins_dir().join(format!("{}.menu", omarchy_user_name()))
}

fn omarchy_user_name() -> String {
    env::var("USER")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| command_output(&["id", "-un"]))
        .unwrap_or_else(|| "user".into())
}

fn command_output(args: &[&str]) -> Option<String> {
    let (bin, rest) = args.split_first()?;
    let out = Command::new(bin).args(rest).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

pub fn omarchy_shell_present() -> bool {
    command_v_login("omarchy-shell").is_some() || Path::new("/usr/share/omarchy").is_dir()
}

pub fn plugin_source_dir(cfg: &TskConfig) -> PathBuf {
    effective_share_dir(cfg).join("omarchy-plugin")
}

pub fn overlay_qml_path() -> PathBuf {
    plugin_install_dir().join("Taskspace.qml")
}

pub fn overlay_installed() -> bool {
    overlay_qml_path().is_file()
}

pub fn control_ui_path(cfg: &TskConfig) -> PathBuf {
    install_metadata_dir(cfg, profile_for_config(cfg)).join("install/omarchy/control-ui")
}

pub fn load_control_ui(cfg: &TskConfig) -> Option<ControlUi> {
    fs::read_to_string(control_ui_path(cfg))
        .ok()
        .and_then(|text| ControlUi::parse(&text))
}

pub fn save_control_ui(cfg: &TskConfig, ui: ControlUi) -> Result<PathBuf> {
    let path = control_ui_path(cfg);
    ensure_parent(&path)?;
    fs::write(&path, format!("{}\n", ui.as_str())).map_err(|source| TskError::Write {
        path: path.clone(),
        source,
    })?;
    Ok(path)
}

/// Toggle the `tsk.taskspace` overlay via omarchy-shell. Returns true when the
/// IPC call was delivered (the shell still no-ops if the plugin is disabled).
pub fn toggle_omarchy_overlay() -> bool {
    if !omarchy_shell_present() || !overlay_installed() {
        return false;
    }
    run_logged(&["omarchy-shell", "shell", "toggle", PLUGIN_ID]).is_ok()
}

pub fn install_omarchy_plugin(
    cfg: &TskConfig,
    profile: InstallProfile,
    options: &InstallPluginOptions,
) -> Result<Vec<String>> {
    let mut actions = Vec::new();
    let src = plugin_source_dir(cfg);
    if !src.is_dir() {
        actions.push(format!(
            "Omarchy plugin skipped (missing {})",
            src.display()
        ));
        return Ok(actions);
    }

    let dest = plugin_install_dir();
    let tsk_cmd = resolve_tsk_command(cfg);
    let control_ui = options.control_ui;
    if options.dry_run {
        actions.push(format!("would copy {} → {}", src.display(), dest.display()));
        actions.push(format!(
            "would set Omarchy control UI to {}",
            control_ui.as_str()
        ));
        if !control_ui.includes_overlay() {
            actions.push("would install bar-widget only (no overlay)".into());
        }
        actions.push(format!(
            "would enable {PLUGIN_ID} and disable {WORKSPACES_ID}"
        ));
        actions.extend(install_menu_launch_prefix(cfg, options)?);
        return Ok(actions);
    }

    copy_plugin_tree(&src, &dest, &tsk_cmd, control_ui)?;
    let saved = save_control_ui(cfg, control_ui)?;
    actions.push(format!("installed plugin {}", dest.display()));
    actions.push(format!(
        "control UI {} ({})",
        control_ui.as_str(),
        saved.display()
    ));

    let _ = run_logged(&["omarchy", "plugin", "validate", &dest.to_string_lossy()]);
    let _ = run_logged(&["omarchy-shell", "shell", "rescanPlugins"]);
    match run_logged(&[
        "omarchy",
        "plugin",
        "enable",
        PLUGIN_ID,
        "--section",
        "left",
        "--after",
        "omarchy.menu",
    ]) {
        Ok(_) => actions.push(format!("enabled {PLUGIN_ID}")),
        Err(err) => actions.push(format!("enable {PLUGIN_ID}: {err}")),
    }
    match run_logged(&["omarchy", "plugin", "disable", WORKSPACES_ID]) {
        Ok(_) => actions.push(format!("disabled {WORKSPACES_ID}")),
        Err(err) => actions.push(format!("disable {WORKSPACES_ID}: {err}")),
    }
    let _ = run_logged(&["omarchy-shell", "shell", "rescanPlugins"]);

    let _ = profile;
    actions.extend(install_menu_launch_prefix(cfg, options)?);
    Ok(actions)
}

pub fn uninstall_omarchy_plugin() -> Result<Vec<String>> {
    let mut actions = Vec::new();
    if plugin_install_dir().is_dir() {
        let _ = run_logged(&["omarchy", "plugin", "disable", PLUGIN_ID]);
        actions.push(format!("disabled {PLUGIN_ID}"));
    }
    let _ = run_logged(&[
        "omarchy",
        "plugin",
        "enable",
        WORKSPACES_ID,
        "--section",
        "left",
    ]);
    actions.extend(restore_menu_launch_prefix()?);
    let _ = run_logged(&["omarchy-shell", "shell", "rescanPlugins"]);
    Ok(actions)
}

fn copy_plugin_tree(src: &Path, dest: &Path, tsk_cmd: &str, control_ui: ControlUi) -> Result<()> {
    ensure_parent(&dest.join("_"))?;
    fs::create_dir_all(dest).map_err(|source| TskError::Write {
        path: dest.to_path_buf(),
        source,
    })?;
    let mut wanted = HashSet::new();
    for entry in fs::read_dir(src).map_err(|source| TskError::Read {
        path: src.to_path_buf(),
        source,
    })? {
        let entry = entry.map_err(|source| TskError::Read {
            path: src.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = path.file_name().unwrap().to_owned();
        let name_str = name.to_string_lossy();
        if !control_ui.includes_overlay() && OVERLAY_FILES.contains(&name_str.as_ref()) {
            continue;
        }
        wanted.insert(name.clone());
        let raw = fs::read_to_string(&path).map_err(|source| TskError::Read {
            path: path.clone(),
            source,
        })?;
        let body = if name_str == "manifest.json" {
            manifest_for_control_ui(&raw, control_ui)
        } else {
            raw.replace("@TSK_CMD@", tsk_cmd)
        };
        let target = dest.join(&name);
        fs::write(&target, body).map_err(|source| TskError::Write {
            path: target,
            source,
        })?;
    }
    for entry in fs::read_dir(dest).map_err(|source| TskError::Read {
        path: dest.to_path_buf(),
        source,
    })? {
        let entry = entry.map_err(|source| TskError::Read {
            path: dest.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        if path.is_file() && !wanted.contains(&entry.file_name()) {
            let _ = fs::remove_file(path);
        }
    }
    Ok(())
}

pub fn install_menu_launch_prefix(
    cfg: &TskConfig,
    options: &InstallPluginOptions,
) -> Result<Vec<String>> {
    let tsk_cmd = resolve_tsk_command(cfg);
    let clone = cloned_menu_dir();
    if options.dry_run {
        if clone.is_dir() {
            return Ok(vec![format!(
                "would patch {} launch prefix",
                clone.display()
            )]);
        }
        return Ok(vec![format!(
            "would clone omarchy.menu → {} and patch launch prefix",
            clone.display()
        )]);
    }

    let mut actions = Vec::new();
    if !clone.is_dir() {
        run_logged(&["omarchy", "plugin", "clone", "omarchy.menu"])?;
        actions.push(format!("cloned omarchy.menu → {}", clone.display()));
    }
    let qml = clone.join("Menu.qml");
    if !qml.is_file() {
        actions.push(format!("menu clone missing {}", qml.display()));
        return Ok(actions);
    }
    let content = fs::read_to_string(&qml).map_err(|source| TskError::Read {
        path: qml.clone(),
        source,
    })?;
    let (patched, changed) = patch_menu_launch(&content, &tsk_cmd);
    if changed {
        fs::write(&qml, patched).map_err(|source| TskError::Write {
            path: qml.clone(),
            source,
        })?;
        actions.push(format!("patched {} ({TSK_MANAGED_LAUNCH})", qml.display()));
        let _ = run_logged(&["omarchy-shell", "shell", "rescanPlugins"]);
    } else if content.contains(TSK_MANAGED_LAUNCH) {
        actions.push(format!("{} already uses tsk launch", qml.display()));
    } else {
        actions.push(format!(
            "could not find app launch line in {}",
            qml.display()
        ));
    }
    Ok(actions)
}

pub fn restore_menu_launch_prefix() -> Result<Vec<String>> {
    let qml = cloned_menu_dir().join("Menu.qml");
    if !qml.is_file() {
        return Ok(Vec::new());
    }
    let content = fs::read_to_string(&qml).map_err(|source| TskError::Read {
        path: qml.clone(),
        source,
    })?;
    let (restored, changed) = unpatch_menu_launch(&content);
    if !changed {
        return Ok(Vec::new());
    }
    fs::write(&qml, restored).map_err(|source| TskError::Write {
        path: qml.clone(),
        source,
    })?;
    Ok(vec![format!(
        "restored appLibrary.launch in {}",
        qml.display()
    )])
}

pub fn patch_menu_launch(qml: &str, tsk_cmd: &str) -> (String, bool) {
    if qml.contains(TSK_MANAGED_LAUNCH) {
        return (qml.to_string(), false);
    }
    if !qml.contains(STOCK_LAUNCH) {
        return (qml.to_string(), false);
    }
    (
        qml.replacen(STOCK_LAUNCH, &patched_launch_block(tsk_cmd), 1),
        true,
    )
}

pub fn unpatch_menu_launch(qml: &str) -> (String, bool) {
    let Some(start) = qml.find(&format!("// {TSK_MANAGED_LAUNCH}")) else {
        return (qml.to_string(), false);
    };
    let rest = &qml[start..];
    let Some(exec_rel) = rest.find("Util.execDetached(") else {
        return (qml.to_string(), false);
    };
    let after = &rest[exec_rel..];
    let Some(close_rel) = after.find(')') else {
        return (qml.to_string(), false);
    };
    let end = start + exec_rel + close_rel + 1;
    let mut out = String::new();
    out.push_str(&qml[..start]);
    out.push_str(STOCK_LAUNCH);
    out.push_str(&qml[end..]);
    (out, true)
}

fn patched_launch_block(tsk_cmd: &str) -> String {
    format!(
        "// {TSK_MANAGED_LAUNCH}\n      if (root.appLibrary) root.appLibrary.beginLaunchFeedback(label)\n      Util.execDetached({} + \" launch \" + Util.shellQuote(appId + \".desktop\"))",
        js_string(tsk_cmd)
    )
}

fn js_string(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn manifest_for_control_ui(raw: &str, control_ui: ControlUi) -> String {
    if control_ui.includes_overlay() {
        return raw.to_string();
    }
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(raw) else {
        return raw.to_string();
    };
    let Some(obj) = value.as_object_mut() else {
        return raw.to_string();
    };
    obj.insert("kinds".into(), serde_json::json!(["bar-widget"]));
    obj.remove("keepLoaded");
    if let Some(entry_points) = obj
        .get_mut("entryPoints")
        .and_then(|value| value.as_object_mut())
    {
        entry_points.remove("overlay");
    }
    serde_json::to_string_pretty(&value).unwrap_or_else(|_| raw.to_string())
}

pub fn plugin_enabled_in_shell_json() -> bool {
    layout_contains_id(&shell_json_path(), PLUGIN_ID)
}

pub fn workspaces_in_left_layout() -> bool {
    left_layout_contains_id(&shell_json_path(), WORKSPACES_ID)
}

pub fn menu_launch_patched() -> bool {
    let qml = cloned_menu_dir().join("Menu.qml");
    qml.is_file()
        && fs::read_to_string(qml)
            .ok()
            .is_some_and(|c| c.contains(TSK_MANAGED_LAUNCH))
}

fn layout_contains_id(path: &Path, id: &str) -> bool {
    let Ok(text) = fs::read_to_string(path) else {
        return false;
    };
    text.contains(&format!("\"{id}\""))
}

fn left_layout_contains_id(path: &Path, id: &str) -> bool {
    let Ok(text) = fs::read_to_string(path) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return text.contains(&format!("\"{id}\""));
    };
    value
        .pointer("/bar/layout/left")
        .and_then(|v| v.as_array())
        .is_some_and(|left| {
            left.iter().any(|item| {
                item.get("id")
                    .and_then(|v| v.as_str())
                    .is_some_and(|s| s == id)
                    || item.as_str() == Some(id)
            })
        })
}

fn run_logged(args: &[&str]) -> Result<()> {
    let Some((bin, rest)) = args.split_first() else {
        return Ok(());
    };
    let resolved = command_v_login(bin).unwrap_or_else(|| (*bin).to_string());
    let status = Command::new(&resolved)
        .args(rest)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .status()
        .map_err(|e| TskError::Other(format!("failed to run {bin}: {e}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(TskError::Other(format!("{bin} {} failed", rest.join(" "))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STOCK: &str = r#"    } else if (row.kind === "app") {
      var appId = row.appId
      var label = row.label
      applySerial = requestSerial
      opened = false
      filterText = ""
      if (root.appLibrary) root.appLibrary.launch(appId, label)
    } else {
"#;

    #[test]
    fn patch_menu_launch_inserts_tsk_prefix() {
        let (out, changed) = patch_menu_launch(STOCK, "/usr/bin/tsk");
        assert!(changed);
        assert!(out.contains(TSK_MANAGED_LAUNCH));
        assert!(out.contains("beginLaunchFeedback"));
        assert!(out.contains("/usr/bin/tsk"));
        assert!(out.contains("launch "));
        assert!(!out.contains(STOCK_LAUNCH));
    }

    #[test]
    fn patch_menu_launch_is_idempotent() {
        let (once, _) = patch_menu_launch(STOCK, "tsk");
        let (twice, changed) = patch_menu_launch(&once, "tsk");
        assert!(!changed);
        assert_eq!(once, twice);
    }

    #[test]
    fn unpatch_menu_launch_restores_stock() {
        let (patched, _) = patch_menu_launch(STOCK, "/usr/bin/tsk");
        let (restored, changed) = unpatch_menu_launch(&patched);
        assert!(changed);
        assert!(restored.contains(STOCK_LAUNCH));
        assert!(!restored.contains(TSK_MANAGED_LAUNCH));
    }

    #[test]
    fn manifest_for_tui_drops_overlay_kind() {
        let raw = r#"{
  "id": "tsk.taskspace",
  "kinds": ["bar-widget", "overlay"],
  "keepLoaded": true,
  "entryPoints": {
    "barWidget": "BarWidget.qml",
    "overlay": "Taskspace.qml"
  }
}"#;
        let out = manifest_for_control_ui(raw, ControlUi::Tui);
        let value: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(value["kinds"], serde_json::json!(["bar-widget"]));
        assert!(value.get("keepLoaded").is_none());
        assert_eq!(value["entryPoints"]["barWidget"], "BarWidget.qml");
        assert!(value["entryPoints"].get("overlay").is_none());
        assert_eq!(manifest_for_control_ui(raw, ControlUi::Shell), raw);
    }
}
