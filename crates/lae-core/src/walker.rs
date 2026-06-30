//! Walker / task menu launcher and LAE binary resolution.

use std::path::PathBuf;
use std::process::Command;

use crate::config::{load_config, LaeConfig};
use crate::error::{LaeError, Result};

/// Installed `lae` binary, or `LAE` env when set to an executable path.
pub fn resolve_lae_binary(cfg: &LaeConfig) -> PathBuf {
    std::env::var("LAE")
        .ok()
        .map(PathBuf::from)
        .filter(|p| p.is_file())
        .unwrap_or_else(|| cfg.install_hypr_share_dir.join("bin/lae"))
}

pub fn launch_task_menu() -> Result<()> {
    spawn_walker_menu()
}

pub fn spawn_walker_menu() -> Result<()> {
    let cfg = load_config()?;
    let launcher = resolve_walker_launcher(&cfg)?;
    let script = format!(
        "exec {} -m menus:laetasks --width 644 --minheight 300 --maxheight 630",
        shell_quote(&launcher)
    );
    Command::new("sh")
        .arg("-lc")
        .arg(script)
        .spawn()
        .map_err(|e| LaeError::Other(format!("failed to launch walker: {e}")))?;
    Ok(())
}

fn resolve_walker_launcher(cfg: &LaeConfig) -> Result<String> {
    for candidate in [
        cfg.walker_launch_command.as_str(),
        "omarchy-launch-walker",
        "walker",
    ] {
        if let Some(path) = command_v_login(candidate) {
            return Ok(path);
        }
    }
    Err(LaeError::Other(
        "walker or omarchy-launch-walker not found on PATH (try login shell)".into(),
    ))
}

/// Resolve a command through the user's login shell — Waybar often has a stripped PATH.
fn command_v_login(name: &str) -> Option<String> {
    let script = format!("command -v {}", shell_quote(name));
    let output = Command::new("sh").arg("-lc").arg(script).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!path.is_empty()).then_some(path)
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

pub fn menu_action_prefix(cfg: &LaeConfig) -> String {
    resolve_lae_binary(cfg).display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_quote_escapes_single_quotes() {
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
    }
}
