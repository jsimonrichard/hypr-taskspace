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
pub const TSK_MANAGED_APPS: &str = "tsk-managed-apps";

const STOCK_LAUNCH: &str = "if (root.appLibrary) root.appLibrary.launch(appId, label)";
const STOCK_ICON_SOURCE: &str =
    "source: row.isApp && root.appLibrary ? root.appLibrary.iconSource(row.appIcon) : \"\"";
const PATCHED_ICON_SOURCE: &str = "source: row.isApp\n                  ? (root.appLibrary && typeof root.appLibrary.iconSource === \"function\"\n                    ? root.appLibrary.iconSource(row.appIcon)\n                    : Quickshell.iconPath(String(row.appIcon || \"\"), true))\n                  : \"\"";

const STOCK_MERGE_APP_ROWS: &str = r#"  function mergeAppRows() {
    if (!root.appLibrary) return

    var rows = root.appLibrary.sortedEntries("")
    var appRows = []
    for (var j = 0; j < rows.length; j++) {
      var entry = rows[j].entry
      var appId = String(entry.id || "")
      if (!appId) continue
      var subtext = root.appLibrary.entrySubtext(entry)
      var aliases = subtext ? [subtext] : []
      try {
        if (entry.keywords && typeof entry.keywords.join === "function") aliases = aliases.concat(entry.keywords)
      } catch (e) { }
      appRows.push({
        id: "apps." + appId,
        parent: "apps",
        kind: "app",
        icon: "",
        appIcon: String(entry.icon || ""),
        appId: appId,
        label: root.appLibrary.entryName(entry),
        title: "",
        target: "",
        description: subtext,
        action: "",
        provider: "",
        aliases: aliases,
        when: "",
        checked: "",
        order: 0
      })
    }

    var merged = MenuModel.mergeAppRows(root.items, root.itemOrder, appRows)
    root.items = merged.items
    root.itemOrder = merged.itemOrder
    if (root.opened) root.rebuildDisplay()
  }"#;

