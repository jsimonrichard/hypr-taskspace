//! Taskspace-aware browser / link opening (Chromium-first).
//!
//! By default Chromium uses the host profile so extensions and logins (password
//! manager, etc.) are shared across taskspaces. Set `[browser].isolate_profile`
//! for a blank per-task `--user-data-dir`. Either way, a launch still opens or
//! focuses a window on the current task workspace.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use crate::binary::{command_v_login, resolve_named_command};
use crate::config::{load_config, TskConfig};
use crate::error::{Result, TskError};
use crate::hyprland::{self, HyprWindow};
use crate::models::{ContextMode, SessionState, Task};
use crate::window_registry::infer_task_id;
use crate::workspaces::primary_task_workspace;

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

/// Per-task Chromium user-data directory (used only when profile isolation is on).
pub fn default_browser_profile_dir(config: &TskConfig, task_id: &str) -> PathBuf {
    config.tasks_base_dir.join(task_id).join(".tsk/chromium")
}

pub fn browser_profile_path(task: &Task, config: &TskConfig) -> PathBuf {
    task.browser_profile
        .as_ref()
        .map(|p| PathBuf::from(p))
        .unwrap_or_else(|| default_browser_profile_dir(config, &task.id))
}

/// Isolated profile path when `[browser].isolate_profile` is on; otherwise `None`
/// so Chromium uses the host profile (extensions + logins).
pub(crate) fn task_chromium_profile_dir(task: &Task, cfg: &TskConfig) -> Option<PathBuf> {
    cfg.browser_isolate_profile
        .then(|| browser_profile_path(task, cfg))
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
    launch_task_browser_with(task, None)
}

/// Like [`launch_task_browser`], but use `command` when the caller selected a specific browser.
pub fn launch_task_browser_with(task: &Task, command: Option<&str>) -> Result<()> {
    let cfg = load_config()?;
    let state = crate::registry::Registry::new(None, cfg.clone())?.load_state()?;
    let browser = resolve_browser_override(&cfg, command)?;
    if !is_chromium_family(&browser) {
        return spawn_plain_browser(&browser, &cfg, &state, task);
    }
    open_urls_in_task_with_browser(&cfg, &state, task, &[], &browser)
}

/// Open URLs in a new Chromium window and move it to `workspace`.
pub fn open_new_window_with_urls(
    cfg: &TskConfig,
    workspace: &str,
    urls: &[&str],
    profile_dir: Option<&Path>,
) -> Result<()> {
    let browser = resolve_browser_command(cfg)?;
    let known_browsers: HashSet<String> = hyprland::get_clients()
        .unwrap_or_default()
        .into_iter()
        .filter(|c| is_browser_class(&c.class_name))
        .map(|c| c.address.clone())
        .collect();

    if hyprland::available() && cfg.hyprland_enabled {
        hyprland::switch_workspace_for_navigation(workspace);
    }

    if is_chromium_family(&browser) {
        spawn_chromium(&browser, profile_dir, urls, true, false, cfg)?;
        if hyprland::available() && cfg.hyprland_enabled {
            ensure_browser_on_workspace(workspace, profile_dir, &known_browsers);
        }
        return Ok(());
    }

    let mut cmd = Command::new(&browser);
    cmd.args(urls);
    cmd.spawn()
        .map_err(|e| TskError::Other(format!("failed to launch browser `{browser}`: {e}")))?;
    Ok(())
}

fn open_urls_in_task(
    cfg: &TskConfig,
    state: &SessionState,
    task: &Task,
    urls: &[&str],
) -> Result<()> {
    let browser = resolve_browser_command(cfg)?;
    open_urls_in_task_with_browser(cfg, state, task, urls, &browser)
}

