//! Window home-workspace tracking and bulk restore.

use std::collections::HashSet;

use crate::error::Result;
use crate::hyprland::{self, HyprWindow};
use crate::models::{SessionState, TaskStatus, WindowRecord};
use crate::terminal::{TUI_WINDOW_CLASS, TUI_WINDOW_TITLE};
use crate::workspaces::task_for_workspace_name;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RestoreReport {
    pub synced: usize,
    pub restored: usize,
    pub already_home: usize,
    pub skipped: usize,
    pub moves: Vec<RestoreMove>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreMove {
    pub address: String,
    pub title: String,
    pub from: String,
    pub to: String,
}

pub fn client_workspace_name(client: &HyprWindow) -> String {
    if !client.workspace_name.is_empty() {
        client.workspace_name.clone()
    } else {
        client.workspace.to_string()
    }
}

pub fn infer_task_id(state: &SessionState, workspace_name: &str, title: &str) -> Option<String> {
    if let Some(task) = task_for_workspace_name(state, workspace_name) {
        return Some(task.id.clone());
    }
    for task in state.tasks.values() {
        if task.status == TaskStatus::Archived {
            continue;
        }
        let prefix = format!("[{}]", task.id);
        if title.starts_with(&prefix) {
            return Some(task.id.clone());
        }
    }
    None
}

fn is_ephemeral_window(client: &HyprWindow) -> bool {
    client.title == TUI_WINDOW_TITLE || client.class_name == TUI_WINDOW_CLASS
}

/// Refresh window records from Hyprland. New windows get `home_workspace_name` from their
/// current workspace; existing records keep their home.
pub fn sync_window_registry(state: &mut SessionState) -> Result<usize> {
    if !hyprland::available() {
        return Ok(0);
    }
    let clients = hyprland::get_clients()?;
    let live: HashSet<String> = clients.iter().map(|c| c.address.clone()).collect();
    state.windows.retain(|addr, _| live.contains(addr));

    for client in &clients {
        if is_ephemeral_window(client) {
            continue;
        }
        let workspace_name = client_workspace_name(client);
        let task_id = infer_task_id(state, &workspace_name, &client.title);
        let entry = state
            .windows
            .entry(client.address.clone())
            .or_insert_with(|| WindowRecord {
                hypr_address: client.address.clone(),
                home_workspace_name: workspace_name.clone(),
                task_id: task_id.clone(),
                ..Default::default()
            });
        if entry.home_workspace_name.is_empty() {
            entry.home_workspace_name = workspace_name.clone();
        }
        entry.task_id = entry.task_id.clone().or(task_id);
        entry.title = client.title.clone();
        entry.class_name = client.class_name.clone();
        entry.workspace = client.workspace;
        entry.workspace_name = workspace_name;
        entry.pid = client.pid;
    }

    Ok(clients
        .iter()
        .filter(|c| !is_ephemeral_window(c))
        .count())
}

pub fn misplaced_clients(state: &SessionState, clients: &[HyprWindow]) -> Vec<(HyprWindow, String)> {
    let mut out = Vec::new();
    for client in clients {
        if is_ephemeral_window(client) {
            continue;
        }
        let Some(record) = state.windows.get(&client.address) else {
            continue;
        };
        if record.home_workspace_name.is_empty() {
            continue;
        }
        let current = client_workspace_name(client);
        if current == record.home_workspace_name {
            continue;
        }
        out.push((client.clone(), record.home_workspace_name.clone()));
    }
    out
}

pub fn restore_windows(state: &mut SessionState, dry_run: bool) -> Result<RestoreReport> {
    let synced = sync_window_registry(state)?;
    if !hyprland::available() {
        return Ok(RestoreReport {
            synced,
            ..Default::default()
        });
    }

    let clients = hyprland::get_clients()?;
    let mut report = RestoreReport {
        synced,
        ..Default::default()
    };

    for client in &clients {
        if is_ephemeral_window(client) {
            report.skipped += 1;
            continue;
        }
        let Some(record) = state.windows.get(&client.address) else {
            report.skipped += 1;
            continue;
        };
        if record.home_workspace_name.is_empty() {
            report.skipped += 1;
            continue;
        }
        let current = client_workspace_name(client);
        if current == record.home_workspace_name {
            report.already_home += 1;
            continue;
        }

        report.moves.push(RestoreMove {
            address: client.address.clone(),
            title: client.title.clone(),
            from: current,
            to: record.home_workspace_name.clone(),
        });
    }

    if dry_run {
        report.restored = report.moves.len();
        return Ok(report);
    }

    for mv in &report.moves {
        hyprland::move_window_to_workspace_silent(&mv.address, &mv.to);
        if let Some(record) = state.windows.get_mut(&mv.address) {
            record.workspace_name = mv.to.clone();
        }
    }
    report.restored = report.moves.len();
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ContextMode, Task, TaskStatus};
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn task_state() -> SessionState {
        let task = Task {
            id: "auth-fix".into(),
            name: "Auth Fix".into(),
            status: TaskStatus::Active,
            repo_url: None,
            repo_path: PathBuf::from("/tmp/auth-fix"),
            source_repo_path: None,
            branch: None,
            container_name: "tsk-auth-fix".into(),
            workspace_count: 10,
            browser_profile: None,
            created_at: chrono::Utc::now(),
            last_active_at: chrono::Utc::now(),
            agent_notes_path: None,
            ports: vec![],
        };
        SessionState {
            context_mode: ContextMode::Task,
            current_task_id: Some("auth-fix".into()),
            default_workspace_count: 10,
            tasks: HashMap::from([("auth-fix".into(), task)]),
            ..Default::default()
        }
    }

    #[test]
    fn infer_task_id_from_workspace_name() {
        let state = task_state();
        assert_eq!(
            infer_task_id(&state, "auth-fix-3", "Terminal"),
            Some("auth-fix".into())
        );
        assert_eq!(infer_task_id(&state, "3", "Terminal"), None);
    }

    #[test]
    fn infer_task_id_from_title_prefix() {
        let state = task_state();
        assert_eq!(
            infer_task_id(&state, "99", "[auth-fix] nvim"),
            Some("auth-fix".into())
        );
    }

    #[test]
    fn misplaced_clients_detects_drift() {
        let mut state = task_state();
        state.windows.insert(
            "0xabc".into(),
            WindowRecord {
                hypr_address: "0xabc".into(),
                home_workspace_name: "auth-fix-2".into(),
                ..Default::default()
            },
        );
        let clients = vec![HyprWindow {
            address: "0xabc".into(),
            title: "term".into(),
            class_name: "Alacritty".into(),
            workspace: 0,
            workspace_name: "billing-4".into(),
            pid: Some(1),
        }];
        let misplaced = misplaced_clients(&state, &clients);
        assert_eq!(misplaced.len(), 1);
        assert_eq!(misplaced[0].1, "auth-fix-2");
    }
}
