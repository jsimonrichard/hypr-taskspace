//! Taskspace-aware browser / link opening (Chromium-first).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::Duration;

use crate::binary::command_v_login;
use crate::config::{load_config, TskConfig};
use crate::error::{Result, TskError};
use crate::hyprland::{self, HyprWindow};
use crate::models::{ContextMode, SessionState, Task};
use crate::workspaces::primary_task_workspace;
use crate::window_registry::infer_task_id;

const BROWSER_CLASS_MARKERS: &[&str] = &["chromium", "chrome", "brave", "vivaldi", "opera", "edge"];

pub fn is_browser_class(class: &str) -> bool {
    let lower = class.to_lowercase();
    BROWSER_CLASS_MARKERS
        .iter()
        .any(|marker| lower.contains(marker))
}

const BROWSER_FALLBACKS: &[&str] = &[
    "chromium",
    "google-chrome-stable",
    "google-chrome",
    "brave-browser",
    "brave",
    "vivaldi-stable",
    "microsoft-edge-stable",
];

/// Per-task Chromium user-data directory (isolated profile).
pub fn default_browser_profile_dir(config: &TskConfig, task_id: &str) -> PathBuf {
    config.tasks_base_dir.join(task_id).join(".tsk/chromium")
}

pub fn browser_profile_path(task: &Task, config: &TskConfig) -> PathBuf {
    task.browser_profile
        .as_ref()
        .map(|p| PathBuf::from(p))
        .unwrap_or_else(|| default_browser_profile_dir(config, &task.id))
}

/// Open one or more http(s) URLs in the taskspace browser, or delegate to system xdg-open.
pub fn open_urls(urls: &[&str], task_id: Option<&str>, host: bool) -> Result<()> {
    let cfg = load_config()?;
    if host || urls.is_empty() {
        return delegate_to_system_xdg_open(urls);
    }

    let mut state = crate::registry::Registry::new(None, cfg.clone())?.load_state()?;
    crate::context_sync::sync_from_active_workspace(&mut state);

    if let Some(tid) = task_id {
        let task = state
            .tasks
            .get(tid)
            .cloned()
            .ok_or_else(|| TskError::Other(format!("Unknown task: {tid}")))?;
        return open_urls_in_task(&cfg, &state, &task, urls);
    }

    if state.context_mode == ContextMode::Task {
        if let Some(tid) = state.current_task_id.as_deref() {
            if let Some(task) = state.tasks.get(tid).cloned() {
                return open_urls_in_task(&cfg, &state, &task, urls);
            }
        }
    }

    delegate_to_system_xdg_open(urls)
}

/// Focus or launch the task browser (no URL).
pub fn launch_task_browser(task: &Task) -> Result<()> {
    let cfg = load_config()?;
    let state = crate::registry::Registry::new(None, cfg.clone())?.load_state()?;
    open_urls_in_task(&cfg, &state, task, &[])
}

fn open_urls_in_task(
    cfg: &TskConfig,
    state: &SessionState,
    task: &Task,
    urls: &[&str],
) -> Result<()> {
    let profile_dir = browser_profile_path(task, cfg);
    std::fs::create_dir_all(&profile_dir).map_err(|source| TskError::Write {
        path: profile_dir.clone(),
        source,
    })?;

    let browser = resolve_browser_command(cfg)?;
    let existing = find_task_browser_window(state, task);
    let known_browsers: HashSet<String> = hyprland::get_clients()
        .unwrap_or_default()
        .into_iter()
        .filter(|c| is_browser_class(&c.class_name))
        .map(|c| c.address.clone())
        .collect();

    if let Some(window) = &existing {
        if urls.is_empty() {
            focus_browser_window(window);
            return Ok(());
        }
        spawn_chromium(&browser, &profile_dir, urls, false, true, cfg)?;
        focus_browser_window(window);
        return Ok(());
    }

    let target_ws = target_workspace_for_browser(state, task);
    if hyprland::available() && cfg.hyprland_enabled {
        hyprland::switch_workspace_for_navigation(&target_ws);
    }

    let new_window = urls.is_empty();
    let open_urls: Vec<&str> = if urls.is_empty() {
        vec![]
    } else {
        urls.to_vec()
    };
    spawn_chromium(&browser, &profile_dir, &open_urls, new_window, false, cfg)?;

    if hyprland::available() && cfg.hyprland_enabled {
        ensure_browser_on_workspace(&target_ws, &profile_dir, &known_browsers);
    }

    Ok(())
}

