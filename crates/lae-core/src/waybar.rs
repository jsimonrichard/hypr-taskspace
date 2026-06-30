use std::collections::{HashMap, HashSet};
use std::fs;
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::context_sync;
use crate::error::{LaeError, Result};
use crate::hyprland;
use crate::models::SessionState;
use crate::registry::Registry;
use crate::taskspaces::visible_default_workspace_count;
use crate::workspaces::allowed_workspace_names;
use crate::xdg::{ensure_parent, lae_runtime_dir, lae_waybar_file, lae_waybar_modules_cache};

pub const WAYBAR_MODULE_COUNT: usize = 10;
pub const ACTIVE_WORKSPACE_ICON: &str = "󱓻";
pub const WAYBAR_SIGNAL: i32 = 11;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct WaybarModuleJson {
    #[serde(default)]
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tooltip: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class: Option<String>,
}

pub type WaybarModulesCache = HashMap<String, WaybarModuleJson>;

#[derive(Debug, Clone, Serialize)]
struct WaybarData {
    taskspace: String,
    context_mode: String,
    task_id: Option<String>,
    task_name: Option<String>,
    workspaces: Vec<String>,
    workspace_count: usize,
    visible_workspace_count: u32,
    occupied_workspace_indices: Vec<i32>,
    active_workspace: i32,
    active_workspace_name: Option<String>,
}

pub fn read_waybar_modules_cache() -> Result<WaybarModulesCache> {
    let path = lae_waybar_modules_cache()?;
    let raw = fs::read_to_string(&path).map_err(|source| LaeError::Read {
        path: path.clone(),
        source,
    })?;
    serde_json::from_str(&raw).map_err(|source| LaeError::Parse { path, source })
}

pub fn write_modules_cache(modules: &WaybarModulesCache, notify: bool) -> Result<()> {
    let path = lae_waybar_modules_cache()?;
    ensure_parent(&path)?;
    let tmp = path.with_extension("tmp");
    let body = serde_json::to_string(modules).map_err(|e| LaeError::Other(e.to_string()))?;
    fs::write(&tmp, format!("{body}\n")).map_err(|source| LaeError::Write {
        path: tmp.clone(),
        source,
    })?;
    fs::rename(&tmp, &path).map_err(|source| LaeError::Write {
        path: path.clone(),
        source,
    })?;
    if notify {
        notify_waybar();
    }
    Ok(())
}

pub fn notify_waybar() {
    if Command::new("which").arg("waybar").output().is_ok_and(|o| o.status.success()) {
        let _ = Command::new("pkill")
            .args(["-RTMIN+11", "waybar"])
            .status();
    }
}

pub fn build_all_modules(state: &SessionState, sync: bool) -> WaybarModulesCache {
    let mut state = state.clone();
    if sync {
        context_sync::sync_from_active_workspace(&mut state);
    }
    let occupied = fetch_occupied_indices(&state);
    build_all_modules_with_active_name(&state, None, &occupied)
}

/// Build module JSON using a known active workspace name — no `hyprctl` for focus.
pub fn build_all_modules_for_active_name(
    state: &SessionState,
    active_name: &str,
    occupied: &HashSet<i32>,
) -> WaybarModulesCache {
    let mut state = state.clone();
    context_sync::sync_from_workspace_name(&mut state, active_name);
    build_all_modules_with_active_name(&state, Some(active_name), occupied)
}

pub fn fetch_occupied_indices(state: &SessionState) -> HashSet<i32> {
    let allowed = allowed_workspace_names(state);
    occupied_relative_indices(&allowed)
}

/// Hyprland workspace names that currently have windows (for the active taskspace).
pub fn fetch_occupied_names(state: &SessionState) -> HashSet<String> {
    let allowed: HashSet<String> = allowed_workspace_names(state).into_iter().collect();
    occupied_names(&allowed)
}

fn build_all_modules_with_active_name(
    state: &SessionState,
    active_name: Option<&str>,
    occupied: &HashSet<i32>,
) -> WaybarModulesCache {
    let data = build_waybar_data_with(state, active_name, occupied);
    let mut modules = HashMap::new();
    modules.insert("task".into(), task_module(&data));
    for index in 1..=WAYBAR_MODULE_COUNT {
        modules.insert(
            workspace_module_key(index),
            workspace_module(&data, index),
        );
    }
    modules
}

pub fn refresh_modules_cache(registry: &Registry, notify: bool) -> Result<bool> {
    let mut state = registry.load_state()?;
    let changed = context_sync::sync_from_active_workspace(&mut state);
    let modules = build_all_modules(&state, false);
    let previous = read_waybar_modules_cache().ok();
    if Some(&modules) != previous.as_ref() {
        write_modules_cache(&modules, notify)?;
        write_waybar_json(&state)?;
    }
    if changed {
        registry.save_state(&state)?;
    }
    Ok(changed)
}

fn write_waybar_json(state: &SessionState) -> Result<()> {
    let path = lae_waybar_file()?;
    ensure_parent(&path)?;
    let data = build_waybar_data(state, true);
    let body = serde_json::to_string(&data).map_err(|e| LaeError::Other(e.to_string()))?;
    fs::write(&path, format!("{body}\n")).map_err(|source| LaeError::Write { path, source })
}

