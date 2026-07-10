//! Launch terminals — task manager TUI and task-scoped host/container shells.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::{load_config, TskConfig};
use crate::distrobox;
use crate::error::{TskError, Result};
use crate::binary::{resolve_tsk_spawn_binary, command_v_login};
use crate::models::Task;
use crate::registry::Registry;
use crate::task_env;

pub const TERMINAL_FALLBACKS: &[&str] = &[
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

/// Open a terminal in the task checkout (Distrobox enter when isolation is enabled).
pub fn launch_task_terminal(task: &Task, env: &[(String, String)]) -> Result<()> {
    let cfg = load_config()?;
    crate::vcs::ensure_task_checkout_ready(task, &cfg)?;
    let term = resolve_terminal_command(&cfg)?;
    let title = format!("[{}] terminal", task.id);

    if task.container_isolation {
        // Open the host terminal immediately so Distrobox create/enter can show
        // pull/start progress in that window instead of blocking before spawn.
        if distrobox::container_exists(&task.container_name) {
            let argv = distrobox::shell_enter_argv(&task.container_name, &task.repo_path);
            let (program, args) = argv
                .split_first()
                .ok_or_else(|| TskError::Other("empty distrobox enter argv".into()))?;
            let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
            return spawn_terminal_command(
                &term,
                Path::new(program),
                &arg_refs,
                Some(&task.repo_path),
                &title,
                "org.tsk.task-terminal",
                env,
            );
        }

        // Container missing (e.g. deferred create never finished) — create inside
        // the terminal so the user sees progress/errors, then enter.
        let image = distrobox::resolve_create_image(&cfg.distrobox_image)?;
        let task_home = crate::task_cleanup::task_data_dir(&cfg, &task.id);
        let name_q = shell_single_quote(&task.container_name);
        let image_q = shell_single_quote(&image);
        let home_q = shell_single_quote(&task_home.display().to_string());
        let enter = distrobox::shell_enter_argv(&task.container_name, &task.repo_path)
            .into_iter()
            .map(|a| shell_single_quote(&a))
            .collect::<Vec<_>>()
            .join(" ");
        let script = format!(
            "set -euo pipefail\n\
             printf 'Creating Distrobox %s (was missing)…\\n' {name_q}\n\
             distrobox create -Y --name {name_q} --image {image_q} --home {home_q} --no-entry\n\
             exec {enter}\n"
        );
        return spawn_terminal_command(
            &term,
            Path::new("bash"),
            &["-lc", &script],
            Some(&task.repo_path),
            &title,
            "org.tsk.task-terminal",
            env,
        );
    }

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

fn shell_single_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

pub fn resolve_terminal_command(cfg: &TskConfig) -> Result<String> {
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
    let cfg = load_config()?;
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".into());
    let base = terminal_base_name(term);
    let mut cmd = Command::new(term);
    task_env::apply_task_process_env(&mut cmd, env, &cfg);

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

pub fn spawn_terminal_command(
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
    let cfg = load_config()?;
    task_env::apply_task_process_env(&mut cmd, env, &cfg);
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
            ]);
            if let Some(cwd) = cwd {
                cmd.args([&format!("--directory={}", cwd.display())]);
            }
            cmd.args(["--"]);
            cmd.arg(program);
            cmd.args(args);
        }
        "alacritty" => {
            cmd.args(["--class", class, "-t", title]);
            if let Some(cwd) = cwd {
                cmd.args(["--working-directory", &cwd.display().to_string()]);
            }
            cmd.args(["-e"]);
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
