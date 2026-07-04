use crate::models::SessionState;
use crate::workspaces::{
    allowed_workspace_names, is_default_taskspace_workspace_name,
    resolve_bar_workspace_name, task_for_workspace_name,
};

/// Align taskspace with a known workspace name (no Hyprland IPC).
pub fn sync_from_workspace_name(state: &mut SessionState, name: &str) -> bool {
    if name.is_empty() {
        return false;
    }

    let allowed = allowed_workspace_names(state);
    let Some(resolved) = resolve_bar_workspace_name(name, state, &allowed) else {
        return false;
    };

    let mut changed = false;

    if is_default_taskspace_workspace_name(&resolved, state.default_workspace_count) {
        if state.context_mode != crate::models::ContextMode::Default
            || state.current_task_id.is_some()
        {
            state.context_mode = crate::models::ContextMode::Default;
            state.current_task_id = None;
            changed = true;
        }
    } else if let Some(task) = task_for_workspace_name(state, &resolved) {
        let task_id = task.id.clone();
        if state.context_mode != crate::models::ContextMode::Task
            || state.current_task_id.as_deref() != Some(task_id.as_str())
        {
            state.context_mode = crate::models::ContextMode::Task;
            state.current_task_id = Some(task_id);
            changed = true;
        }
    }

    if let Some(idx) = allowed.iter().position(|n| n == &resolved) {
        let rel = (idx + 1) as i32;
        let key = state.taskspace_key();
        if state.last_workspace.get(&key).copied() != Some(rel) {
            state.last_workspace.insert(key.clone(), rel);
            changed = true;
        }
        let before = state
            .last_monitor_workspace
            .get(&key)
            .cloned()
            .unwrap_or_default();
        crate::workspace_nav::refresh_monitor_slots(state);
        let after = state
            .last_monitor_workspace
            .get(&key)
            .cloned()
            .unwrap_or_default();
        if before != after {
            changed = true;
        }
    }

    changed
}

pub fn sync_from_active_workspace(state: &mut SessionState) -> bool {
    if !crate::hyprland::available() {
        return false;
    }

    let Ok(Some(active)) = crate::hyprland::get_active_workspace() else {
        return false;
    };
    if active.name.is_empty() {
        return false;
    }

    sync_from_workspace_name(state, &active.name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ContextMode, SessionState, Task, TaskStatus};
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn task_state() -> SessionState {
        let task = Task {
            id: "auth-fix".into(),
            name: "Auth Fix".into(),
            status: TaskStatus::Active,
            repo_url: None,
            repo_path: PathBuf::from("/tmp/auth-fix/repo"),
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
    fn numeric_hypr_name_stays_in_task_taskspace() {
        let mut state = task_state();
        sync_from_workspace_name(&mut state, "8");
        assert_eq!(state.context_mode, ContextMode::Task);
        assert_eq!(state.current_task_id.as_deref(), Some("auth-fix"));
        assert_eq!(state.last_workspace.get("task:auth-fix"), Some(&8));
    }

    #[test]
    fn global_slot_switches_to_default_taskspace() {
        let mut state = task_state();
        state.global_workspace_slots = vec![1];
        sync_from_workspace_name(&mut state, "1");
        assert_eq!(state.context_mode, ContextMode::Default);
        assert!(state.current_task_id.is_none());
        assert_eq!(state.last_workspace.get("default"), Some(&1));
    }
}
