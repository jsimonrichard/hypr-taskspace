use std::collections::{HashMap, HashSet};
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::context_sync;
use crate::error::Result;
use crate::hyprland;
use crate::models::SessionState;
use crate::repos::{is_scratch_task, task_source_repo_path};
use crate::task_ids::{format_workspaces_tooltip, short_task_id, workspace_tooltip_label};
use crate::taskspaces::visible_default_workspace_count;
use crate::vcs::repo_label;
use crate::workspaces::allowed_workspace_names;
use crate::xdg::{ensure_parent, tsk_runtime_dir};

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
    repo_name: Option<String>,
    workspaces: Vec<String>,
    workspace_count: usize,
    visible_workspace_count: u32,
    occupied_workspace_indices: Vec<i32>,
    active_workspace: i32,
    active_workspace_name: Option<String>,
    global_workspace_slots: Vec<u32>,
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
    modules.insert("task".into(), task_module(&data, state));
    for index in 1..=WAYBAR_MODULE_COUNT {
        modules.insert(
            workspace_module_key(index),
            workspace_module(&data, index, state),
        );
    }
    modules
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
        repo_name: task.and_then(|t| {
            if is_scratch_task(t) {
                None
            } else {
                Some(repo_label(task_source_repo_path(t)))
            }
        }),
        workspaces: allowed.clone(),
        workspace_count: allowed.len(),
        visible_workspace_count: visible,
        occupied_workspace_indices: occupied.iter().copied().collect(),
        active_workspace: active_rel,
        active_workspace_name: active_name,
        global_workspace_slots: state.global_workspace_slots.clone(),
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

fn task_module(data: &WaybarData, state: &SessionState) -> WaybarModuleJson {
    if let Some(task_id) = &data.task_id {
        let name = data.task_name.as_deref().unwrap_or(task_id);
        let short_id = short_task_id(state, task_id);
        let text = if let Some(repo) = &data.repo_name {
            format!("󱓝 {repo}: {name}")
        } else {
            format!("󱓝 {name}")
        };
        let workspaces = format_workspaces_tooltip(&data.workspaces, state);
        let tooltip = if let Some(repo) = &data.repo_name {
            format!("Task: {name} · {short_id} ({repo})\nWorkspaces: {workspaces}")
        } else {
            format!("Task: {name} · {short_id}\nWorkspaces: {workspaces}")
        };
        return WaybarModuleJson {
            text,
            tooltip: Some(tooltip),
            class: Some("task".into()),
        };
    }
    WaybarModuleJson {
        text: "󰣇 default".into(),
        tooltip: Some(format!(
            "Default taskspace\nWorkspaces: {}",
            format_workspaces_tooltip(&data.workspaces, state)
        )),
        class: Some("default".into()),
    }
}

fn workspace_module(data: &WaybarData, index: usize, state: &SessionState) -> WaybarModuleJson {
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
    if data.context_mode == "task"
        && workspace_name
            .parse::<u32>()
            .ok()
            .is_some_and(|slot| data.global_workspace_slots.contains(&slot))
    {
        classes.push("global");
    }
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
        tooltip: Some(workspace_tooltip_label(Some(state), &workspace_name)),
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
    let dir = tsk_runtime_dir()?;
    ensure_parent(&dir.join("_")).map(|_| ())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use chrono::Utc;

    use crate::models::{ContextMode, Task, TaskStatus};

    use super::*;

    #[test]
    fn workspace_labels_use_digit_and_zero_for_ten() {
        assert_eq!(workspace_label(1), "1");
        assert_eq!(workspace_label(10), "0");
    }

    #[test]
    fn task_module_includes_repo_name() {
        let mut state = SessionState {
            context_mode: ContextMode::Task,
            current_task_id: Some("t1".into()),
            ..Default::default()
        };
        state.tasks.insert(
            "t1".into(),
            Task {
                id: "t1".into(),
                name: "Auth Fix".into(),
                status: TaskStatus::Active,
                repo_url: None,
                repo_path: PathBuf::from("/home/user/projects/my-app"),
                source_repo_path: Some(PathBuf::from("/home/user/projects/my-app")),
                branch: None,
                container_name: "tsk-t1".into(),
                workspace_count: 3,
                browser_profile: None,
                created_at: Utc::now(),
                last_active_at: Utc::now(),
                agent_notes_path: None,
                ports: vec![],
            },
        );

        let modules = build_all_modules(&state, false);
        let task = modules.get("task").expect("task module");
        assert_eq!(task.text, "󱓝 my-app: Auth Fix");
    }
}
