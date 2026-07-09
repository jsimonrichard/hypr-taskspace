//! Launch editors and browsers (optionally via Distrobox).

use std::path::Path;
use std::process::{Command, Stdio};

use crate::binary::command_v_login;
use crate::config::load_config;
use crate::distrobox;
use crate::error::{TskError, Result};
use crate::models::{SessionState, Task};
use crate::task_env;

const EDITOR_CANDIDATES: &[&str] = &["cursor", "code"];
const BROWSER_CANDIDATES: &[&str] = &[
    "chromium",
    "chromium-browser",
    "google-chrome-stable",
    "google-chrome",
    "brave",
    "firefox",
];

/// Prefer Cursor, then VS Code.
pub fn resolve_editor_command() -> Option<String> {
    for name in EDITOR_CANDIDATES {
        if let Some(path) = command_v_login(name) {
            return Some(path);
        }
    }
    None
}

pub fn resolve_browser_command() -> Option<String> {
    if let Ok(browser) = std::env::var("BROWSER") {
        let browser = browser.trim();
        if !browser.is_empty() {
            if browser.contains('/') && Path::new(browser).is_file() {
                return Some(browser.to_string());
            }
            if let Some(path) = command_v_login(browser) {
                return Some(path);
            }
        }
    }
    for name in BROWSER_CANDIDATES {
        if let Some(path) = command_v_login(name) {
            return Some(path);
        }
    }
    None
}

/// Open the task checkout in Cursor/VS Code (inside Distrobox when isolation is on).
pub fn launch_task_editor(task: &Task, state: &SessionState) -> Result<()> {
    let editor = resolve_editor_command().ok_or_else(|| {
        TskError::Other(
            "no editor found (looked for cursor, code) — install Cursor/VS Code or launch manually"
                .into(),
        )
    })?;
    let path = task.repo_path.display().to_string();
    spawn_task_command(task, state, &editor, &[&path])
}

/// Open Chromium (or configured browser) inside the task container when isolation is on.
pub fn launch_task_browser(task: &Task, state: &SessionState) -> Result<()> {
    let browser = resolve_browser_command().ok_or_else(|| {
        TskError::Other(
            "no browser found (looked for chromium, chrome, brave, firefox) — set $BROWSER"
                .into(),
        )
    })?;
    spawn_task_command(task, state, &browser, &[])
}

fn spawn_task_command(
    task: &Task,
    state: &SessionState,
    program: &str,
    args: &[&str],
) -> Result<()> {
    let cfg = load_config()?;
    let env = task_env::build_task_env(state, task, &cfg.tasks_base_dir, None);

    if task.container_isolation {
        ensure_container_ready(task)?;
        let child = distrobox::run_in_container(&task.container_name, program, args)?;
        // Detach: drop wait — GUI apps stay alive under Distrobox.
        std::mem::forget(child);
        let _ = env;
        return Ok(());
    }

    let mut cmd = Command::new(program);
    task_env::apply_env(&mut cmd, &env);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    cmd.spawn()
        .map_err(|e| TskError::Other(format!("failed to launch `{program}`: {e}")))?;
    Ok(())
}

fn ensure_container_ready(task: &Task) -> Result<()> {
    if !distrobox::container_exists(&task.container_name) {
        let cfg = load_config()?;
        let task_home = crate::task_cleanup::task_data_dir(&cfg, &task.id);
        eprintln!(
            "tsk: container `{}` missing — creating before launch…",
            task.container_name
        );
        distrobox::create_container(
            &task.container_name,
            &task_home,
            &cfg.distrobox_image,
        )?;
    }
    distrobox::start_container(&task.container_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editor_candidates_include_cursor() {
        assert!(EDITOR_CANDIDATES.contains(&"cursor"));
        assert!(EDITOR_CANDIDATES.contains(&"code"));
    }

    #[test]
    fn browser_candidates_include_chromium() {
        assert!(BROWSER_CANDIDATES.contains(&"chromium"));
    }
}