fn open_urls_in_task_with_browser(
    cfg: &TskConfig,
    state: &SessionState,
    task: &Task,
    urls: &[&str],
    browser: &str,
) -> Result<()> {
    let profile_dir = task_chromium_profile_dir(task, cfg);
    if let Some(ref profile_dir) = profile_dir {
        std::fs::create_dir_all(profile_dir).map_err(|source| TskError::Write {
            path: profile_dir.clone(),
            source,
        })?;
    }

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
        // Focus first so `--new-tab` lands in this window when sharing a profile.
        focus_browser_window(window);
        spawn_chromium(browser, profile_dir.as_deref(), urls, false, true, cfg)?;
        return Ok(());
    }

    if is_chromium_family(browser) {
        let restored = crate::browser_session::restore_pending(cfg, task)?;
        if restored > 0 {
            if !urls.is_empty() {
                if let Some(window) = find_task_browser_window(state, task) {
                    focus_browser_window(&window);
                    spawn_chromium(browser, profile_dir.as_deref(), urls, false, true, cfg)?;
                } else {
                    spawn_chromium(browser, profile_dir.as_deref(), urls, true, false, cfg)?;
                }
            }
            return Ok(());
        }
    }

    let target_ws = target_workspace_for_browser(state, task);
    if hyprland::available() && cfg.hyprland_enabled {
        hyprland::switch_workspace_for_navigation(&target_ws);
    }

    let open_urls: Vec<&str> = if urls.is_empty() {
        vec![]
    } else {
        urls.to_vec()
    };
    // Always a new window: with a shared host profile, omitting this would
    // open a tab in whatever Chromium window is already running.
    spawn_chromium(
        browser,
        profile_dir.as_deref(),
        &open_urls,
        true,
        false,
        cfg,
    )?;

    if hyprland::available() && cfg.hyprland_enabled {
        ensure_browser_on_workspace(&target_ws, profile_dir.as_deref(), &known_browsers);
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
                || infer_task_id(state, &client.workspace_name, &client.title).as_deref()
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
    profile_dir: Option<&Path>,
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

fn client_matches_profile(client: &HyprWindow, profile_dir: Option<&Path>) -> bool {
    let Some(profile_dir) = profile_dir else {
        return true;
    };
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
    profile_dir: Option<&Path>,
    urls: &[&str],
    new_window: bool,
    existing_instance: bool,
    cfg: &TskConfig,
) -> Result<()> {
    let profile_flag = profile_dir.map(|profile_dir| {
        let profile_dir = profile_dir
            .canonicalize()
            .unwrap_or_else(|_| profile_dir.to_path_buf());
        user_data_dir_flag(cfg, &profile_dir)
    });

    let mut cmd = Command::new(browser);
    cmd.args(chromium_argv(
        profile_flag.as_deref(),
        urls,
        new_window,
        existing_instance,
    ));

    cmd.spawn()
        .map_err(|e| TskError::Other(format!("failed to launch browser `{browser}`: {e}")))?;
    Ok(())
}

fn chromium_argv(
    profile_flag: Option<&str>,
    urls: &[&str],
    new_window: bool,
    existing_instance: bool,
) -> Vec<String> {
    let mut args = Vec::new();
    if let Some(flag) = profile_flag {
        args.push(flag.to_string());
        args.push("--no-first-run".into());
        args.push("--no-default-browser-check".into());
    }
    if existing_instance {
        for url in urls {
            args.push(format!("--new-tab={url}"));
        }
        return args;
    }
    if new_window || urls.is_empty() {
        args.push("--new-window".into());
    }
    if !urls.is_empty() {
        args.push("--".into());
        args.extend(urls.iter().map(|u| (*u).to_string()));
    }
    args
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

fn resolve_browser_override(cfg: &TskConfig, command: Option<&str>) -> Result<String> {
    if let Some(cmd) = command {
        return resolve_named_command(cmd).ok_or_else(|| {
            TskError::Other(format!(
                "browser not found: {cmd} — install it or set [browser].command"
            ))
        });
    }
    resolve_browser_command(cfg)
}

/// Chromium-family binaries accept `--user-data-dir` when profile isolation is on.
pub fn is_chromium_family(program: &str) -> bool {
    is_browser_class(program)
}

fn spawn_plain_browser(
    browser: &str,
    cfg: &TskConfig,
    state: &SessionState,
    task: &Task,
) -> Result<()> {
    let env = crate::task_env::build_task_env(state, task, &cfg.tasks_base_dir, None);
    let mut cmd = Command::new(browser);
    crate::task_env::apply_task_process_env(&mut cmd, &env, cfg);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    cmd.spawn()
        .map_err(|e| TskError::Other(format!("failed to launch browser `{browser}`: {e}")))?;
    Ok(())
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
            let self_path = std::env::current_exe()
                .ok()
                .and_then(|p| p.canonicalize().ok());
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
    fn shared_profile_omits_user_data_dir() {
        let args = chromium_argv(None, &[], true, false);
        assert_eq!(args, vec!["--new-window"]);
        assert!(args.iter().all(|a| !a.starts_with("--user-data-dir")));
    }

    #[test]
    fn isolated_profile_includes_user_data_dir() {
        let args = chromium_argv(
            Some("--user-data-dir=/tmp/chromium-profile"),
            &[],
            true,
            false,
        );
        assert_eq!(
            args,
            vec![
                "--user-data-dir=/tmp/chromium-profile",
                "--no-first-run",
                "--no-default-browser-check",
                "--new-window",
            ]
        );
    }

    #[test]
    fn shared_profile_opens_urls_in_new_window() {
        let args = chromium_argv(None, &["https://example.com"], true, false);
        assert_eq!(args, vec!["--new-window", "--", "https://example.com"]);
    }

    #[test]
    fn task_profile_dir_none_unless_isolated() {
        let cfg = TskConfig::default();
        let task = Task {
            id: "tabc123".into(),
            name: "test".into(),
            status: crate::models::TaskStatus::Active,
            repo_url: None,
            repo_path: PathBuf::from("/tmp"),
            source_repo_path: None,
            branch: None,
            container_name: "tsk-tabc123".into(),
            container_isolation: false,
            workspace_count: 3,
            browser_profile: None,
            created_at: chrono::Utc::now(),
            last_active_at: chrono::Utc::now(),
            listed_at: chrono::Utc::now(),
            agent_notes_path: None,
            ports: vec![],
        };
        assert!(task_chromium_profile_dir(&task, &cfg).is_none());

        let mut isolated = cfg.clone();
        isolated.browser_isolate_profile = true;
        let path = task_chromium_profile_dir(&task, &isolated).unwrap();
        assert!(path.to_string_lossy().ends_with(".tsk/chromium"));
    }

    #[test]
    fn is_browser_class_matches_chromium_variants() {
        assert!(is_browser_class("chromium"));
        assert!(is_browser_class("google-chrome"));
        assert!(is_browser_class("Brave-browser"));
        assert!(!is_browser_class("Alacritty"));
    }

    #[test]
    fn chromium_family_excludes_firefox() {
        assert!(is_chromium_family("/usr/bin/chromium"));
        assert!(is_chromium_family("google-chrome-stable"));
        assert!(!is_chromium_family("firefox"));
        assert!(!is_chromium_family("/usr/lib/firefox/firefox"));
    }
}
