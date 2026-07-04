use std::collections::HashSet;

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

pub fn is_global_workspace_slot(slot: u32, global_slots: &[u32]) -> bool {
    global_slots.contains(&slot)
}

pub fn is_global_workspace_name(name: &str, state: &SessionState) -> bool {
    name.parse::<u32>()
        .ok()
        .is_some_and(|slot| is_global_workspace_slot(slot, &state.global_workspace_slots))
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

/// Task taskspace slots — same count as default (`SUPER+1..0` keybinds).
pub fn task_taskspace_workspace_names(state: &SessionState, task_id: &str) -> Vec<String> {
    (1..=state.default_workspace_count)
        .map(|slot| {
            if is_global_workspace_slot(slot, &state.global_workspace_slots) {
                default_taskspace_workspace_name(slot)
            } else {
                task_workspace_name(task_id, slot)
            }
        })
        .collect()
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
        ContextMode::Task => state
            .current_task_id
            .as_ref()
            .map(|id| task_taskspace_workspace_names(state, id))
            .unwrap_or_else(|| default_taskspace_workspace_names(state.default_workspace_count)),
        ContextMode::Default => {
            default_taskspace_workspace_names(state.default_workspace_count)
        }
    }
}

/// Workspace names shown on the Waybar strip.
pub fn bar_workspace_names(state: &SessionState) -> Vec<String> {
    allowed_workspace_names(state)
}

/// Map a Hyprland workspace name to the bar button key for the current taskspace.
pub fn resolve_bar_workspace_name(
    hypr_name: &str,
    state: &SessionState,
    bar_names: &[String],
) -> Option<String> {
    if bar_names.iter().any(|n| n == hypr_name) {
        return Some(hypr_name.to_string());
    }
    relative_slot_from_name(hypr_name).and_then(|rel| {
        if hypr_name
            .parse::<u32>()
            .ok()
            .is_some_and(|_| is_default_taskspace_workspace_name(hypr_name, state.default_workspace_count))
            && !bar_names.iter().any(|n| n == hypr_name)
        {
            return None;
        }
        if is_global_workspace_slot(rel, &state.global_workspace_slots) {
            let global_name = default_taskspace_workspace_name(rel);
            if bar_names.iter().any(|n| n == &global_name) {
                return Some(global_name);
            }
        }
        bar_names.get(rel.saturating_sub(1) as usize).cloned()
    })
}

/// Map a Hyprland workspace name to the bar button key for the current taskspace.
pub fn bar_active_workspace_name(
    active_hypr_name: &str,
    state: &SessionState,
    bar_names: &[String],
) -> String {
    resolve_bar_workspace_name(active_hypr_name, state, bar_names).unwrap_or_else(|| {
        bar_names
            .first()
            .cloned()
            .unwrap_or_else(|| "1".into())
    })
}

/// Occupied bar slots for the current taskspace strip.
pub fn bar_occupied_names(_state: &SessionState, bar_names: &[String]) -> HashSet<String> {
    let bar_set: HashSet<String> = bar_names.iter().cloned().collect();
    let mut occupied = HashSet::new();
    if !crate::hyprland::available() {
        return occupied;
    }
    if let Ok(clients) = crate::hyprland::get_clients() {
        for client in clients {
            let name = &client.workspace_name;
            if bar_set.contains(name) {
                occupied.insert(name.clone());
            }
        }
    }
    occupied
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
    fn bar_active_maps_task_workspace_to_slot() {
        let state = SessionState::default();
        let bar = default_taskspace_workspace_names(10);
        assert_eq!(bar_active_workspace_name("t-4", &state, &bar), "4");
        assert_eq!(bar_active_workspace_name("4", &state, &bar), "4");
    }

    #[test]
    fn task_mode_matches_default_slot_count() {
        use crate::models::{ContextMode, Task, TaskStatus};

        let mut state = SessionState {
            context_mode: ContextMode::Task,
            current_task_id: Some("auth-fix".into()),
            default_workspace_count: 10,
            ..Default::default()
        };
        state.tasks.insert(
            "auth-fix".into(),
            Task {
                id: "auth-fix".into(),
                name: "Auth Fix".into(),
                status: TaskStatus::Active,
                repo_url: None,
                repo_path: "/tmp".into(),
                source_repo_path: None,
                branch: None,
                container_name: "tsk-auth-fix".into(),
                workspace_count: 3,
                browser_profile: None,
                created_at: chrono::Utc::now(),
                last_active_at: chrono::Utc::now(),
                agent_notes_path: None,
                ports: vec![],
            },
        );
        let names = allowed_workspace_names(&state);
        assert_eq!(names.len(), 10);
        assert_eq!(names[0], "auth-fix-1");
        assert_eq!(names[9], "auth-fix-10");
    }

    #[test]
    fn global_slot_uses_default_workspace_name_in_task_mode() {
        use crate::models::{ContextMode, Task, TaskStatus};

        let mut state = SessionState {
            context_mode: ContextMode::Task,
            current_task_id: Some("auth-fix".into()),
            default_workspace_count: 10,
            global_workspace_slots: vec![1, 10],
            ..Default::default()
        };
        state.tasks.insert(
            "auth-fix".into(),
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
            },
        );
        let names = allowed_workspace_names(&state);
        assert_eq!(names[0], "1");
        assert_eq!(names[1], "auth-fix-2");
        assert_eq!(names[9], "10");
        assert!(is_global_workspace_name("1", &state));
        assert!(!is_global_workspace_name("auth-fix-2", &state));
    }

    #[test]
    fn resolve_bar_workspace_name_prefers_global_slot() {
        let mut state = SessionState {
            context_mode: ContextMode::Task,
            current_task_id: Some("auth-fix".into()),
            default_workspace_count: 10,
            global_workspace_slots: vec![1],
            ..Default::default()
        };
        state.tasks.insert(
            "auth-fix".into(),
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
            },
        );
        let bar = allowed_workspace_names(&state);
        assert_eq!(
            resolve_bar_workspace_name("1", &state, &bar),
            Some("1".into())
        );
        assert_eq!(
            resolve_bar_workspace_name("auth-fix-8", &state, &bar),
            Some("auth-fix-8".into())
        );
        assert!(resolve_bar_workspace_name("8", &state, &bar).is_none());
    }

    #[test]
    fn resolve_bar_does_not_map_foreign_default_number_to_task_slot() {
        let mut state = SessionState {
            context_mode: ContextMode::Task,
            current_task_id: Some("auth-fix".into()),
            default_workspace_count: 10,
            global_workspace_slots: vec![1],
            ..Default::default()
        };
        state.tasks.insert(
            "auth-fix".into(),
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
            },
        );
        let bar = allowed_workspace_names(&state);
        assert!(resolve_bar_workspace_name("2", &state, &bar).is_none());
    }
}