fn target_workspace_for_browser(state: &SessionState, task: &Task) -> String {
    if let Ok(Some(active)) = hyprland::get_active_workspace() {
        let workspace_names: HashSet<String> = task.workspace_names().into_iter().collect();
        if workspace_names.contains(&active.name) {
            return active.name;
        }
    }
    primary_task_workspace(
        &task.id,
        state.default_workspace_count,
        &state.global_workspace_slots,
    )
}

fn find_task_browser_window(state: &SessionState, task: &Task) -> Option<HyprWindow> {
    if !hyprland::available() {
        return None;
    }
    let clients = hyprland::get_clients().ok()?;
    let workspace_names: HashSet<String> = task.workspace_names().into_iter().collect();

    clients
        .into_iter()
        .filter(|client| is_browser_class(&client.class_name))
        .filter(|client| {
            workspace_names.contains(&client.workspace_name)
                || infer_task_id(state, &client.workspace_name, &client.title)
                    .as_deref()
                    == Some(task.id.as_str())
        })
        .max_by_key(|client| client.address.clone())
}

fn focus_browser_window(window: &HyprWindow) {
    if !window.workspace_name.is_empty() {
        hyprland::switch_workspace_for_navigation(&window.workspace_name);
    }
    hyprland::focus_window(&window.address);
}

fn ensure_browser_on_workspace(
    workspace: &str,
    profile_dir: &Path,
    known_before: &HashSet<String>,
) {
    for _ in 0..20 {
        thread::sleep(Duration::from_millis(50));
        let Ok(clients) = hyprland::get_clients() else {
            continue;
        };
        for client in clients {
            if !is_browser_class(&client.class_name) {
                continue;
            }
            if known_before.contains(&client.address) {
                continue;
            }
            if !client_matches_profile(&client, profile_dir) {
                continue;
            }
            let current = if !client.workspace_name.is_empty() {
                client.workspace_name.clone()
            } else {
                client.workspace.to_string()
            };
            if current != workspace {
                hyprland::move_window_to_workspace_silent(&client.address, workspace);
            }
            hyprland::focus_window(&client.address);
            return;
        }
    }
}

fn client_matches_profile(client: &HyprWindow, profile_dir: &Path) -> bool {
    let Some(pid) = client.pid else {
        return true;
    };
    let Ok(cmdline) = std::fs::read_to_string(format!("/proc/{pid}/cmdline")) else {
        return true;
    };
    let profile = profile_dir.to_string_lossy();
    cmdline.contains(profile.as_ref())
}

fn spawn_chromium(
    browser: &str,
    profile_dir: &Path,
    urls: &[&str],
    new_window: bool,
    existing_instance: bool,
    cfg: &TskConfig,
) -> Result<()> {
    let profile_dir = profile_dir
        .canonicalize()
        .unwrap_or_else(|_| profile_dir.to_path_buf());
    let profile_flag = user_data_dir_flag(cfg, &profile_dir);

    let mut cmd = Command::new(browser);
    cmd.arg(&profile_flag);
    cmd.args(["--no-first-run", "--no-default-browser-check"]);

    if existing_instance {
        for url in urls {
            cmd.arg(format!("--new-tab={url}"));
        }
    } else {
        if new_window || urls.is_empty() {
            cmd.arg("--new-window");
        }
        if !urls.is_empty() {
            cmd.arg("--");
            cmd.args(urls);
        }
    }

    cmd.spawn().map_err(|e| {
        TskError::Other(format!(
            "failed to launch browser `{browser}`: {e}"
        ))
    })?;
    Ok(())
}

