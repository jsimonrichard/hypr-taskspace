use std::process::Command;

use crate::error::Result;

pub fn restart_waybar() -> bool {
    if command_exists("omarchy-restart-waybar") {
        let _ = Command::new("omarchy-restart-waybar").status();
        return true;
    }
    if command_exists("waybar") {
        let _ = Command::new("pkill").args(["-9", "-x", "waybar"]).status();
        let _ = Command::new("setsid").args(["waybar"]).status();
        return true;
    }
    false
}

pub fn apply_after_hypr() -> Result<Vec<String>> {
    let mut actions = Vec::new();
    if command_exists("hyprctl") {
        let _ = Command::new("hyprctl").arg("reload").status();
        actions.push("reloaded Hyprland config".into());
    } else {
        actions.push("Hyprland not active — run `hyprctl reload` after login".into());
    }
    Ok(actions)
}

pub fn apply_after_waybar() -> Vec<String> {
    if restart_waybar() {
        vec!["restarted Waybar".into()]
    } else if command_exists("waybar") {
        vec!["run `omarchy-restart-waybar` to apply Waybar changes".into()]
    } else {
        Vec::new()
    }
}

fn command_exists(name: &str) -> bool {
    Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {name}"))
        .output()
        .is_ok_and(|o| o.status.success())
}
