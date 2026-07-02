//! Tear-down helpers for archive and delete.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::config::LaeConfig;
use crate::distrobox;
use crate::error::{LaeError, Result};
use crate::hyprland::{self, HyprWindow};
use crate::models::{SessionState, Task};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TaskTeardownPreview {
    pub window_count: usize,
    pub data_dir: PathBuf,
    pub container_name: String,
    pub container_exists: bool,
}

pub fn task_data_dir(config: &LaeConfig, task_id: &str) -> PathBuf {
    config.tasks_base_dir.join(task_id)
}

pub fn preview_teardown(config: &LaeConfig, task: &Task) -> Result<TaskTeardownPreview> {
    Ok(TaskTeardownPreview {
        window_count: count_task_windows(task)?,
        data_dir: task_data_dir(config, task.id.as_str()),
        container_name: task.container_name.clone(),
        container_exists: distrobox::container_exists(&task.container_name),
    })
}

pub fn is_active_task_context(state: &SessionState, task: &Task) -> bool {
    if state.current_task_id.as_deref() == Some(task.id.as_str()) {
        return true;
    }
    if !hyprland::available() {
        return false;
    }
    let Ok(Some(active)) = hyprland::get_active_workspace() else {
        return false;
    };
    let workspace_names: HashSet<String> = task.workspace_names().into_iter().collect();
    workspace_names.contains(&active.name)
}

pub fn count_task_windows(task: &Task) -> Result<usize> {
    Ok(clients_for_task(task)?.len())
}

pub fn clients_for_task(task: &Task) -> Result<Vec<HyprWindow>> {
    if !hyprland::available() {
        return Ok(Vec::new());
    }
    let workspace_names: HashSet<String> = task.workspace_names().into_iter().collect();
    let title_prefix = format!("[{}]", task.id);
    Ok(hyprland::get_clients()?
        .into_iter()
        .filter(|client| {
            workspace_names.contains(&client.workspace_name)
                || client.title.starts_with(&title_prefix)
        })
        .collect())
}

pub fn close_task_windows(task: &Task) -> Result<usize> {
    let clients = clients_for_task(task)?;
    for client in &clients {
        hyprland::close_window(&client.address);
    }
    Ok(clients.len())
}

pub fn stop_task_container(task: &Task) -> Result<()> {
    distrobox::stop_container(&task.container_name)
}

pub fn remove_task_container(task: &Task) -> Result<()> {
    distrobox::remove_container(&task.container_name)
}

pub fn purge_task_windows(state: &mut SessionState, task_id: &str) {
    state.windows.retain(|_, record| {
        record.task_id.as_deref() != Some(task_id)
    });
}

pub fn purge_task_session_keys(state: &mut SessionState, task_id: &str) {
    state.last_workspace.remove(&format!("task:{task_id}"));
    if state.current_task_id.as_deref() == Some(task_id) {
        state.current_task_id = None;
        state.context_mode = crate::models::ContextMode::Default;
    }
}

pub fn remove_task_data_dir(config: &LaeConfig, task: &Task) -> Result<()> {
    let task_home = task_data_dir(config, task.id.as_str());
    if task_home.exists() {
        remove_dir_all(&task_home)?;
    }
    Ok(())
}

fn remove_dir_all(path: &Path) -> Result<()> {
    std::fs::remove_dir_all(path).map_err(|source| LaeError::Write {
        path: path.to_path_buf(),
        source,
    })
}
