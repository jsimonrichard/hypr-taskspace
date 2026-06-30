//! Taskspace-scoped Hyprland workspace navigation.

use crate::hyprland;
use crate::models::{ContextMode, SessionState};
use crate::workspaces::{
    allowed_workspace_names, default_taskspace_workspace_names, task_workspace_names,
};

pub fn workspace_name_for_relative(state: &SessionState, relative: i32) -> Option<String> {
    relative_to_name(state, relative)
}

pub fn remember_workspace(state: &mut SessionState, relative: i32) {
    state
        .last_workspace
        .insert(state.taskspace_key(), relative);
}

pub fn workspace_next_relative(state: &SessionState) -> Option<i32> {
    let names = allowed_workspace_names(state);
    if names.is_empty() {
        return None;
    }
    match active_relative(state) {
        Some(current) => Some((current % names.len() as i32) + 1),
        None => Some(1),
    }
}

pub fn workspace_prev_relative(state: &SessionState) -> Option<i32> {
    let names = allowed_workspace_names(state);
    if names.is_empty() {
        return None;
    }
    match active_relative(state) {
        Some(current) if current > 1 => Some(current - 1),
        Some(_) => Some(names.len() as i32),
        None => Some(names.len() as i32),
    }
}

pub fn workspace_go(state: &mut SessionState, relative: i32) -> Option<String> {
    let name = workspace_name_for_relative(state, relative)?;
    hyprland::switch_workspace(&name);
    remember_workspace(state, relative);
    Some(name)
}

pub fn workspace_next(state: &mut SessionState) -> Option<String> {
    let relative = workspace_next_relative(state)?;
    workspace_go(state, relative)
}

pub fn workspace_prev(state: &mut SessionState) -> Option<String> {
    let relative = workspace_prev_relative(state)?;
    workspace_go(state, relative)
}

pub fn workspace_goto_name(state: &mut SessionState, name: &str) -> Option<String> {
    if state.context_mode != ContextMode::Global {
        let allowed = allowed_workspace_names(state);
        if !allowed.iter().any(|n| n == name) {
            return None;
        }
    }
    hyprland::switch_workspace(name);
    let allowed = allowed_workspace_names(state);
    if let Some(idx) = allowed.iter().position(|n| n == name) {
        remember_workspace(state, (idx + 1) as i32);
    }
    Some(name.to_string())
}

pub fn focus_last_workspace(state: &mut SessionState) -> Option<String> {
    let key = state.taskspace_key();
    let relative = *state.last_workspace.get(&key).unwrap_or(&1);
    if let Some(name) = relative_to_name(state, relative) {
        hyprland::switch_workspace(&name);
        remember_workspace(state, relative);
        return Some(name);
    }
    let names = allowed_workspace_names(state);
    let name = names.first()?.clone();
    hyprland::switch_workspace(&name);
    remember_workspace(state, 1);
    Some(name)
}

pub fn set_taskspace(state: &mut SessionState, mode: ContextMode, task_id: Option<&str>) -> Result<(), String> {
    match mode {
        ContextMode::Task => {
            let Some(task_id) = task_id else {
                return Err("task_id required for task taskspace".into());
            };
            if !state.tasks.contains_key(task_id) {
                return Err(format!("Unknown task: {task_id}"));
            }
            state.context_mode = ContextMode::Task;
            state.current_task_id = Some(task_id.to_string());
            state.previous_context = None;
            state.previous_task_id = None;
        }
        ContextMode::Default => {
            state.context_mode = ContextMode::Default;
            state.current_task_id = None;
            state.previous_context = None;
            state.previous_task_id = None;
        }
        ContextMode::Global => {
            state.previous_context = Some(state.context_mode);
            state.previous_task_id = state.current_task_id.clone();
            state.context_mode = ContextMode::Global;
        }
    }
    focus_last_workspace(state);
    Ok(())
}

pub fn toggle_global(state: &mut SessionState) {
    if state.context_mode == ContextMode::Global {
        restore_taskspace(state);
    } else {
        let _ = set_taskspace(state, ContextMode::Global, None);
    }
}

pub fn restore_taskspace(state: &mut SessionState) {
    let prev_mode = state.previous_context.unwrap_or(ContextMode::Default);
    let prev_task = state.previous_task_id.clone();
    state.previous_context = None;
    state.previous_task_id = None;
    if prev_mode == ContextMode::Task {
        if let Some(task_id) = prev_task {
            state.context_mode = ContextMode::Task;
            state.current_task_id = Some(task_id);
        } else {
            state.context_mode = ContextMode::Default;
            state.current_task_id = None;
        }
    } else {
        state.context_mode = ContextMode::Default;
        state.current_task_id = None;
    }
    focus_last_workspace(state);
}

pub fn setup_task_workspaces(task_id: &str, slot_count: u32) {
    hyprland::ensure_workspaces(&task_workspace_names(task_id, slot_count));
}

pub fn setup_task_workspaces_for_state(task_id: &str, state: &SessionState) {
    setup_task_workspaces(task_id, state.default_workspace_count);
}

pub fn setup_default_taskspace_workspaces(count: u32) {
    hyprland::ensure_workspaces(&default_taskspace_workspace_names(count));
}

fn relative_to_name(state: &SessionState, relative: i32) -> Option<String> {
    let names = allowed_workspace_names(state);
    let idx = (relative - 1) as usize;
    names.get(idx).cloned()
}

fn active_relative(state: &SessionState) -> Option<i32> {
    let active = hyprland::get_active_workspace().ok().flatten()?;
    if active.name.is_empty() {
        return None;
    }
    allowed_workspace_names(state)
        .iter()
        .position(|n| n == &active.name)
        .map(|i| (i + 1) as i32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::SessionState;

    #[test]
    fn set_taskspace_default_clears_global_overlay() {
        let mut state = SessionState {
            context_mode: ContextMode::Global,
            previous_context: Some(ContextMode::Task),
            previous_task_id: Some("test-task".into()),
            current_task_id: Some("test-task".into()),
            default_workspace_count: 10,
            ..Default::default()
        };
        set_taskspace(&mut state, ContextMode::Default, None).unwrap();
        assert_eq!(state.context_mode, ContextMode::Default);
        assert!(state.previous_context.is_none());
        assert!(state.previous_task_id.is_none());
    }
}