fn user_data_dir_flag(cfg: &TskConfig, profile_dir: &Path) -> String {
    let flag = cfg.browser_user_data_flag.trim();
    let path = profile_dir.display();
    if let Some((name, _)) = flag.split_once('=') {
        format!("{name}={path}")
    } else {
        format!("{flag}={path}")
    }
}

fn resolve_browser_command(cfg: &TskConfig) -> Result<String> {
    if let Some(path) = command_v_login(&cfg.browser_command) {
        return Ok(path);
    }

    if let Ok(browser) = std::env::var("BROWSER") {
        let browser = browser.trim();
        if !browser.is_empty() {
            if browser.contains('/') && Path::new(browser).is_file() {
                return Ok(browser.to_string());
            }
            if let Some(path) = command_v_login(browser) {
                return Ok(path);
            }
        }
    }

    for candidate in BROWSER_FALLBACKS {
        if let Some(path) = command_v_login(candidate) {
            return Ok(path);
        }
    }

    Err(TskError::Other(
        "no browser found — set [browser].command in ~/.config/tsk/config.toml \
         (e.g. chromium or google-chrome-stable)"
            .into(),
    ))
}

pub fn resolve_system_xdg_open() -> Result<String> {
    if let Ok(path) = std::env::var("TSK_REAL_XDG_OPEN") {
        let path = path.trim();
        if !path.is_empty() && Path::new(path).is_file() {
            return Ok(path.to_string());
        }
    }

    for candidate in ["/usr/bin/xdg-open", "/bin/xdg-open"] {
        if Path::new(candidate).is_file() {
            return Ok(candidate.to_string());
        }
    }

    if let Some(path) = command_v_login("xdg-open") {
        let wrapper = Path::new(&path);
        if wrapper
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n == "xdg-open")
        {
            let canonical = wrapper.canonicalize().ok();
            let self_path = std::env::current_exe().ok().and_then(|p| p.canonicalize().ok());
            if canonical == self_path {
                return Err(TskError::Other(
                    "xdg-open wrapper loop — set TSK_REAL_XDG_OPEN=/usr/bin/xdg-open".into(),
                ));
            }
        }
        return Ok(path);
    }

    Err(TskError::Other("xdg-open not found".into()))
}

pub fn delegate_to_system_xdg_open(urls: &[&str]) -> Result<()> {
    if urls.is_empty() {
        return Ok(());
    }
    let xdg = resolve_system_xdg_open()?;
    Command::new(&xdg)
        .args(urls)
        .spawn()
        .map_err(|e| TskError::Other(format!("failed to run `{xdg}`: {e}")))?;
    Ok(())
}

pub fn is_http_url(url: &str) -> bool {
    let url = url.trim();
    url.starts_with("http://") || url.starts_with("https://")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_profile_dir_under_task_home() {
        let cfg = TskConfig::default();
        let path = default_browser_profile_dir(&cfg, "tabc123");
        assert!(path.to_string_lossy().contains("tabc123"));
        assert!(path.to_string_lossy().ends_with(".tsk/chromium"));
    }

    #[test]
    fn user_data_dir_flag_uses_equals_form() {
        let cfg = TskConfig::default();
        let path = PathBuf::from("/tmp/chromium-profile");
        assert_eq!(
            user_data_dir_flag(&cfg, &path),
            "--user-data-dir=/tmp/chromium-profile"
        );
    }

    #[test]
    fn is_browser_class_matches_chromium_variants() {
        assert!(is_browser_class("chromium"));
        assert!(is_browser_class("google-chrome"));
        assert!(is_browser_class("Brave-browser"));
        assert!(!is_browser_class("Alacritty"));
    }
}
