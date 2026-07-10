use std::fs;
use std::path::PathBuf;

use serde::Deserialize;

use crate::error::{TskError, Result};
use crate::host::{default_distrobox_image, migrate_stale_distrobox_image};
use crate::install::profile::is_dev_share_dir;
use crate::share::default_prod_share_dir;
use crate::xdg::{ensure_parent, expand, tsk_config_path, tsk_data_dir, resolve_daemon_socket_path};

pub fn default_daemon_socket_config_value() -> String {
    "~/.local/share/tsk/daemon.sock".into()
}

pub fn default_dev_config_contents() -> String {
    format!(
        r#"[default]
workspace_count = 10

[tasks]
base_dir = "~/tsk-tasks"
workspaces_per_task = 10
max_tasks = 9

[distrobox]
image = "{image}"
container_prefix = "tsk-dev"

[terminal]
command = "xdg-terminal-exec"
title_flag = "--title"

[browser]
command = "chromium"
user_data_flag = "--user-data-dir"

[hyprland]
enabled = true
auto_move_tagged_windows = true
switch_task_on_window_focus = false

[daemon]
socket = "~/.local/share/tsk-dev/daemon.sock"

[data]
dir = "~/.local/share/tsk"

[install.hypr]
config_path = "~/.config/hypr/hyprland.conf"
share_dir = "~/.local/share/tsk-dev"
source_line = "~/.local/share/tsk-dev/hypr/bindings.conf"
"#,
        image = default_distrobox_image(),
    )
}

pub fn default_config_contents() -> String {
    let share = default_prod_share_dir();
    let share_str = share.to_string_lossy();
    let bindings = share.join("hypr/bindings.conf");
    let source_line = bindings.to_string_lossy();
    format!(
        r#"[default]
workspace_count = 10
# Slots that always map to default Hyprland workspaces (e.g. 1 → workspace "1"), even in a task taskspace.
# global_workspaces = [1]

[tasks]
base_dir = "~/tsk-tasks"
workspaces_per_task = 10
max_tasks = 9

[distrobox]
image = "{image}"
container_prefix = "tsk"

[terminal]
command = "xdg-terminal-exec"
title_flag = "--title"

[browser]
command = "chromium"
user_data_flag = "--user-data-dir"

[hyprland]
enabled = true
auto_move_tagged_windows = true
switch_task_on_window_focus = false

[daemon]
socket = "{socket}"

[data]
dir = "~/.local/share/tsk"

[install.hypr]
config_path = "~/.config/hypr/hyprland.conf"
share_dir = "{share_str}"
source_line = "{source_line}"
require_sourced_last = true
allow_user_file_comments = false
"#,
        image = default_distrobox_image(),
        socket = default_daemon_socket_config_value(),
        share_str = share_str,
        source_line = source_line,
    )
}

#[derive(Debug, Clone)]
pub struct TskConfig {
    pub default_workspace_count: u32,
    /// 1-based slot indices that always use default (numeric) Hyprland workspace names.
    pub global_workspace_slots: Vec<u32>,
    pub tasks_base_dir: PathBuf,
    pub workspaces_per_task: u32,
    pub max_tasks: u32,
    pub distrobox_image: String,
    pub container_prefix: String,
    pub hyprland_enabled: bool,
    pub daemon_socket: String,
    /// User-writable runtime data (`state.db`, install manifests/backups).
    pub data_dir: PathBuf,
    /// Read-only integration templates (Hypr/Waybar helpers); `/usr/share/tsk` when packaged.
    pub install_hypr_share_dir: PathBuf,
    pub install_hypr_config_path: PathBuf,
    pub install_hypr_source_line: String,
    pub terminal_command: String,
    pub browser_command: String,
    pub browser_user_data_flag: String,
}

