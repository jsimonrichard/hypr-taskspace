//! Runtime slot → Hyprland workspace target cache for instant keybinds.

use std::fs;
use std::path::PathBuf;

use crate::error::Result;
use crate::hypr_log;
use crate::hyprland;
use crate::models::SessionState;
use crate::workspaces::bar_workspace_names;
use crate::xdg::{ensure_parent, tsk_runtime_dir};

pub fn slot_cache_path(relative: i32) -> Result<PathBuf> {
    Ok(tsk_runtime_dir()?.join(format!("slot-{relative}")))
}

pub fn read_slot_target(relative: i32) -> Option<String> {
    let path = slot_cache_path(relative).ok()?;
    let text = fs::read_to_string(path).ok()?;
    let target = text.trim().to_string();
    if target.is_empty() {
        None
    } else {
        Some(target)
    }
}

/// Switch via `hyprctl dispatch` using the slot cache (keybind hot path).
pub fn switch_slot(relative: i32) -> Option<String> {
    let target = read_slot_target(relative)?;
    hypr_log::scoped(format!("switch_slot cache slot {relative} → {target}"), || {
        hyprland::switch_workspace_for_navigation(&target);
    });
    Some(target)
}

pub fn write_slot_cache(state: &SessionState) {
    let Ok(dir) = tsk_runtime_dir() else {
        return;
    };
    let _ = ensure_parent(&dir.join("_"));
    let bar = bar_workspace_names(state);
    for (i, name) in bar.iter().enumerate().take(10) {
        let path = dir.join(format!("slot-{}", i + 1));
        let _ = fs::write(path, name);
    }
}
