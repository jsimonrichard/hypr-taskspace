use crate::models::{SessionState, Task, TaskStatus};
use crate::workspaces::{is_global_workspace_name, workspace_display_label};

/// Shortest prefix of `id` that uniquely identifies it among `ids` (jj-style).
pub fn unique_id_prefix(id: &str, ids: &[&str]) -> String {
    let id_lower = id.to_ascii_lowercase();
    let others: Vec<String> = ids
        .iter()
        .filter(|&&other| !other.eq_ignore_ascii_case(id))
        .map(|s| s.to_ascii_lowercase())
        .collect();
    if others.is_empty() {
        return id.to_string();
    }
    for len in 1..=id.len() {
        let prefix = &id_lower[..len];
        if others.iter().all(|other| !other.starts_with(prefix)) {
            return id[..len].to_string();
        }
    }
    id.to_string()
}

pub fn active_task_ids(state: &SessionState) -> Vec<String> {
    state
        .tasks
        .values()
        .filter(|t| t.status != TaskStatus::Archived)
        .map(|t| t.id.clone())
        .collect()
}

/// Short display id for a task — unique among active tasks with minimum prefix length.
pub fn short_task_id(state: &SessionState, task_id: &str) -> String {
    let active = active_task_ids(state);
    let ids: Vec<&str> = active.iter().map(String::as_str).collect();
    unique_id_prefix(task_id, &ids)
}

/// Slot numbers for the workspace strip tooltip (no repeated task id prefix).
pub fn format_workspaces_tooltip(workspaces: &[String], state: &SessionState) -> String {
    workspaces
        .iter()
        .map(|name| workspace_slot_label(name, Some(state)))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Hover text for a single workspace button.
pub fn workspace_tooltip_label(state: Option<&SessionState>, workspace_name: &str) -> String {
    let slot = workspace_slot_label(workspace_name, state);
    format!("Workspace {slot}")
}

fn workspace_slot_label(name: &str, state: Option<&SessionState>) -> String {
    let slot = workspace_display_label(name);
    if state.is_some_and(|s| is_global_workspace_name(name, s)) {
        format!("{slot} (global)")
    } else {
        slot
    }
}

#[derive(Debug, Clone)]
pub enum TaskLookup<'a> {
    Found(&'a Task),
    NotFound,
    AmbiguousPrefix(Vec<String>),
}

pub fn lookup_task<'a>(state: &'a SessionState, name_or_id: &str) -> TaskLookup<'a> {
    if let Some(task) = state.tasks.get(name_or_id) {
        return TaskLookup::Found(task);
    }

    let lower = name_or_id.to_ascii_lowercase();
    if let Some(task) = state
        .tasks
        .values()
        .find(|t| t.name.eq_ignore_ascii_case(name_or_id))
    {
        return TaskLookup::Found(task);
    }

    let matches: Vec<&Task> = state
        .tasks
        .values()
        .filter(|t| t.status != TaskStatus::Archived)
        .filter(|t| t.id.to_ascii_lowercase().starts_with(&lower))
        .collect();

    match matches.len() {
        0 => TaskLookup::NotFound,
        1 => TaskLookup::Found(matches[0]),
        _ => TaskLookup::AmbiguousPrefix(matches.iter().map(|t| t.id.clone()).collect()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ContextMode, Task, TaskStatus};
    use chrono::Utc;
    use std::path::PathBuf;

    fn sample_task(id: &str) -> Task {
        Task {
            id: id.into(),
            name: format!("Task {id}"),
            status: TaskStatus::Active,
            repo_url: None,
            repo_path: PathBuf::from("/tmp"),
            source_repo_path: None,
            branch: None,
            container_name: format!("tsk-{id}"),
            container_isolation: false,
            workspace_count: 3,
            browser_profile: None,
            created_at: Utc::now(),
            last_active_at: Utc::now(),
            agent_notes_path: None,
            ports: vec![],
        }
    }

    #[test]
    fn unique_id_prefix_picks_shortest_disambiguator() {
        let ids = ["tabc123", "tabc456", "txyz789"];
        assert_eq!(unique_id_prefix("tabc123", &ids), "tabc1");
        assert_eq!(unique_id_prefix("tabc456", &ids), "tabc4");
        assert_eq!(unique_id_prefix("txyz789", &ids), "tx");
    }

    #[test]
    fn unique_id_prefix_returns_full_id_when_alone() {
        assert_eq!(unique_id_prefix("tdeadbeef", &["tdeadbeef"]), "tdeadbeef");
    }

    #[test]
    fn lookup_task_by_prefix() {
        let mut state = SessionState::default();
        state.tasks.insert("tabc123".into(), sample_task("tabc123"));
        state.tasks.insert("tabc456".into(), sample_task("tabc456"));

        assert!(matches!(
            lookup_task(&state, "tabc1"),
            TaskLookup::Found(t) if t.id == "tabc123"
        ));
        assert!(matches!(
            lookup_task(&state, "tabc4"),
            TaskLookup::Found(t) if t.id == "tabc456"
        ));
        assert!(matches!(lookup_task(&state, "tabc"), TaskLookup::AmbiguousPrefix(_)));
        assert!(matches!(lookup_task(&state, "missing"), TaskLookup::NotFound));
    }

    #[test]
    fn format_workspaces_tooltip_uses_slots_only() {
        let mut state = SessionState {
            context_mode: ContextMode::Task,
            current_task_id: Some("auth-fix".into()),
            default_workspace_count: 10,
            global_workspace_slots: vec![1, 10],
            ..Default::default()
        };
        state.tasks.insert("auth-fix".into(), sample_task("auth-fix"));
        let workspaces = crate::workspaces::allowed_workspace_names(&state);
        let tooltip = format_workspaces_tooltip(&workspaces, &state);
        assert_eq!(tooltip, "1 (global), 2, 3, 4, 5, 6, 7, 8, 9, 0 (global)");
    }
}