impl Default for TskConfig {
    fn default() -> Self {
        Self {
            default_workspace_count: 10,
            global_workspace_slots: Vec::new(),
            tasks_base_dir: expand("~/tsk-tasks"),
            workspaces_per_task: 10,
            max_tasks: 9,
            distrobox_image: default_distrobox_image(),
            container_prefix: "tsk".into(),
            hyprland_enabled: true,
            daemon_socket: default_daemon_socket_config_value(),
            data_dir: tsk_data_dir(),
            install_hypr_share_dir: default_prod_share_dir(),
            install_hypr_config_path: expand("~/.config/hypr/hyprland.conf"),
            install_hypr_source_line: default_prod_share_dir()
                .join("hypr/bindings.conf")
                .to_string_lossy()
                .into_owned(),
            terminal_command: "xdg-terminal-exec".into(),
            browser_command: "chromium".into(),
            browser_user_data_flag: "--user-data-dir".into(),
        }
    }
}

impl TskConfig {
    pub fn daemon_socket_path(&self) -> PathBuf {
        resolve_daemon_socket_path(&self.daemon_socket)
    }

    pub fn state_db_path(&self) -> PathBuf {
        self.data_dir.join("state.db")
    }

    pub fn install_meta_dir(&self) -> PathBuf {
        self.data_dir.join("install")
    }
}

pub fn ensure_dev_config() -> Result<PathBuf> {
    let path = dev_config_path();
    ensure_parent(&path)?;
    if !path.is_file() {
        let contents = seed_dev_config_contents()?;
        fs::write(&path, contents).map_err(|source| TskError::Write {
            path: path.clone(),
            source,
        })?;
    }
    Ok(path)
}

pub fn dev_config_path() -> PathBuf {
    expand("~/.config/tsk-dev/config.toml")
}

/// First-time dev config: copy prod settings when available, with dev paths overridden.
fn seed_dev_config_contents() -> Result<String> {
    let prod_path = tsk_config_path();
    if prod_path.is_file() {
        let raw = fs::read_to_string(&prod_path).map_err(|source| TskError::Read {
            path: prod_path,
            source,
        })?;
        dev_config_from_prod(&raw)
    } else {
        Ok(default_dev_config_contents())
    }
}

fn dev_config_from_prod(prod_contents: &str) -> Result<String> {
    let mut root: toml::Table = toml::from_str(prod_contents)
        .map_err(|e| TskError::Config(format!("prod config: {e}")))?;

    set_nested_str(
        &mut root,
        &["data", "dir"],
        "~/.local/share/tsk",
    );
    set_nested_str(
        &mut root,
        &["install", "hypr", "share_dir"],
        "~/.local/share/tsk-dev",
    );
    set_nested_str(
        &mut root,
        &["install", "hypr", "source_line"],
        "~/.local/share/tsk-dev/hypr/bindings.conf",
    );
    set_nested_str(
        &mut root,
        &["daemon", "socket"],
        "~/.local/share/tsk-dev/daemon.sock",
    );
    set_nested_str(&mut root, &["distrobox", "container_prefix"], "tsk-dev");

    toml::to_string_pretty(&toml::Value::Table(root))
        .map_err(|e| TskError::Config(format!("dev config: {e}")))
}

fn set_nested_str(root: &mut toml::Table, path: &[&str], value: &str) {
    if path.is_empty() {
        return;
    }
    if path.len() == 1 {
        root.insert(path[0].into(), toml::Value::String(value.into()));
        return;
    }
    let entry = root
        .entry(path[0])
        .or_insert_with(|| toml::Value::Table(toml::Table::new()));
    let Some(table) = entry.as_table_mut() else {
        *entry = toml::Value::Table(toml::Table::new());
        let table = entry.as_table_mut().expect("table");
        set_nested_str(table, &path[1..], value);
        return;
    };
    set_nested_str(table, &path[1..], value);
}

pub fn load_dev_config() -> Result<TskConfig> {
    let path = ensure_dev_config()?;
    let raw = std::fs::read_to_string(&path).map_err(|source| TskError::Read {
        path: path.clone(),
        source,
    })?;
    let parsed: RawConfig =
        toml::from_str(&raw).map_err(|e| TskError::Config(e.to_string()))?;
    let mut cfg = parse_config(parsed);
    let needs_repair = dev_config_needs_repair(&raw);
    apply_dev_config_overrides(&mut cfg);
    if needs_repair {
        let repaired = dev_config_from_prod(&raw)?;
        match std::fs::write(&path, &repaired) {
            Ok(()) => eprintln!(
                "repaired stale dev config (install.hypr paths → ~/.local/share/tsk-dev): {}",
                path.display()
            ),
            Err(source) => eprintln!(
                "note: dev config has stale install paths (using ~/.local/share/tsk-dev in-memory; could not write {}: {source})",
                path.display()
            ),
        }
    }
    Ok(cfg)
}

