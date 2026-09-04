use chrono::Utc;

use crate::models::SessionState;
use crate::workspaces::{
    allowed_workspace_names, is_default_taskspace_workspace_name, is_global_workspace_name,
    resolve_bar_workspace_name, task_for_workspace_name,
};

/// Align taskspace with a known workspace name (no Hyprland IPC).
pub fn sync_from_workspace_name(state: &mut SessionState, name: &str) -> bool {
    if name.is_empty() {
        return false;
    }

    let mut changed = false;

    if is_default_taskspace_workspace_name(name, state.default_workspace_count) {
        let visiting_global_from_task = state.context_mode == crate::models::ContextMode::Task
            && state.current_task_id.is_some()
            && is_global_workspace_name(name, state);
        if !visiting_global_from_task
            && (state.context_mode != crate::models::ContextMode::Default
                || state.current_task_id.is_some())
        {
            state.context_mode = crate::models::ContextMode::Default;
            state.current_task_id = None;
            changed = true;
        }
    } else if let Some(task) = task_for_workspace_name(state, name) {
        let task_id = task.id.clone();
        if state.context_mode != crate::models::ContextMode::Task
            || state.current_task_id.as_deref() != Some(task_id.as_str())
        {
            state.context_mode = crate::models::ContextMode::Task;
            state.current_task_id = Some(task_id);
            touch_active_task(state);
            changed = true;
        }
    }

    let allowed = allowed_workspace_names(state);
    let Some(resolved) = resolve_bar_workspace_name(name, state, &allowed) else {
        return changed;
    };

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

/// Bump overlay recency for the current task. No-op in the default taskspace.
pub(crate) fn touch_active_task(state: &mut SessionState) {
    let Some(id) = state.current_task_id.clone() else {
        return;
    };
    if let Some(task) = state.tasks.get_mut(&id) {
        task.last_active_at = Utc::now();
    }
}

/// Whether focusing `name` would change context mode or active task.
pub fn taskspace_would_change(state: &SessionState, name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let mode_before = state.context_mode;
    let task_before = state.current_task_id.clone();
    let mut probe = state.clone();
    sync_from_workspace_name(&mut probe, name);
    probe.context_mode != mode_before || probe.current_task_id != task_before
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

    fn sample_task(id: &str, last_active_at: chrono::DateTime<chrono::Utc>) -> Task {
        Task {
            id: id.into(),
            name: id.into(),
            status: TaskStatus::Active,
            repo_url: None,
            repo_path: PathBuf::from(format!("/tmp/{id}/repo")),
            source_repo_path: None,
            branch: None,
            container_name: format!("tsk-{id}"),
            container_isolation: false,
            workspace_count: 10,
            browser_profile: None,
            created_at: last_active_at,
            last_active_at,
            listed_at: last_active_at,
            agent_notes_path: None,
            ports: vec![],
        }
    }

    fn task_state() -> SessionState {
        let task = sample_task("auth-fix", chrono::Utc::now());
        SessionState {
            context_mode: ContextMode::Task,
            current_task_id: Some("auth-fix".into()),
            default_workspace_count: 10,
            tasks: HashMap::from([("auth-fix".into(), task)]),
            ..Default::default()
        }
    }

    #[test]
    fn task_workspace_name_stays_in_task_taskspace() {
        let mut state = task_state();
        sync_from_workspace_name(&mut state, "auth-fix-8");
        assert_eq!(state.context_mode, ContextMode::Task);
        assert_eq!(state.current_task_id.as_deref(), Some("auth-fix"));
        assert_eq!(state.last_workspace.get("task:auth-fix"), Some(&8));
    }

    #[test]
    fn global_slot_stays_in_task_taskspace() {
        let mut state = task_state();
        state.global_workspace_slots = vec![1];
        sync_from_workspace_name(&mut state, "1");
        assert_eq!(state.context_mode, ContextMode::Task);
        assert_eq!(state.current_task_id.as_deref(), Some("auth-fix"));
        assert_eq!(state.last_workspace.get("task:auth-fix"), Some(&1));
    }

    #[test]
    fn non_global_default_workspace_switches_to_default_taskspace() {
        let mut state = task_state();
        state.global_workspace_slots = vec![1];
        sync_from_workspace_name(&mut state, "3");
        assert_eq!(state.context_mode, ContextMode::Default);
        assert!(state.current_task_id.is_none());
        assert_eq!(state.last_workspace.get("default"), Some(&3));
    }

    #[test]
    fn global_then_local_slot_stays_in_task() {
        let mut state = task_state();
        state.global_workspace_slots = vec![1];
        sync_from_workspace_name(&mut state, "1");
        sync_from_workspace_name(&mut state, "auth-fix-2");
        assert_eq!(state.context_mode, ContextMode::Task);
        assert_eq!(state.current_task_id.as_deref(), Some("auth-fix"));
        assert_eq!(state.last_workspace.get("task:auth-fix"), Some(&2));
    }

    #[test]
    fn taskspace_would_change_detects_cross_task_navigation() {
        let state = task_state();
        assert!(taskspace_would_change(&state, "3"));
        assert!(!taskspace_would_change(&state, "auth-fix-8"));
    }

    #[test]
    fn taskspace_would_change_global_slot_stays_in_task() {
        let mut state = task_state();
        state.global_workspace_slots = vec![1];
        assert!(!taskspace_would_change(&state, "1"));
    }

    #[test]
    fn switching_taskspace_via_workspace_updates_last_active_at() {
        let older_at = chrono::Utc::now() - chrono::Duration::seconds(60);
        let newer_at = older_at + chrono::Duration::seconds(30);
        let mut state = task_state();
        state
            .tasks
            .insert("auth-fix".into(), sample_task("auth-fix", older_at));
        state
            .tasks
            .insert("billing".into(), sample_task("billing", newer_at));
        state.current_task_id = Some("billing".into());

        std::thread::sleep(std::time::Duration::from_millis(5));
        assert!(sync_from_workspace_name(&mut state, "auth-fix-3"));
        assert_eq!(state.current_task_id.as_deref(), Some("auth-fix"));
        let auth = state.tasks.get("auth-fix").unwrap();
        assert!(auth.last_active_at > older_at);
        assert!(auth.last_active_at > newer_at);
        assert_eq!(auth.listed_at, older_at);
        assert_eq!(state.tasks.get("billing").unwrap().last_active_at, newer_at);
    }

    #[test]
    fn same_taskspace_workspace_does_not_touch_last_active_at() {
        let mut state = task_state();
        let at = state.tasks.get("auth-fix").unwrap().last_active_at;
        std::thread::sleep(std::time::Duration::from_millis(5));
        sync_from_workspace_name(&mut state, "auth-fix-8");
        assert_eq!(state.tasks.get("auth-fix").unwrap().last_active_at, at);
    }
}
