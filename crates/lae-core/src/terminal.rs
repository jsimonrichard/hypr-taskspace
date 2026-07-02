//! Launch the task manager TUI inside the user's terminal emulator.

use std::path::Path;
use std::process::Command;

use crate::config::{load_config, LaeConfig};
use crate::error::{LaeError, Result};
use crate::binary::{command_v_login, resolve_lae_binary};

const TERMINAL_FALLBACKS: &[&str] = &[
    "xdg-terminal-exec",
    "alacritty",
    "ghostty",
    "kitty",
    "foot",
    "wezterm",
];

/// Window title — matched by Hyprland rules in share/hypr/window-rules.conf.
pub const TUI_WINDOW_TITLE: &str = "lae tasks";
/// Wayland app-id / terminal class for Hyprland float rules.
pub const TUI_WINDOW_CLASS: &str = "org.lae.task-tui";

pub fn launch_task_tui() -> Result<()> {
    let cfg = load_config()?;
    let lae = resolve_lae_binary(&cfg);
    let term = resolve_terminal_command(&cfg)?;
    spawn_terminal(&term, &lae, &["task", "tui"])
}

fn resolve_terminal_command(cfg: &LaeConfig) -> Result<String> {
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

    Err(LaeError::Other(
        "no terminal emulator found — set [terminal].command in ~/.config/lae/config.toml \
         (Omarchy: xdg-terminal-exec or alacritty)"
            .into(),
    ))
}

fn spawn_terminal(term: &str, lae: &Path, args: &[&str]) -> Result<()> {
    let base = Path::new(term)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(term);

    let mut cmd = Command::new(term);
    match base {
        "xdg-terminal-exec" => {
            cmd.args([
                &format!("--app-id={TUI_WINDOW_CLASS}"),
                &format!("--title={TUI_WINDOW_TITLE}"),
                "--",
            ]);
            cmd.arg(lae);
            cmd.args(args);
        }
        "kitty" => {
            cmd.args([
                &format!("--class={TUI_WINDOW_CLASS}"),
                &format!("--title={TUI_WINDOW_TITLE}"),
                "--",
            ]);
            cmd.arg(lae);
            cmd.args(args);
        }
        "alacritty" => {
            cmd.args([
                "--class",
                TUI_WINDOW_CLASS,
                "-t",
                TUI_WINDOW_TITLE,
                "-e",
            ]);
            cmd.arg(lae);
            cmd.args(args);
        }
        "ghostty" => {
            cmd.args([
                &format!("--class={TUI_WINDOW_CLASS}"),
                &format!("--title={TUI_WINDOW_TITLE}"),
                "-e",
            ]);
            cmd.arg(lae);
            cmd.args(args);
        }
        "foot" => {
            cmd.args(["-a", TUI_WINDOW_CLASS, "-T", TUI_WINDOW_TITLE, "-e"]);
            cmd.arg(lae);
            cmd.args(args);
        }
        "wezterm" | "kgx" | "xfce4-terminal" => {
            cmd.args(["--class", TUI_WINDOW_CLASS, "-e"]);
            cmd.arg(lae);
            cmd.args(args);
        }
        _ => {
            cmd.args(["-e"]);
            cmd.arg(lae);
            cmd.args(args);
        }
    }

    cmd.spawn().map_err(|e| {
        LaeError::Other(format!("failed to launch terminal `{term}`: {e}"))
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallbacks_include_xdg_terminal_exec() {
        assert!(TERMINAL_FALLBACKS.contains(&"xdg-terminal-exec"));
    }
}