fn apply_dev_config_overrides(cfg: &mut TskConfig) {
    use crate::install::profile::dev_share_dir;

    let share = dev_share_dir();
    cfg.install_hypr_share_dir = share.clone();
    cfg.install_hypr_source_line = share
        .join("hypr/bindings.conf")
        .to_string_lossy()
        .into_owned();
    if !cfg.daemon_socket.contains("tsk-dev") {
        cfg.daemon_socket = "~/.local/share/tsk-dev/daemon.sock".into();
    }
    if cfg.container_prefix != "tsk-dev" {
        cfg.container_prefix = "tsk-dev".into();
    }
}

fn dev_config_needs_repair(raw: &str) -> bool {
    let Ok(parsed) = toml::from_str::<RawConfig>(raw) else {
        return false;
    };
    let cfg = parse_config(parsed);
    !is_dev_share_dir(&cfg.install_hypr_share_dir)
        || !cfg.daemon_socket.contains("tsk-dev")
        || cfg.container_prefix != "tsk-dev"
}

#[derive(Debug, Deserialize)]
struct RawConfig {
    #[serde(default)]
    default: RawDefault,
    #[serde(default)]
    tasks: RawTasks,
    #[serde(default)]
    distrobox: RawDistrobox,
    #[serde(default)]
    hyprland: RawHyprland,
    #[serde(default)]
    daemon: RawDaemon,
    #[serde(default)]
    terminal: RawTerminal,
    #[serde(default)]
    browser: RawBrowser,
    #[serde(default)]
    data: RawData,
    #[serde(default)]
    install: RawInstall,
}

