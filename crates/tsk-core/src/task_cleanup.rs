//! Tear-down helpers for archive and delete.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::config::TskConfig;
use crate::distrobox;
use crate::error::{TskError, Result};
use crate::hyprland::{self, HyprWindow};
use crate::models::{SessionState, Task};
use crate::task_paths::is_managed_task_checkout;
use crate::terminal::{TUI_WINDOW_CLASS, TUI_WINDOW_TITLE};
use crate::workspaces::{is_global_workspace_slot, task_owned_workspace_names};
use crate::vcs::{
    detach_linked_checkout, jj_workspace_name_for_task, reattach_linked_checkout,
    remove_linked_checkout,
};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TaskTeardownPreview {
    pub window_count: usize,
    pub data_dir: PathBuf,
    pub container_name: String,
    pub container_exists: bool,
}

pub fn task_data_dir(config: &TskConfig, task_id: &str) -> PathBuf {
    config.tasks_base_dir.join(task_id)
}

pub fn preview_teardown(config: &TskConfig, task: &Task) -> Result<TaskTeardownPreview> {
    Ok(TaskTeardownPreview {
        window_count: count_task_windows(config, task)?,
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

pub fn count_task_windows(config: &TskConfig, task: &Task) -> Result<usize> {
    Ok(clients_for_task(config, task)?.len())
}

pub fn client_belongs_to_task(client: &HyprWindow, config: &TskConfig, task: &Task) -> bool {
    if client.title == TUI_WINDOW_TITLE || client.class_name == TUI_WINDOW_CLASS {
        return false;
    }
    if client
        .workspace_name
        .parse::<u32>()
        .ok()
        .is_some_and(|slot| is_global_workspace_slot(slot, &config.global_workspace_slots))
    {
        return false;
    }
    let workspace_names: HashSet<String> = task_owned_workspace_names(
        &task.id,
        config.default_workspace_count,
        &config.global_workspace_slots,
    )
    .into_iter()
    .collect();
    let title_prefix = format!("[{}]", task.id);
    workspace_names.contains(&client.workspace_name)
        || client.title.starts_with(&title_prefix)
}

pub fn clients_for_task(config: &TskConfig, task: &Task) -> Result<Vec<HyprWindow>> {
    if !hyprland::available() {
        return Ok(Vec::new());
    }
    Ok(hyprland::get_clients()?
        .into_iter()
        .filter(|client| client_belongs_to_task(client, config, task))
        .collect())
}

pub fn close_task_windows(config: &TskConfig, task: &Task) -> Result<usize> {
    let clients = clients_for_task(config, task)?;
    for client in &clients {
        hyprland::close_window(&client.address);
    }
    Ok(clients.len())
}

pub fn start_task_container(task: &Task) -> Result<()> {
    distrobox::start_container(&task.container_name)
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
    state
        .last_monitor_workspace
        .remove(&format!("task:{task_id}"));
    if state.current_task_id.as_deref() == Some(task_id) {
        state.current_task_id = None;
        state.context_mode = crate::models::ContextMode::Default;
    }
}

/// Window close, container stop, and checkout detach for archive — does not touch session state.
pub fn run_archive_teardown(config: &TskConfig, task: &Task) -> Result<()> {
    let _closed = close_task_windows(config, task)?;
    if let Err(err) = stop_task_container(task) {
        eprintln!(
            "tsk: archive task {}: stop container {}: {err}",
            task.id, task.container_name
        );
    }
    if let Err(err) = detach_task_checkout(config, task) {
        eprintln!("tsk: archive task {}: detach checkout: {err}", task.id);
    }
    Ok(())
}

pub fn detach_task_checkout(config: &TskConfig, task: &Task) -> Result<()> {
    if !is_managed_task_checkout(&task.repo_path, &config.tasks_base_dir, &task.id) {
        return Ok(());
    }
    let source = task.source_repo_path.as_deref();
    let name = jj_workspace_name_for_task(&task.id);
    detach_linked_checkout(&task.repo_path, source, Some(&name))
}

pub fn reattach_task_checkout(config: &TskConfig, task: &Task) -> Result<()> {
    if !is_managed_task_checkout(&task.repo_path, &config.tasks_base_dir, &task.id) {
        return Ok(());
    }
    let source = task.source_repo_path.as_deref();
    let name = jj_workspace_name_for_task(&task.id);
    reattach_linked_checkout(&task.repo_path, source, Some(&name))
}

pub fn remove_task_checkout(config: &TskConfig, task: &Task) -> Result<()> {
    if !is_managed_task_checkout(&task.repo_path, &config.tasks_base_dir, &task.id) {
        return Ok(());
    }
    let source = task.source_repo_path.as_deref();
    let name = jj_workspace_name_for_task(&task.id);
    remove_linked_checkout(&task.repo_path, source, Some(&name))
}

pub fn remove_task_data_dir(config: &TskConfig, task: &Task) -> Result<()> {
    let task_home = task_data_dir(config, task.id.as_str());
    if task_home.exists() {
        remove_dir_all(&task_home)?;
    }
    Ok(())
}

fn remove_dir_all(path: &Path) -> Result<()> {
    std::fs::remove_dir_all(path).map_err(|source| TskError::Write {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::TaskStatus;

    fn sample_task() -> Task {
        Task {
            id: "auth-fix".into(),
            name: "Auth Fix".into(),
            status: TaskStatus::Active,
            repo_url: None,
            repo_path: "/tmp".into(),
            source_repo_path: None,
            branch: None,
            container_name: "tsk-auth-fix".into(),
            workspace_count: 10,
            browser_profile: None,
            created_at: chrono::Utc::now(),
            last_active_at: chrono::Utc::now(),
            agent_notes_path: None,
            ports: vec![],
        }
    }

    fn sample_config() -> TskConfig {
        TskConfig {
            global_workspace_slots: vec![1, 10],
            ..TskConfig::default()
        }
    }

    fn sample_client(workspace_name: &str, title: &str) -> HyprWindow {
        HyprWindow {
            address: "0x1".into(),
            title: title.into(),
            class_name: "kitty".into(),
            workspace: 1,
            workspace_name: workspace_name.into(),
            pid: Some(1),
        }
    }

    #[test]
    fn client_belongs_to_task_counts_task_workspace_windows() {
        let config = sample_config();
        let task = sample_task();
        let client = sample_client("auth-fix-2", "editor");
        assert!(client_belongs_to_task(&client, &config, &task));
    }

    #[test]
    fn client_belongs_to_task_counts_title_tagged_windows() {
        let config = sample_config();
        let task = sample_task();
        let client = sample_client("auth-fix-5", "[auth-fix] terminal");
        assert!(client_belongs_to_task(&client, &config, &task));
    }

    #[test]
    fn client_belongs_to_task_ignores_global_workspace_windows() {
        let config = sample_config();
        let task = sample_task();
        let global = sample_client("1", "[auth-fix] terminal");
        assert!(!client_belongs_to_task(&global, &config, &task));
        let global_ten = sample_client("10", "browser");
        assert!(!client_belongs_to_task(&global_ten, &config, &task));
    }

    #[test]
    fn client_belongs_to_task_ignores_task_manager_tui() {
        let config = sample_config();
        let task = sample_task();
        let tui = HyprWindow {
            address: "0x2".into(),
            title: TUI_WINDOW_TITLE.into(),
            class_name: TUI_WINDOW_CLASS.into(),
            workspace: 1,
            workspace_name: "auth-fix-2".into(),
            pid: Some(1),
        };
        assert!(!client_belongs_to_task(&tui, &config, &task));
    }

    #[test]
    fn client_belongs_to_task_ignores_global_slot_task_name() {
        let config = sample_config();
        let task = sample_task();
        // Slot 1 is global, so auth-fix-1 is not a live Hyprland workspace name.
        let client = sample_client("auth-fix-1", "orphan");
        assert!(!client_belongs_to_task(&client, &config, &task));
    }
}