const PATCHED_MERGE_APP_ROWS: &str = r#"  function mergeAppRows() {
    // tsk-managed-apps
    // Cloned menus currently get a null plugin shell, so shell.appLibrary is
    // missing. DesktopEntries is the same Quickshell singleton AppLibrary uses.
    var rows = []
    if (root.appLibrary && typeof root.appLibrary.sortedEntries === "function") {
      rows = root.appLibrary.sortedEntries("")
    } else {
      var values = (DesktopEntries.applications && DesktopEntries.applications.values) || []
      for (var di = 0; di < values.length; di++) {
        var desktop = values[di]
        if (!desktop || desktop.noDisplay) continue
        rows.push({ entry: desktop })
      }
    }
    var appRows = []
    for (var j = 0; j < rows.length; j++) {
      var entry = rows[j].entry
      var appId = String(entry.id || "")
      if (!appId) continue
      var subtext = (root.appLibrary && typeof root.appLibrary.entrySubtext === "function")
        ? root.appLibrary.entrySubtext(entry)
        : String((entry && entry.genericName) || "")
      var aliases = subtext ? [subtext] : []
      try {
        if (entry.keywords && typeof entry.keywords.join === "function") aliases = aliases.concat(entry.keywords)
      } catch (e) { }
      appRows.push({
        id: "apps." + appId,
        parent: "apps",
        kind: "app",
        icon: "",
        appIcon: String(entry.icon || ""),
        appId: appId,
        label: (root.appLibrary && typeof root.appLibrary.entryName === "function")
          ? root.appLibrary.entryName(entry)
          : String((entry && entry.name) || appId),
        title: "",
        target: "",
        description: subtext,
        action: "",
        provider: "",
        aliases: aliases,
        when: "",
        checked: "",
        order: 0
      })
    }

    var merged = MenuModel.mergeAppRows(root.items, root.itemOrder, appRows)
    root.items = merged.items
    root.itemOrder = merged.itemOrder
    if (root.opened) root.rebuildDisplay()
  }"#;
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
    restart_omarchy_shell_for_menu(&mut actions);
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
            return Ok(vec![
                format!(
                    "would refresh {} from omarchy.menu and patch apps + launch",
                    clone.display()
                ),
                "would restart omarchy-shell to load the menu clone".into(),
            ]);
        }
        return Ok(vec![
            format!(
                "would clone omarchy.menu → {} and patch apps + launch",
                clone.display()
            ),
            "would restart omarchy-shell to load the menu clone".into(),
        ]);
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
    if let Some(stock) = omarchy_menu_source_qml() {
        fs::copy(&stock, &qml).map_err(|source| TskError::Write {
            path: qml.clone(),
            source,
        })?;
        actions.push(format!(
            "refreshed {} from {}",
            qml.display(),
            stock.display()
        ));
    }
    let content = fs::read_to_string(&qml).map_err(|source| TskError::Read {
        path: qml.clone(),
        source,
    })?;
    let (with_apps, apps_changed) = patch_menu_apps(&content);
    let (patched, launch_changed) = patch_menu_launch(&with_apps, &tsk_cmd);
    if apps_changed || launch_changed {
        fs::write(&qml, patched).map_err(|source| TskError::Write {
            path: qml.clone(),
            source,
        })?;
        if apps_changed {
            actions.push(format!("patched {} ({TSK_MANAGED_APPS})", qml.display()));
        }
        if launch_changed {
            actions.push(format!("patched {} ({TSK_MANAGED_LAUNCH})", qml.display()));
        }
        restart_omarchy_shell_for_menu(&mut actions);
    } else if content.contains(TSK_MANAGED_LAUNCH) && content.contains(TSK_MANAGED_APPS) {
        actions.push(format!(
            "{} already uses tsk launch and DesktopEntries fallback",
            qml.display()
        ));
        // keepLoaded omarchy.menu ignores rescanPlugins / inotify reload; a
        // second `tsk install all` used to no-op here while the running shell
        // still had the pre-patch Menu.qml.
        restart_omarchy_shell_for_menu(&mut actions);
    } else {
        if !content.contains(TSK_MANAGED_APPS) {
            actions.push(format!("could not find mergeAppRows in {}", qml.display()));
        }
        if !content.contains(TSK_MANAGED_LAUNCH) {
            actions.push(format!(
                "could not find app launch line in {}",
                qml.display()
            ));
        }
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
    let (without_launch, launch_changed) = unpatch_menu_launch(&content);
    let (restored, apps_changed) = unpatch_menu_apps(&without_launch);
    if !launch_changed && !apps_changed {
        return Ok(Vec::new());
    }
    fs::write(&qml, restored).map_err(|source| TskError::Write {
        path: qml.clone(),
        source,
    })?;
    Ok(vec![format!(
        "restored stock omarchy.menu launch and apps list in {}",
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
    let marker = format!("// {TSK_MANAGED_LAUNCH}");
    let Some(start) = qml.find(&marker) else {
        return (qml.to_string(), false);
    };
    let rest = &qml[start..];
    let Some(exec_rel) = rest.find("Util.execDetached(") else {
        return (qml.to_string(), false);
    };
    let after_name = &rest[exec_rel + "Util.execDetached".len()..];
    let Some(close_rel) = matching_paren(after_name) else {
        return (qml.to_string(), false);
    };
    let end = start + exec_rel + "Util.execDetached".len() + close_rel + 1;
    let mut out = String::new();
    out.push_str(&qml[..start]);
    out.push_str(STOCK_LAUNCH);
    out.push_str(&qml[end..]);
    (out, true)
}

fn matching_paren(s: &str) -> Option<usize> {
    let mut depth = 0;
    for (i, c) in s.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

fn patched_launch_block(tsk_cmd: &str) -> String {
    format!(
        "// {TSK_MANAGED_LAUNCH}\n      if (root.appLibrary && typeof root.appLibrary.beginLaunchFeedback === \"function\")\n        root.appLibrary.beginLaunchFeedback(label)\n      Util.execDetached({} + \" launch \" + Util.shellQuote(appId + \".desktop\"))",
        js_string(tsk_cmd)
    )
}

pub fn patch_menu_apps(qml: &str) -> (String, bool) {
    if qml.contains(TSK_MANAGED_APPS) {
        return (qml.to_string(), false);
    }
    if !qml.contains(STOCK_MERGE_APP_ROWS) {
        return (qml.to_string(), false);
    }
    let mut next = qml.replacen(STOCK_MERGE_APP_ROWS, PATCHED_MERGE_APP_ROWS, 1);
    if next.contains(STOCK_ICON_SOURCE) {
        next = next.replacen(STOCK_ICON_SOURCE, PATCHED_ICON_SOURCE, 1);
    }
    (next, true)
}

pub fn unpatch_menu_apps(qml: &str) -> (String, bool) {
    if !qml.contains(TSK_MANAGED_APPS) {
        return (qml.to_string(), false);
    }
    if !qml.contains(PATCHED_MERGE_APP_ROWS) {
        return (qml.to_string(), false);
    }
    let mut next = qml.replacen(PATCHED_MERGE_APP_ROWS, STOCK_MERGE_APP_ROWS, 1);
    if next.contains(PATCHED_ICON_SOURCE) {
        next = next.replacen(PATCHED_ICON_SOURCE, STOCK_ICON_SOURCE, 1);
    }
    (next, true)
}

fn omarchy_path() -> PathBuf {
    env::var("OMARCHY_PATH")
        .ok()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
        .unwrap_or_else(|| PathBuf::from("/usr/share/omarchy"))
}

fn omarchy_menu_source_qml() -> Option<PathBuf> {
    let path = omarchy_path().join("shell/plugins/menu/Menu.qml");
    path.is_file().then_some(path)
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
            .is_some_and(|c| c.contains(TSK_MANAGED_LAUNCH) && c.contains(TSK_MANAGED_APPS))
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

/// `omarchy-shell shell rescanPlugins` does not reload keepLoaded plugins
/// (cloned `omarchy.menu`). `omarchy restart shell` is the upstream command
/// that tears the process down and loads Menu.qml from disk.
fn restart_omarchy_shell_for_menu(actions: &mut Vec<String>) {
    match run_logged(&["omarchy", "restart", "shell"]) {
        Ok(_) => actions.push("restarted omarchy-shell to load the menu clone".into()),
        Err(err) => actions.push(format!(
            "omarchy restart shell: {err} — SUPER+Space Apps will stay empty until the shell restarts"
        )),
    }
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
        assert!(out.contains("typeof root.appLibrary.beginLaunchFeedback"));
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
        assert_eq!(restored, STOCK);
        assert!(restored.contains(STOCK_LAUNCH));
        assert!(!restored.contains(TSK_MANAGED_LAUNCH));
    }

    #[test]
    fn patch_menu_apps_uses_desktop_entries_fallback() {
        let stock = format!("{STOCK_MERGE_APP_ROWS}\n                {STOCK_ICON_SOURCE}\n");
        let (out, changed) = patch_menu_apps(&stock);
        assert!(changed);
        assert!(out.contains(TSK_MANAGED_APPS));
        assert!(out.contains("DesktopEntries.applications"));
        assert!(out.contains("Quickshell.iconPath"));
        assert!(!out.contains("if (!root.appLibrary) return"));
        let (twice, changed) = patch_menu_apps(&out);
        assert!(!changed);
        assert_eq!(out, twice);
    }

    #[test]
    fn unpatch_menu_apps_restores_stock() {
        let stock = format!("{STOCK_MERGE_APP_ROWS}\n                {STOCK_ICON_SOURCE}\n");
        let (patched, _) = patch_menu_apps(&stock);
        let (restored, changed) = unpatch_menu_apps(&patched);
        assert!(changed);
        assert_eq!(restored, stock);
        assert!(!restored.contains(TSK_MANAGED_APPS));
    }

    #[test]
    fn patch_applies_to_packaged_omarchy_menu() {
        let Some(path) = omarchy_menu_source_qml() else {
            return;
        };
        let stock = fs::read_to_string(path).expect("read packaged omarchy Menu.qml");
        let (apps, apps_changed) = patch_menu_apps(&stock);
        assert!(
            apps_changed,
            "packaged omarchy Menu.qml mergeAppRows no longer matches STOCK_MERGE_APP_ROWS"
        );
        let (launch, launch_changed) = patch_menu_launch(&apps, "/usr/bin/tsk");
        assert!(launch_changed);
        let (unlaunch, _) = unpatch_menu_launch(&launch);
        let (restored, _) = unpatch_menu_apps(&unlaunch);
        assert_eq!(restored, stock);
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

    fn overlay_qml() -> &'static str {
        include_str!("../../../../share/omarchy-plugin/Taskspace.qml")
    }

    fn overlay_model_js() -> &'static str {
        include_str!("../../../../share/omarchy-plugin/TaskspaceModel.js")
    }

    #[test]
    fn overlay_does_not_detach_user_commands() {
        assert!(
            !overlay_qml().contains("execDetached"),
            "overlay must wait on tsk/omarchy-file-select so failures reach the error dialog"
        );
    }

    #[test]
    fn overlay_action_processes_read_stderr() {
        let qml = overlay_qml();
        assert!(qml.contains("id: actionProc"));
        assert!(qml.contains("actionProc.errText"));
        assert!(qml.contains("id: folderPickProc"));
        assert!(qml.contains("folderPickProc.errText"));
        assert!(qml.contains("id: repoErr"));
        assert!(qml.contains("id: listErr"));
    }

    #[test]
    fn overlay_holds_command_errors_across_open() {
        let qml = overlay_qml();
        assert!(qml.contains("holdCommandError"));
        assert!(qml.contains("hasPendingError()"));
        assert!(qml.contains("queuePendingError"));
        assert!(qml.contains("showCommandError"));
        assert!(qml.contains("runTsk([\"task\", \"restore\""));
        assert!(qml.contains("runTsk([\"task\", \"switch\""));
        assert!(qml.contains("runTsk([\"taskspace\", \"default\"]"));
    }

    #[test]
    fn overlay_model_titles_restore_and_switch_failures() {
        let js = overlay_model_js();
        assert!(js.contains("Could not restore that task"));
        assert!(js.contains("function isSwitchAction"));
        assert!(js.contains("function commandFailureTitle"));
    }
}