#[derive(Debug, Default, Deserialize)]
struct RawData {
    dir: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct RawDefault {
    workspace_count: Option<u32>,
    desktop_count: Option<u32>,
    global_workspaces: Option<Vec<u32>>,
}

#[derive(Debug, Default, Deserialize)]
struct RawTasks {
    base_dir: Option<String>,
    workspaces_per_task: Option<u32>,
    max_tasks: Option<u32>,
}

#[derive(Debug, Default, Deserialize)]
struct RawDistrobox {
    image: Option<String>,
    container_prefix: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct RawHyprland {
    enabled: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
struct RawDaemon {
    socket: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct RawTerminal {
    command: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct RawBrowser {
    command: Option<String>,
    user_data_flag: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct RawInstall {
    hypr: RawInstallHypr,
}

#[derive(Debug, Default, Deserialize)]
struct RawInstallHypr {
    share_dir: Option<String>,
    config_path: Option<String>,
    source_line: Option<String>,
}

pub fn ensure_config() -> Result<PathBuf> {
    let path = tsk_config_path();
    ensure_parent(&path)?;
    if !path.is_file() {
        std::fs::write(&path, default_config_contents()).map_err(|source| TskError::Write {
            path: path.clone(),
            source,
        })?;
    }
    Ok(path)
}

pub fn load_config() -> Result<TskConfig> {
    if crate::dev_session::dev_session_active() {
        return load_dev_config();
    }
    load_config_at(&ensure_config()?)
}

pub fn load_config_at(path: &std::path::Path) -> Result<TskConfig> {
    let raw = std::fs::read_to_string(path).map_err(|source| TskError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let parsed: RawConfig =
        toml::from_str(&raw).map_err(|e| TskError::Config(e.to_string()))?;
    Ok(parse_config(parsed))
}

/// Prod config file — ignores `TSK_CONFIG` / dev auto-detection.
pub fn load_prod_config() -> Result<TskConfig> {
    let path = expand("~/.config/tsk/config.toml");
    if path.is_file() {
        load_config_at(&path)
    } else {
        Ok(TskConfig::default())
    }
}

fn parse_config(raw: RawConfig) -> TskConfig {
    let mut cfg = TskConfig::default();
    cfg.default_workspace_count = raw
        .default
        .workspace_count
        .or(raw.default.desktop_count)
        .unwrap_or(10);
    if let Some(slots) = raw.default.global_workspaces {
        cfg.global_workspace_slots = normalize_global_workspace_slots(slots, cfg.default_workspace_count);
    }
    if let Some(base) = raw.tasks.base_dir {
        cfg.tasks_base_dir = expand(base);
    }
    if let Some(n) = raw.tasks.workspaces_per_task {
        cfg.workspaces_per_task = n;
    }
    if let Some(n) = raw.tasks.max_tasks {
        cfg.max_tasks = n;
    }
    if let Some(image) = raw.distrobox.image {
        cfg.distrobox_image =
            migrate_stale_distrobox_image(&image).unwrap_or(image);
    }
    if let Some(prefix) = raw.distrobox.container_prefix {
        cfg.container_prefix = prefix;
    }
    if let Some(enabled) = raw.hyprland.enabled {
        cfg.hyprland_enabled = enabled;
    }
    if let Some(socket) = raw.daemon.socket {
        cfg.daemon_socket = socket;
    }
    if let Some(dir) = raw.data.dir {
        cfg.data_dir = expand(dir);
    }
    if let Some(share) = raw.install.hypr.share_dir {
        cfg.install_hypr_share_dir = expand(share);
    } else {
        cfg.install_hypr_share_dir = default_prod_share_dir();
    }
    if let Some(path) = raw.install.hypr.config_path {
        cfg.install_hypr_config_path = expand(path);
    }
    if let Some(line) = raw.install.hypr.source_line {
        cfg.install_hypr_source_line = expand(line).to_string_lossy().into_owned();
    } else {
        cfg.install_hypr_source_line = cfg
            .install_hypr_share_dir
            .join("hypr/bindings.conf")
            .to_string_lossy()
            .into_owned();
    }
    if let Some(cmd) = raw.terminal.command {
        cfg.terminal_command = cmd;
    }
    if let Some(cmd) = raw.browser.command {
        cfg.browser_command = cmd;
    }
    if let Some(flag) = raw.browser.user_data_flag {
        cfg.browser_user_data_flag = flag;
    }
    cfg
}

fn normalize_global_workspace_slots(slots: Vec<u32>, workspace_count: u32) -> Vec<u32> {
    let mut out: Vec<u32> = slots
        .into_iter()
        .filter(|slot| (1..=workspace_count).contains(slot))
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_workspace_count_is_ten() {
        let cfg = parse_config(toml::from_str("").unwrap());
        assert_eq!(cfg.default_workspace_count, 10);
    }

    #[test]
    fn default_config_uses_host_distrobox_image() {
        let contents = default_config_contents();
        assert!(contents.contains("[distrobox]"));
        assert!(contents.contains("image = "));
        assert!(!contents.contains("quay.io/toolbx-images/"));
    }

    #[test]
    fn stale_toolbx_images_are_migrated_on_load() {
        let raw: RawConfig = toml::from_str(
            r#"
[distrobox]
image = "quay.io/toolbx-images/arch-toolbox:latest"
"#,
        )
        .unwrap();
        let cfg = parse_config(raw);
        assert_eq!(cfg.distrobox_image, "quay.io/toolbx/arch-toolbox:latest");
    }

    #[test]
    fn default_config_socket_is_under_local_share() {
        let contents = default_config_contents();
        assert!(contents.contains("~/.local/share/tsk/daemon.sock"));
    }

    #[test]
    fn global_workspaces_are_normalized() {
        let raw: RawConfig = toml::from_str(
            r#"
[default]
workspace_count = 10
global_workspaces = [10, 1, 1, 99, 3]
"#,
        )
        .unwrap();
        let cfg = parse_config(raw);
        assert_eq!(cfg.global_workspace_slots, vec![1, 3, 10]);
    }

    #[test]
    fn state_db_lives_in_data_dir_not_share_dir() {
        let mut cfg = TskConfig::default();
        cfg.data_dir = expand("~/.local/share/tsk");
        cfg.install_hypr_share_dir = PathBuf::from("/usr/share/tsk");
        assert_eq!(cfg.state_db_path(), expand("~/.local/share/tsk/state.db"));
    }

    #[test]
    fn daemon_socket_path_resolves_from_config() {
        let mut cfg = TskConfig::default();
        cfg.daemon_socket = "~/.local/share/tsk/daemon.sock".into();
        assert!(cfg
            .daemon_socket_path()
            .ends_with(".local/share/tsk/daemon.sock"));
    }

    #[test]
    fn dev_config_from_prod_overrides_packaged_share_paths() {
        let prod = r#"
[install.hypr]
share_dir = "/usr/share/tsk"
source_line = "/usr/share/tsk/hypr/bindings.conf"
[daemon]
socket = "~/.local/share/tsk/daemon.sock"
[distrobox]
container_prefix = "tsk"
"#;
        let dev = dev_config_from_prod(prod).expect("dev config");
        assert!(dev.contains("share_dir = \"~/.local/share/tsk-dev\""));
        assert!(dev.contains("source_line = \"~/.local/share/tsk-dev/hypr/bindings.conf\""));
        assert!(dev.contains("socket = \"~/.local/share/tsk-dev/daemon.sock\""));
        assert!(dev.contains("container_prefix = \"tsk-dev\""));
    }

    #[test]
    fn apply_dev_config_overrides_fixes_stale_share_dir() {
        let stale = r#"
[install.hypr]
share_dir = "/usr/share/tsk"
source_line = "/usr/share/tsk/hypr/bindings.conf"
[daemon]
socket = "~/.local/share/tsk/daemon.sock"
[distrobox]
container_prefix = "tsk"
"#;
        let mut cfg = parse_config(toml::from_str(stale).unwrap());
        apply_dev_config_overrides(&mut cfg);
        assert_eq!(cfg.install_hypr_share_dir, expand("~/.local/share/tsk-dev"));
        assert!(cfg.install_hypr_source_line.contains("tsk-dev"));
        assert_eq!(cfg.daemon_socket, "~/.local/share/tsk-dev/daemon.sock");
        assert_eq!(cfg.container_prefix, "tsk-dev");
    }

    #[test]
    fn dev_config_needs_repair_when_share_dir_is_prod() {
        let stale = r#"
[install.hypr]
share_dir = "/usr/share/tsk"
[daemon]
socket = "~/.local/share/tsk/daemon.sock"
[distrobox]
container_prefix = "tsk"
"#;
        assert!(dev_config_needs_repair(stale));
    }

    #[test]
    fn dev_config_from_prod_preserves_global_workspaces_and_overrides_paths() {
        let prod = r#"
[default]
workspace_count = 10
global_workspaces = [1, 10]

[tasks]
base_dir = "~/tsk-tasks"

[distrobox]
container_prefix = "tsk"

[daemon]
socket = "~/.local/share/tsk/daemon.sock"

[data]
dir = "~/.local/share/tsk"

[install.hypr]
share_dir = "~/.local/share/tsk"
source_line = "~/.local/share/tsk/hypr/bindings.conf"
require_sourced_last = true
"#;
        let dev = dev_config_from_prod(prod).expect("dev config");
        let cfg = parse_config(toml::from_str(&dev).unwrap());
        assert_eq!(cfg.global_workspace_slots, vec![1, 10]);
        assert_eq!(cfg.tasks_base_dir, expand("~/tsk-tasks"));
        assert_eq!(cfg.container_prefix, "tsk-dev");
        assert_eq!(cfg.data_dir, expand("~/.local/share/tsk"));
        assert_eq!(cfg.daemon_socket, "~/.local/share/tsk-dev/daemon.sock");
        assert_eq!(
            cfg.install_hypr_share_dir,
            expand("~/.local/share/tsk-dev")
        );
        assert!(dev.contains("require_sourced_last = true"));
    }
}
