//! Installed LAE binary resolution and login-shell PATH lookup.

use std::path::PathBuf;
use std::process::Command;

use crate::config::LaeConfig;

/// Installed `lae` binary, or `LAE` env when set to an executable path.
pub fn resolve_lae_binary(cfg: &LaeConfig) -> PathBuf {
    std::env::var("LAE")
        .ok()
        .map(PathBuf::from)
        .filter(|p| p.is_file())
        .unwrap_or_else(|| cfg.install_hypr_share_dir.join("bin/lae"))
}

/// Resolve a command through the user's login shell — Hyprland exec often has a stripped PATH.
pub(crate) fn command_v_login(name: &str) -> Option<String> {
    let script = format!("command -v {}", shell_quote(name));
    let output = Command::new("sh").arg("-lc").arg(script).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!path.is_empty()).then_some(path)
}

pub(crate) fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_quote_escapes_single_quotes() {
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
    }
}