fn build_waybar_data(state: &SessionState, sync: bool) -> WaybarData {
    let mut state = state.clone();
    if sync {
        context_sync::sync_from_active_workspace(&mut state);
    }
    let allowed = allowed_workspace_names(&state);
    let occupied = occupied_relative_indices(&allowed);
    build_waybar_data_with(&state, None, &occupied)
}

fn build_waybar_data_with(
    state: &SessionState,
    active_name: Option<&str>,
    occupied: &HashSet<i32>,
) -> WaybarData {
    let allowed = allowed_workspace_names(state);

    let active_name = active_name.map(str::to_string).or_else(|| {
        hyprland::get_active_workspace()
            .ok()
            .flatten()
            .map(|ws| ws.name)
    });

    let mut active_rel = *state
        .last_workspace
        .get(&state.taskspace_key())
        .unwrap_or(&1);
    if let Some(ref name) = active_name {
        if let Some(idx) = allowed.iter().position(|n| n == name) {
            active_rel = (idx + 1) as i32;
        }
    }

    let task = state
        .current_task_id
        .as_ref()
        .and_then(|id| state.tasks.get(id));

    let visible = visible_default_workspace_count(state, &allowed, active_rel, occupied);

    WaybarData {
        taskspace: state.taskspace_label(),
        context_mode: state.context_mode.as_str().into(),
        task_id: state.current_task_id.clone(),
        task_name: task.map(|t| t.name.clone()),
        workspaces: allowed.clone(),
        workspace_count: allowed.len(),
        visible_workspace_count: visible,
        occupied_workspace_indices: occupied.iter().copied().collect(),
        active_workspace: active_rel,
        active_workspace_name: active_name,
    }
}

fn occupied_relative_indices(allowed: &[String]) -> HashSet<i32> {
    let mut occupied = HashSet::new();
    if !hyprland::available() {
        return occupied;
    }
    if let Ok(clients) = hyprland::get_clients() {
        for client in clients {
            if let Some(idx) = allowed.iter().position(|n| n == &client.workspace_name) {
                occupied.insert((idx + 1) as i32);
            }
        }
    }
    occupied
}

fn occupied_names(allowed: &HashSet<String>) -> HashSet<String> {
    let mut occupied = HashSet::new();
    if !hyprland::available() {
        return occupied;
    }
    if let Ok(clients) = hyprland::get_clients() {
        for client in clients {
            if allowed.contains(&client.workspace_name) {
                occupied.insert(client.workspace_name);
            }
        }
    }
    occupied
}

fn task_module(data: &WaybarData) -> WaybarModuleJson {
    if let Some(task_id) = &data.task_id {
        let name = data.task_name.as_deref().unwrap_or(task_id);
        return WaybarModuleJson {
            text: format!("󱓝 {name}"),
            tooltip: Some(format!(
                "Task: {name}\nWorkspaces: {}",
                data.workspaces.join(", ")
            )),
            class: Some("task".into()),
        };
    }
    WaybarModuleJson {
        text: "󰣇 default".into(),
        tooltip: Some(format!(
            "Default taskspace workspaces: {}",
            data.workspaces.join(", ")
        )),
        class: Some("default".into()),
    }
}

fn workspace_module(data: &WaybarData, index: usize) -> WaybarModuleJson {
    if index < 1 || index > WAYBAR_MODULE_COUNT {
        return hidden_module();
    }
    if index > data.workspace_count {
        return hidden_module();
    }

    let is_active = data.active_workspace == index as i32;
    if index as u32 > data.visible_workspace_count && !is_active {
        return hidden_module();
    }

    let workspace_name = data
        .workspaces
        .get(index - 1)
        .cloned()
        .unwrap_or_else(|| index.to_string());
    let occupied: HashSet<i32> = data.occupied_workspace_indices.iter().copied().collect();
    let mut classes = Vec::new();
    if is_active {
        classes.push("active");
    } else if !occupied.contains(&(index as i32)) {
        classes.push("empty");
    }

    WaybarModuleJson {
        text: if is_active {
            ACTIVE_WORKSPACE_ICON.into()
        } else {
            workspace_label(index)
        },
        tooltip: Some(workspace_name),
        class: Some(if classes.is_empty() {
            "idle".into()
        } else {
            classes.join(" ")
        }),
    }
}

fn hidden_module() -> WaybarModuleJson {
    WaybarModuleJson {
        text: String::new(),
        tooltip: None,
        class: Some("hidden".into()),
    }
}

pub fn workspace_label(index: usize) -> String {
    if index == 10 {
        "0".into()
    } else {
        index.to_string()
    }
}

pub fn workspace_module_key(index: usize) -> String {
    format!("workspace-{index}")
}

pub fn ensure_runtime_dir() -> Result<()> {
    let dir = lae_runtime_dir()?;
    ensure_parent(&dir.join("_")).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_labels_match_python() {
        assert_eq!(workspace_label(1), "1");
        assert_eq!(workspace_label(10), "0");
    }
}
