use std::path::PathBuf;

use serde::Deserialize;

use crate::error::{LaeError, Result};
use crate::host::default_distrobox_image;
use crate::xdg::{ensure_parent, expand, lae_config_path, resolve_daemon_socket_path};

pub fn default_daemon_socket_config_value() -> String {
    "~/.local/share/lae/daemon.sock".into()
}

pub fn default_config_contents() -> String {
    format!(
        r#"[default]
workspace_count = 10

[tasks]
base_dir = "~/lae-tasks"
workspaces_per_task = 10
max_tasks = 9

[distrobox]
image = "{image}"
container_prefix = "lae"

[terminal]
command = "xdg-terminal-exec"
title_flag = "--title"

[hyprland]
enabled = true
auto_move_tagged_windows = true
switch_task_on_window_focus = false

[daemon]
socket = "{socket}"

[install.hypr]
config_path = "~/.config/hypr/hyprland.conf"
share_dir = "~/.local/share/lae"
source_line = "~/.local/share/lae/hypr/bindings.conf"
require_sourced_last = true
allow_user_file_comments = false
"#,
        image = default_distrobox_image(),
        socket = default_daemon_socket_config_value(),
    )
}

#[derive(Debug, Clone)]
pub struct LaeConfig {
    pub default_workspace_count: u32,
    pub tasks_base_dir: PathBuf,
    pub workspaces_per_task: u32,
    pub max_tasks: u32,
    pub distrobox_image: String,
    pub container_prefix: String,
    pub hyprland_enabled: bool,
    pub daemon_socket: String,
    pub install_hypr_share_dir: PathBuf,
    pub install_hypr_config_path: PathBuf,
    pub install_hypr_source_line: String,
    pub terminal_command: String,
}

impl Default for LaeConfig {
    fn default() -> Self {
        Self {
            default_workspace_count: 10,
            tasks_base_dir: expand("~/lae-tasks"),
            workspaces_per_task: 10,
            max_tasks: 9,
            distrobox_image: default_distrobox_image(),
            container_prefix: "lae".into(),
            hyprland_enabled: true,
            daemon_socket: default_daemon_socket_config_value(),
            install_hypr_share_dir: expand("~/.local/share/lae"),
            install_hypr_config_path: expand("~/.config/hypr/hyprland.conf"),
            install_hypr_source_line: expand("~/.local/share/lae/hypr/bindings.conf")
                .to_string_lossy()
                .into_owned(),
            terminal_command: "xdg-terminal-exec".into(),
        }
    }
}

impl LaeConfig {
    pub fn daemon_socket_path(&self) -> PathBuf {
        resolve_daemon_socket_path(&self.daemon_socket)
    }
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
    install: RawInstall,
}

#[derive(Debug, Default, Deserialize)]
struct RawDefault {
    workspace_count: Option<u32>,
    desktop_count: Option<u32>,
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
    let path = lae_config_path();
    ensure_parent(&path)?;
    if !path.is_file() {
        std::fs::write(&path, default_config_contents()).map_err(|source| LaeError::Write {
            path: path.clone(),
            source,
        })?;
    }
    Ok(path)
}

pub fn load_config() -> Result<LaeConfig> {
    let path = ensure_config()?;
    let raw = std::fs::read_to_string(&path).map_err(|source| LaeError::Read {
        path: path.clone(),
        source,
    })?;
    let parsed: RawConfig =
        toml::from_str(&raw).map_err(|e| LaeError::Config(e.to_string()))?;
    Ok(parse_config(parsed))
}

fn parse_config(raw: RawConfig) -> LaeConfig {
    let mut cfg = LaeConfig::default();
    cfg.default_workspace_count = raw
        .default
        .workspace_count
        .or(raw.default.desktop_count)
        .unwrap_or(10);
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
        cfg.distrobox_image = image;
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
    if let Some(share) = raw.install.hypr.share_dir {
        cfg.install_hypr_share_dir = expand(share);
    }
    if let Some(path) = raw.install.hypr.config_path {
        cfg.install_hypr_config_path = expand(path);
    }
    if let Some(line) = raw.install.hypr.source_line {
        cfg.install_hypr_source_line = expand(line).to_string_lossy().into_owned();
    }
    if let Some(cmd) = raw.terminal.command {
        cfg.terminal_command = cmd;
    }
    cfg
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
        assert!(contents.contains("quay.io/toolbx-images/"));
    }

    #[test]
    fn default_config_socket_is_under_local_share() {
        let contents = default_config_contents();
        assert!(contents.contains("~/.local/share/lae/daemon.sock"));
    }

    #[test]
    fn daemon_socket_path_resolves_from_config() {
        let mut cfg = LaeConfig::default();
        cfg.daemon_socket = "~/.local/share/lae/daemon.sock".into();
        assert!(cfg
            .daemon_socket_path()
            .ends_with(".local/share/lae/daemon.sock"));
    }
}
