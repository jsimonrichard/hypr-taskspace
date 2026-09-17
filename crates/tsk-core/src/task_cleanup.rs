//! Tear-down helpers for archive and delete.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::config::TskConfig;
use crate::distrobox;
use crate::error::{Result, TskError};
use crate::hyprland::{self, HyprWindow};
use crate::models::{SessionState, Task};
use crate::repos::{is_scratch_task, paths_match, task_source_repo_path};
use crate::task_paths::{is_managed_task_checkout, task_workspace_dir};
use crate::terminal::{TUI_WINDOW_CLASS, TUI_WINDOW_TITLE};
use crate::vcs::{
    detach_linked_checkout, list_forgotten_jj_checkouts, list_linked_checkouts,
    reattach_linked_checkout, remove_linked_checkout,
};
use crate::workspaces::{is_global_workspace_slot, task_owned_workspace_names};

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
        container_exists: task.container_isolation
            && distrobox::container_exists(&task.container_name),
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
    workspace_names.contains(&client.workspace_name) || client.title.starts_with(&title_prefix)
}

pub fn clients_for_task(config: &TskConfig, task: &Task) -> Result<Vec<HyprWindow>> {
    if !config.hyprland_enabled || !hyprland::available() {
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
    if !task.container_isolation {
        return Ok(());
    }
    distrobox::start_container(&task.container_name)
}

pub fn stop_task_container(task: &Task) -> Result<()> {
    if !task.container_isolation {
        return Ok(());
    }
    distrobox::stop_container(&task.container_name)
}

pub fn remove_task_container(task: &Task) -> Result<()> {
    if !task.container_isolation {
        return Ok(());
    }
    distrobox::remove_container(&task.container_name)
}

pub fn purge_task_windows(state: &mut SessionState, task_id: &str) {
    state
        .windows
        .retain(|_, record| record.task_id.as_deref() != Some(task_id));
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
/// Browser tabs are snapshotted in `prepare_archive` before leaving the taskspace.
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

/// Checkouts of this task's source repo that live under the task home.
///
/// Membership comes from `git worktree list` / `jj workspace list` (and leftover
/// `.jj` dirs that still point at the source after `workspace forget`). Folder
/// names are not consulted.
pub fn owned_task_checkouts(config: &TskConfig, task: &Task) -> Result<Vec<(PathBuf, String)>> {
    let task_home = task_data_dir(config, task.id.as_str());
    let ws = task_workspace_dir(&task_home);
    let mut out = Vec::new();

    if is_scratch_task(task) {
        if is_managed_task_checkout(&task.repo_path, &config.tasks_base_dir, &task.id) {
            out.push((task.repo_path.clone(), task.id.clone()));
        }
        return Ok(out);
    }

    let source = task_source_repo_path(task);
    for (path, name) in list_linked_checkouts(source)? {
        if !is_managed_task_checkout(&path, &config.tasks_base_dir, &task.id) {
            continue;
        }
        if out.iter().any(|(existing, _)| paths_match(existing, &path)) {
            continue;
        }
        out.push((path, name));
    }

    if ws.is_dir() {
        for (path, name) in list_forgotten_jj_checkouts(source, &ws)? {
            if out.iter().any(|(existing, _)| paths_match(existing, &path)) {
                continue;
            }
            out.push((path, name));
        }
    }
    Ok(out)
}

fn for_each_owned_checkout(
    config: &TskConfig,
    task: &Task,
    mut op: impl FnMut(&Path, Option<&Path>, &str) -> Result<()>,
) -> Result<()> {
    let source = task.source_repo_path.as_deref();
    for (checkout, name) in owned_task_checkouts(config, task)? {
        op(&checkout, source, &name)?;
    }
    Ok(())
}

pub fn detach_task_checkout(config: &TskConfig, task: &Task) -> Result<()> {
    for_each_owned_checkout(config, task, |checkout, source, name| {
        detach_linked_checkout(checkout, source, Some(name))
    })
}

pub fn reattach_task_checkout(config: &TskConfig, task: &Task) -> Result<()> {
    for_each_owned_checkout(config, task, |checkout, source, name| {
        reattach_linked_checkout(checkout, source, Some(name))
    })
}

pub fn remove_task_checkout(config: &TskConfig, task: &Task) -> Result<()> {
    for_each_owned_checkout(config, task, |checkout, source, name| {
        remove_linked_checkout(checkout, source, Some(name))
    })
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
            container_isolation: false,
            workspace_count: 10,
            browser_profile: None,
            created_at: chrono::Utc::now(),
            last_active_at: chrono::Utc::now(),
            listed_at: chrono::Utc::now(),
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
    fn clients_for_task_skips_hyprland_when_disabled() {
        let config = TskConfig {
            hyprland_enabled: false,
            ..sample_config()
        };
        let clients = clients_for_task(&config, &sample_task()).unwrap();
        assert!(clients.is_empty());
    }

    #[test]
    fn owned_task_checkouts_includes_sibling() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("app");
        crate::vcs::init_scratch_repo(&source).unwrap();
        let source_str = source.to_str().unwrap();
        for args in [
            &["config", "user.email", "tsk@test"][..],
            &["config", "user.name", "tsk"][..],
            &["commit", "--allow-empty", "-m", "init"][..],
        ] {
            let mut cmd = std::process::Command::new("git");
            cmd.arg("-C").arg(source_str);
            cmd.args(args);
            cmd.status().unwrap();
        }
        let tasks_base = dir.path().join("tasks");
        let primary = tasks_base.join("tabc").join("workspace").join("app");
        let sibling = tasks_base.join("tabc").join("workspace").join("app-review");
        crate::vcs::create_linked_checkout(
            &source,
            &primary,
            "tabc",
            crate::vcs::VcsKind::Git,
            None,
        )
        .unwrap();
        crate::vcs::create_linked_checkout(
            &source,
            &sibling,
            "tabc-review",
            crate::vcs::VcsKind::Git,
            None,
        )
        .unwrap();
        let now = chrono::Utc::now();
        let task = Task {
            id: "tabc".into(),
            name: "Feature".into(),
            status: TaskStatus::Active,
            repo_url: None,
            repo_path: primary.clone(),
            source_repo_path: Some(source.clone()),
            branch: None,
            container_name: "tsk-tabc".into(),
            container_isolation: false,
            workspace_count: 10,
            browser_profile: None,
            created_at: now,
            last_active_at: now,
            listed_at: now,
            agent_notes_path: None,
            ports: vec![],
        };
        let config = TskConfig {
            tasks_base_dir: tasks_base,
            ..TskConfig::default()
        };
        let owned = owned_task_checkouts(&config, &task).unwrap();
        assert!(owned.iter().any(|(p, n)| p == &primary && n == "tabc"));
        assert!(owned
            .iter()
            .any(|(p, n)| p == &sibling && n == "tabc-review"));
        std::fs::write(sibling.join("keep.txt"), "sibling local").unwrap();
        detach_task_checkout(&config, &task).unwrap();
        assert!(!sibling.join(".git").exists());
        assert!(sibling.join("keep.txt").is_file());
        assert!(!primary.join(".git").exists());
        assert_eq!(git_worktree_listed(&source, &sibling), Some(true));
        let after_detach = owned_task_checkouts(&config, &task).unwrap();
        assert!(
            after_detach
                .iter()
                .any(|(p, n)| p == &sibling && n == "tabc-review"),
            "detached sibling must still be discoverable for restore"
        );
        reattach_task_checkout(&config, &task).unwrap();
        assert!(sibling.join(".git").exists());
        assert!(primary.join(".git").exists());
        assert_eq!(git_worktree_listed(&source, &sibling), Some(true));
        assert_eq!(
            std::fs::read_to_string(sibling.join("keep.txt")).unwrap(),
            "sibling local"
        );
    }

    #[test]
    fn owned_task_checkouts_uses_git_list_not_folder_name() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("app");
        crate::vcs::init_scratch_repo(&source).unwrap();
        let source_str = source.to_str().unwrap();
        for args in [
            &["config", "user.email", "tsk@test"][..],
            &["config", "user.name", "tsk"][..],
            &["commit", "--allow-empty", "-m", "init"][..],
        ] {
            let mut cmd = std::process::Command::new("git");
            cmd.arg("-C").arg(source_str);
            cmd.args(args);
            cmd.status().unwrap();
        }
        let tasks_base = dir.path().join("tasks");
        let primary = tasks_base.join("tabc").join("workspace").join("app");
        let custom = tasks_base
            .join("tabc")
            .join("workspace")
            .join("review-copy");
        crate::vcs::create_linked_checkout(
            &source,
            &primary,
            "tabc",
            crate::vcs::VcsKind::Git,
            None,
        )
        .unwrap();
        crate::vcs::create_linked_checkout(
            &source,
            &custom,
            "tabc-review",
            crate::vcs::VcsKind::Git,
            None,
        )
        .unwrap();
        let now = chrono::Utc::now();
        let task = Task {
            id: "tabc".into(),
            name: "Feature".into(),
            status: TaskStatus::Active,
            repo_url: None,
            repo_path: primary.clone(),
            source_repo_path: Some(source),
            branch: None,
            container_name: "tsk-tabc".into(),
            container_isolation: false,
            workspace_count: 10,
            browser_profile: None,
            created_at: now,
            last_active_at: now,
            listed_at: now,
            agent_notes_path: None,
            ports: vec![],
        };
        let config = TskConfig {
            tasks_base_dir: tasks_base,
            ..TskConfig::default()
        };
        let owned = owned_task_checkouts(&config, &task).unwrap();
        assert!(owned.iter().any(|(p, n)| p == &primary && n == "tabc"));
        assert!(owned
            .iter()
            .any(|(p, n)| p == &custom && n == "tabc-review"));
    }

    #[test]
    fn owned_task_checkouts_skips_unrelated_workspace_dir() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("app");
        crate::vcs::init_scratch_repo(&source).unwrap();
        let source_str = source.to_str().unwrap();
        for args in [
            &["config", "user.email", "tsk@test"][..],
            &["config", "user.name", "tsk"][..],
            &["commit", "--allow-empty", "-m", "init"][..],
        ] {
            let mut cmd = std::process::Command::new("git");
            cmd.arg("-C").arg(source_str);
            cmd.args(args);
            cmd.status().unwrap();
        }
        let tasks_base = dir.path().join("tasks");
        let primary = tasks_base.join("tabc").join("workspace").join("app");
        crate::vcs::create_linked_checkout(
            &source,
            &primary,
            "tabc",
            crate::vcs::VcsKind::Git,
            None,
        )
        .unwrap();
        let leftover = tasks_base.join("tabc").join("workspace").join("app-notes");
        std::fs::create_dir_all(&leftover).unwrap();
        std::fs::write(leftover.join("notes.txt"), "not a checkout").unwrap();
        let now = chrono::Utc::now();
        let task = Task {
            id: "tabc".into(),
            name: "Feature".into(),
            status: TaskStatus::Active,
            repo_url: None,
            repo_path: primary.clone(),
            source_repo_path: Some(source),
            branch: None,
            container_name: "tsk-tabc".into(),
            container_isolation: false,
            workspace_count: 10,
            browser_profile: None,
            created_at: now,
            last_active_at: now,
            listed_at: now,
            agent_notes_path: None,
            ports: vec![],
        };
        let config = TskConfig {
            tasks_base_dir: tasks_base,
            ..TskConfig::default()
        };
        let owned = owned_task_checkouts(&config, &task).unwrap();
        assert!(owned.iter().all(|(p, _)| p != &leftover));
        assert_eq!(owned.len(), 1);
        assert_eq!(owned[0].0, primary);
    }

    fn git_worktree_listed(source: &Path, checkout: &Path) -> Option<bool> {
        let out = std::process::Command::new("git")
            .args(["-C", source.to_str()?, "worktree", "list"])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&out.stdout);
        let canon = std::fs::canonicalize(checkout).unwrap_or_else(|_| checkout.to_path_buf());
        Some(text.lines().any(|line| {
            line.split_whitespace().next().is_some_and(|p| {
                std::path::Path::new(p) == checkout
                    || std::fs::canonicalize(p).ok().as_ref() == Some(&canon)
            })
        }))
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
