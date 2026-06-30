use crate::models::SessionState;
use crate::workspaces::{
    allowed_workspace_names, is_default_taskspace_workspace_name, task_for_workspace_name,
};

/// Align taskspace with a known workspace name (no Hyprland IPC).
pub fn sync_from_workspace_name(state: &mut SessionState, name: &str) -> bool {
    if name.is_empty() {
        return false;
    }

    let mut changed = false;

    if is_default_taskspace_workspace_name(name, state.default_workspace_count) {
        if state.context_mode != crate::models::ContextMode::Default
            || state.current_task_id.is_some()
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
            changed = true;
        }
    }

    let names = allowed_workspace_names(state);
    if let Some(idx) = names.iter().position(|n| n == name) {
        let rel = (idx + 1) as i32;
        let key = state.taskspace_key();
        if state.last_workspace.get(&key).copied() != Some(rel) {
            state.last_workspace.insert(key, rel);
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
