//! Launch terminals — task manager TUI and task-scoped host shells.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::{load_config, TskConfig};
use crate::error::{TskError, Result};
use crate::binary::{resolve_tsk_spawn_binary, command_v_login};
use crate::models::Task;
use crate::registry::Registry;
use crate::task_env;

const TERMINAL_FALLBACKS: &[&str] = &[
    "xdg-terminal-exec",
    "alacritty",
    "ghostty",
    "kitty",
    "foot",
    "wezterm",
];

/// Window title — matched by Hyprland rules in share/hypr/window-rules.conf.
pub const TUI_WINDOW_TITLE: &str = "tsk tasks";
/// Wayland app-id / terminal class for Hyprland float rules.
pub const TUI_WINDOW_CLASS: &str = "org.tsk.task-tui";

pub fn launch_task_tui() -> Result<()> {
    let cfg = load_config()?;
    let registry = Registry::new(None, cfg.clone())?;
    let state = registry.load_state()?;
    let env = task_env::build_taskspace_env(&state, &cfg.tasks_base_dir);
    let tsk = resolve_tsk_spawn_binary(&cfg);
    let term = resolve_terminal_command(&cfg)?;
    spawn_terminal_command(
        &term,
        &tsk,
        &["task", "tui"],
        None,
        TUI_WINDOW_TITLE,
        TUI_WINDOW_CLASS,
        &env,
    )
}

/// Open a host terminal in the task's linked checkout (no container isolation).
pub fn launch_task_terminal(task: &Task, env: &[(String, String)]) -> Result<()> {
    let cfg = load_config()?;
    crate::vcs::ensure_task_checkout_ready(task, &cfg)?;
    let term = resolve_terminal_command(&cfg)?;
    let title = format!("[{}] terminal", task.id);
    spawn_host_shell(&term, &task.repo_path, &title, env)
}

pub fn launch_host_terminal(cwd: Option<PathBuf>, env: &[(String, String)]) -> Result<()> {
    let cfg = load_config()?;
    let term = resolve_terminal_command(&cfg)?;
    let cwd = cwd.or_else(|| std::env::var_os("HOME").map(PathBuf::from));
    spawn_host_shell(
        &term,
        cwd.as_deref().unwrap_or(Path::new(".")),
        "terminal",
        env,
    )
}

fn resolve_terminal_command(cfg: &TskConfig) -> Result<String> {
    if let Some(path) = command_v_login(&cfg.terminal_command) {
        return Ok(path);
    }

    if let Ok(term) = std::env::var("TERMINAL") {
        let term = term.trim();
        if !term.is_empty() {
            if term.contains('/') && Path::new(term).is_file() {
                return Ok(term.to_string());
            }
            if let Some(path) = command_v_login(term) {
                return Ok(path);
            }
        }
    }

    for candidate in TERMINAL_FALLBACKS {
        if let Some(path) = command_v_login(candidate) {
            return Ok(path);
        }
    }

    Err(TskError::Other(
        "no terminal emulator found — set [terminal].command in ~/.config/tsk/config.toml \
         (Omarchy: xdg-terminal-exec or alacritty)"
            .into(),
    ))
}

fn spawn_host_shell(
    term: &str,
    cwd: &Path,
    title: &str,
    env: &[(String, String)],
) -> Result<()> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".into());
    let base = terminal_base_name(term);
    let mut cmd = Command::new(term);
    task_env::apply_env(&mut cmd, env);

    match base {
        "xdg-terminal-exec" => {
            cmd.args([
                &format!("--title={title}"),
                &format!("--dir={}", cwd.display()),
            ]);
        }
        "kitty" => {
            cmd.args([
                &format!("--title={title}"),
                &format!("--directory={}", cwd.display()),
            ]);
        }
        "alacritty" => {
            cmd.args([
                "-t",
                title,
                "--working-directory",
                &cwd.display().to_string(),
            ]);
        }
        "ghostty" => {
            cmd.args([
                &format!("--title={title}"),
                &format!("--working-directory={}", cwd.display()),
            ]);
        }
        "foot" => {
            cmd.args(["-T", title, "-D", &cwd.display().to_string()]);
        }
        "wezterm" | "kgx" | "xfce4-terminal" => {
            cmd.args(["--title", title, "--working-directory", &cwd.display().to_string()]);
        }
        _ => {
            cmd.args(["--working-directory", &cwd.display().to_string()]);
        }
    }

    cmd.arg(&shell);
    cmd.spawn().map_err(|e| {
        TskError::Other(format!("failed to launch terminal `{term}`: {e}"))
    })?;
    Ok(())
}

fn spawn_terminal_command(
    term: &str,
    program: &Path,
    args: &[&str],
    cwd: Option<&Path>,
    title: &str,
    class: &str,
    env: &[(String, String)],
) -> Result<()> {
    let base = terminal_base_name(term);
    let mut cmd = Command::new(term);
    if let Ok(config) = std::env::var("TSK_CONFIG") {
        cmd.env("TSK_CONFIG", config);
    }
    task_env::apply_env(&mut cmd, env);
    // Avoid passing the Hyprland wrapper path through to the spawned TUI process.
    cmd.env_remove("TSK");
    match base {
        "xdg-terminal-exec" => {
            cmd.args([
                &format!("--app-id={class}"),
                &format!("--title={title}"),
            ]);
            if let Some(cwd) = cwd {
                cmd.arg(format!("--dir={}", cwd.display()));
            }
            cmd.args(["--"]);
            cmd.arg(program);
            cmd.args(args);
        }
        "kitty" => {
            cmd.args([
                &format!("--class={class}"),
                &format!("--title={title}"),
                "--",
            ]);
            if let Some(cwd) = cwd {
                cmd.args([&format!("--directory={}", cwd.display())]);
            }
            cmd.arg(program);
            cmd.args(args);
        }
        "alacritty" => {
            cmd.args(["--class", class, "-t", title, "-e"]);
            cmd.arg(program);
            cmd.args(args);
        }
        "ghostty" => {
            cmd.args([
                &format!("--class={class}"),
                &format!("--title={title}"),
                "-e",
            ]);
            cmd.arg(program);
            cmd.args(args);
        }
        "foot" => {
            cmd.args(["-a", class, "-T", title, "-e"]);
            cmd.arg(program);
            cmd.args(args);
        }
        "wezterm" | "kgx" | "xfce4-terminal" => {
            cmd.args(["--class", class, "-e"]);
            cmd.arg(program);
            cmd.args(args);
        }
        _ => {
            cmd.args(["-e"]);
            cmd.arg(program);
            cmd.args(args);
        }
    }

    cmd.spawn().map_err(|e| {
        TskError::Other(format!("failed to launch terminal `{term}`: {e}"))
    })?;
    Ok(())
}

fn terminal_base_name(term: &str) -> &str {
    Path::new(term)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(term)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallbacks_include_xdg_terminal_exec() {
        assert!(TERMINAL_FALLBACKS.contains(&"xdg-terminal-exec"));
    }
}
