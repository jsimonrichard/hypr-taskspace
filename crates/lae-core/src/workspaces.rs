use crate::models::{ContextMode, SessionState, Task, TaskStatus};

pub const DEFAULT_MIN_VISIBLE_WORKSPACES: u32 = 10;

pub fn default_taskspace_workspace_name(relative: u32) -> String {
    relative.to_string()
}

pub fn default_taskspace_workspace_names(count: u32) -> Vec<String> {
    (1..=count)
        .map(default_taskspace_workspace_name)
        .collect()
}

pub fn is_default_taskspace_workspace_name(name: &str, workspace_count: u32) -> bool {
    name.parse::<u32>()
        .ok()
        .is_some_and(|n| (1..=workspace_count).contains(&n))
}

pub fn task_workspace_name(task_id: &str, relative: u32) -> String {
    format!("{task_id}-{relative}")
}

pub fn task_workspace_names(task_id: &str, count: u32) -> Vec<String> {
    (1..=count).map(|n| task_workspace_name(task_id, n)).collect()
}

/// Short label shown on the bar (1–9, 0 for slot 10) from a full Hyprland workspace name.
pub fn workspace_display_label(name: &str) -> String {
    let rel = relative_slot_from_name(name).unwrap_or(0);
    if rel == 10 {
        "0".into()
    } else if rel > 0 {
        rel.to_string()
    } else {
        name.to_string()
    }
}

/// 1-based slot index within a taskspace (`"3"` → 3, `"auth-fix-2"` → 2).
pub fn relative_slot_from_name(name: &str) -> Option<u32> {
    if let Ok(n) = name.parse::<u32>() {
        return Some(n);
    }
    name.rsplit_once('-')
        .and_then(|(_, rel)| rel.parse().ok())
}

pub fn task_for_workspace_name<'a>(state: &'a SessionState, name: &str) -> Option<&'a Task> {
    if is_default_taskspace_workspace_name(name, state.default_workspace_count) {
        return None;
    }
    state.tasks.values().find(|task| {
        task.status != TaskStatus::Archived && name.starts_with(&format!("{}-", task.id))
    })
}

pub fn allowed_workspace_names(state: &SessionState) -> Vec<String> {
    match state.context_mode {
        ContextMode::Global => {
            let mut names = default_taskspace_workspace_names(state.default_workspace_count);
            for task in state.tasks.values() {
                if task.status != TaskStatus::Archived {
                    names.extend(task.workspace_names());
                }
            }
            names
        }
        ContextMode::Task => state
            .current_task_id
            .as_ref()
            .and_then(|id| state.tasks.get(id))
            .map(|task| task.workspace_names())
            .unwrap_or_else(|| default_taskspace_workspace_names(state.default_workspace_count)),
        ContextMode::Default => {
            default_taskspace_workspace_names(state.default_workspace_count)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_display_label_from_names() {
        assert_eq!(workspace_display_label("3"), "3");
        assert_eq!(workspace_display_label("10"), "0");
        assert_eq!(workspace_display_label("auth-fix-2"), "2");
    }

    #[test]
    fn relative_slot_from_name_parses() {
        assert_eq!(relative_slot_from_name("5"), Some(5));
        assert_eq!(relative_slot_from_name("my-task-3"), Some(3));
    }
}
