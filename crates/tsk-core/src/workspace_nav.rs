//! Taskspace-scoped Hyprland workspace navigation.

use std::collections::HashMap;

use crate::hypr_log;
use crate::hyprland::{self, Monitor};
use crate::models::{ContextMode, SessionState};
use crate::workspaces::{
    allowed_workspace_names, default_taskspace_workspace_names, task_workspace_names,
};

pub fn workspace_name_for_relative(state: &SessionState, relative: i32) -> Option<String> {
    relative_to_name(state, relative)
}

pub fn remember_workspace(state: &mut SessionState, relative: i32) {
    let key = state.taskspace_key();
    state.last_workspace.insert(key, relative);
    refresh_monitor_slots(state);
}

/// Update taskspace + slot memory after navigating to a resolved workspace name.
pub fn sync_workspace_slot(state: &mut SessionState, relative: i32) -> Option<String> {
    let name = workspace_name_for_relative(state, relative)?;
    crate::context_sync::sync_from_workspace_name(state, &name);
    Some(name)
}

/// Clear remembered workspace / per-monitor layout (for install or manual reset).
pub fn clear_navigation_memory(state: &mut SessionState) {
    state.last_monitor_workspace.clear();
    state.last_workspace.clear();
    state
        .last_workspace
        .insert(ContextMode::Default.as_str().into(), 1);
}

/// Remove cached slot-* files under `$XDG_RUNTIME_DIR/tsk/`.
pub fn clear_runtime_slot_cache() {
    if let Ok(dir) = crate::xdg::tsk_runtime_dir() {
        for slot in 1..=10 {
            let _ = std::fs::remove_file(dir.join(format!("slot-{slot}")));
        }
    }
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
    hypr_log::scoped(format!("workspace_go slot {relative} → {name}"), || {
        hyprland::switch_workspace_for_navigation(&name);
        crate::context_sync::sync_from_workspace_name(state, &name);
    });
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
    let allowed = allowed_workspace_names(state);
    if !allowed.iter().any(|n| n == name) {
        return None;
    }
    hypr_log::scoped(format!("workspace_goto_name {name}"), || {
        hyprland::switch_workspace_for_navigation(name);
        crate::context_sync::sync_from_workspace_name(state, name);
    });
    Some(name.to_string())
}

pub fn move_window_to_relative(state: &SessionState, relative: i32) -> Option<String> {
    let name = workspace_name_for_relative(state, relative)?;
    hypr_log::scoped(format!("move_window_to_relative slot {relative} → {name}"), || {
        hyprland::move_active_window_to_workspace(&name);
    });
    Some(name)
}

pub fn focus_last_workspace(state: &mut SessionState) -> Option<String> {
    let key = state.taskspace_key();
    let relative = *state.last_workspace.get(&key).unwrap_or(&1);
    let result = focus_relative_in_taskspace(state, relative);
    if result.is_some() {
        remember_workspace(state, relative);
    }
    result
}

/// Refresh per-monitor slot memory from the current Hyprland layout.
pub fn refresh_monitor_slots(state: &mut SessionState) {
    if !hyprland::available() {
        return;
    }
    let key = state.taskspace_key();
    let allowed = allowed_workspace_names(state);
    if allowed.is_empty() {
        return;
    }
    if let Ok(monitors) = hyprland::list_monitors() {
        capture_monitor_slots(state, &key, &allowed, &monitors);
    }
}

/// When switching taskspaces, map each monitor's slot onto the new taskspace.
pub fn sync_monitors_to_taskspace(
    old_allowed: &[String],
    state: &mut SessionState,
) -> Option<String> {
    hypr_log::scoped(
        format!("sync_monitors_to_taskspace → {}", state.taskspace_key()),
        || sync_monitors_to_taskspace_inner(old_allowed, state),
    )
}

fn sync_monitors_to_taskspace_inner(
    old_allowed: &[String],
    state: &mut SessionState,
) -> Option<String> {
    let new_allowed = allowed_workspace_names(state);
    if new_allowed.is_empty() {
        return None;
    }

    if !hyprland::available() {
        let relative = state
            .last_workspace
            .get(&state.taskspace_key())
            .copied()
            .unwrap_or(1);
        return focus_relative_in_taskspace(state, relative);
    }

    let monitors = sort_monitors_by_layout(list_monitors_with_retry());
    if monitors.len() <= 1 {
        let relative = state
            .last_workspace
            .get(&state.taskspace_key())
            .copied()
            .unwrap_or(1);
        return focus_relative_in_taskspace(state, relative);
    }

    let dest_key = state.taskspace_key();
    let focused_monitor = monitors
        .iter()
        .find(|m| m.focused)
        .map(|m| m.name.clone());
    let max_slots = new_allowed.len();

    let mut targets: Vec<(String, i32, String)> = Vec::new();
    for (index, monitor) in monitors.iter().enumerate() {
        let relative = resolve_monitor_slot(state, &dest_key, &monitor.name, index, max_slots);
        let target = workspace_name_at_relative(&new_allowed, relative)?;
        targets.push((monitor.name.clone(), relative, target));
    }

    let plan = targets
        .iter()
        .map(|(monitor, slot, ws)| format!("{monitor}:slot{slot}={ws}"))
        .collect::<Vec<_>>()
        .join(", ");
    hypr_log::note(format!("restore plan: {plan}"));

    hypr_log::scoped("restore_monitor_targets", || {
        restore_monitor_targets(&targets, &monitors, &new_allowed, old_allowed);
    });
    hypr_log::scoped("verify_monitor_targets", || {
        verify_monitor_targets(&targets, &new_allowed, old_allowed);
    });
    persist_monitor_layout_from_hyprland(state, &new_allowed, focused_monitor.as_deref());

    if let Some(name) = focused_monitor.as_deref() {
        hypr_log::scoped(format!("refocus monitor {name}"), || {
            hyprland::focus_monitor(name);
        });
    }

    focused_monitor
        .as_deref()
        .and_then(|name| {
            targets
                .iter()
                .find(|(monitor, _, _)| monitor == name)
                .map(|(_, _, ws)| ws.clone())
        })
        .or_else(|| {
            workspace_name_at_relative(
                &new_allowed,
                state
                    .last_workspace
                    .get(&dest_key)
                    .copied()
                    .unwrap_or(1),
            )
        })
}

pub fn set_taskspace(
    state: &mut SessionState,
    mode: ContextMode,
    task_id: Option<&str>,
) -> Result<(), String> {
    let old_allowed = allowed_workspace_names(state);
    let old_key = state.taskspace_key();
    let dest_label = match mode {
        ContextMode::Task => task_id
            .map(|id| format!("task:{id}"))
            .unwrap_or_else(|| "task:?".into()),
        ContextMode::Default => "default".into(),
    };

    hypr_log::scoped(format!("set_taskspace {old_key} → {dest_label}"), || {
        set_taskspace_inner(state, mode, task_id, &old_allowed, &old_key)
    })
}

fn set_taskspace_inner(
    state: &mut SessionState,
    mode: ContextMode,
    task_id: Option<&str>,
    old_allowed: &[String],
    old_key: &str,
) -> Result<(), String> {
    if hyprland::available() {
        hypr_log::scoped(format!("capture layout for leaving taskspace {old_key}"), || {
            if let Ok(monitors) = hyprland::list_monitors() {
                capture_monitor_layout(state, old_key, old_allowed, &monitors);
            }
        });
        hyprland::close_tsk_tui_windows();
    }

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
            if hyprland::available() {
                setup_task_workspaces_for_state(task_id, state);
            }
        }
        ContextMode::Default => {
            state.context_mode = ContextMode::Default;
            state.current_task_id = None;
            if hyprland::available() {
                setup_default_taskspace_workspaces(state.default_workspace_count);
            }
        }
    }

    if sync_monitors_to_taskspace(old_allowed, state).is_none() {
        hypr_log::scoped("fallback focus_last_workspace", || {
            focus_last_workspace(state);
        });
    }
    Ok(())
}

pub fn setup_task_workspaces(task_id: &str, slot_count: u32) {
    hypr_log::scoped(format!("setup_task_workspaces {task_id}"), || {
        hyprland::ensure_workspaces(&task_workspace_names(task_id, slot_count));
    });
}

pub fn setup_task_workspaces_for_state(task_id: &str, state: &SessionState) {
    setup_task_workspaces(task_id, state.default_workspace_count);
}

pub fn setup_default_taskspace_workspaces(count: u32) {
    hypr_log::scoped(format!("setup_default_taskspace_workspaces count={count}"), || {
        hyprland::ensure_workspaces(&default_taskspace_workspace_names(count));
    });
}

fn capture_monitor_layout(
    state: &mut SessionState,
    taskspace_key: &str,
    allowed: &[String],
    monitors: &[Monitor],
) {
    capture_monitor_slots(state, taskspace_key, allowed, monitors);
    if let Some(focused) = monitors.iter().find(|m| m.focused) {
        if let Some(slot) = relative_slot_in_allowed(&focused.workspace_name, allowed) {
            state.last_workspace.insert(taskspace_key.to_string(), slot);
        }
    }
}

fn capture_monitor_slots(
    state: &mut SessionState,
    taskspace_key: &str,
    allowed: &[String],
    monitors: &[Monitor],
) {
    let entry = state
        .last_monitor_workspace
        .entry(taskspace_key.to_string())
        .or_default();
    for monitor in monitors {
        if let Some(slot) = relative_slot_in_allowed(&monitor.workspace_name, allowed) {
            entry.insert(monitor.name.clone(), slot);
        }
    }
}

fn sort_monitors_by_layout(mut monitors: Vec<Monitor>) -> Vec<Monitor> {
    monitors.sort_by(|a, b| (a.y, a.x, &a.name).cmp(&(b.y, b.x, &b.name)));
    monitors
}

fn persist_monitor_layout_from_hyprland(
    state: &mut SessionState,
    allowed: &[String],
    focused_monitor: Option<&str>,
) {
    let key = state.taskspace_key();
    if let Ok(monitors) = hyprland::list_monitors() {
        capture_monitor_slots(state, &key, allowed, &monitors);
    }
    if let Some(focused) = focused_monitor {
        if let Some(slot) = state
            .last_monitor_workspace
            .get(&key)
            .and_then(|map| map.get(focused))
            .copied()
        {
            state.last_workspace.insert(key, slot);
        }
    }
}

fn verify_monitor_targets(
    targets: &[(String, i32, String)],
    new_allowed: &[String],
    old_allowed: &[String],
) {
    for (monitor, slot, target_ws) in targets {
        for attempt in 0..2 {
            let current = hyprland::list_monitors().ok().unwrap_or_default();
            let current_name = current
                .iter()
                .find(|m| m.name == *monitor)
                .map(|m| m.workspace_name.as_str());
            if !monitor_needs_move(current_name, target_ws, new_allowed, old_allowed) {
                break;
            }
            hypr_log::scoped(
                format!(
                    "verify attempt {}: {monitor} slot{slot} still not on {target_ws} (currently {:?})",
                    attempt + 1,
                    current_name
                ),
                || hyprland::switch_workspace_on_monitor(monitor, target_ws),
            );
        }
    }
}

fn list_monitors_with_retry() -> Vec<Monitor> {
    hypr_log::scoped("list_monitors_with_retry", || {
        for attempt in 0..3 {
            if let Ok(monitors) = hyprland::list_monitors() {
                if !monitors.is_empty() {
                    return monitors;
                }
            }
            if attempt < 2 {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        }
        hyprland::list_monitors().unwrap_or_default()
    })
}

/// Assign each monitor its target workspace, preferring swaps over focus pulls.
fn restore_monitor_targets(
    targets: &[(String, i32, String)],
    layout_order: &[Monitor],
    new_allowed: &[String],
    old_allowed: &[String],
) {
    let layout_index: HashMap<&str, usize> = layout_order
        .iter()
        .enumerate()
        .map(|(i, m)| (m.name.as_str(), i))
        .collect();

    let max_passes = targets.len() + 2;
    for _ in 0..max_passes {
        let monitors = hyprland::list_monitors().ok().unwrap_or_default();
        let current: HashMap<String, String> = monitors
            .iter()
            .map(|m| (m.name.clone(), m.workspace_name.clone()))
            .collect();

        if targets.iter().all(|(monitor, _, target)| {
            !monitor_needs_move(
                current.get(monitor).map(String::as_str),
                target,
                new_allowed,
                old_allowed,
            )
        }) {
            break;
        }

        let mut pending: Vec<&(String, i32, String)> = targets
            .iter()
            .filter(|(monitor, _, target)| {
                monitor_needs_move(
                    current.get(monitor).map(String::as_str),
                    target,
                    new_allowed,
                    old_allowed,
                )
            })
            .collect();
        // Restore outer monitors before the primary so focus pulls do not skew layout.
        pending.sort_by_key(|(monitor, _, _)| {
            std::cmp::Reverse(layout_index.get(monitor.as_str()).copied().unwrap_or(0))
        });

        let Some((monitor, _, target_ws)) = pending.first() else {
            break;
        };

        if let Some(holder) = monitors.iter().find(|m| {
            m.name != **monitor && m.workspace_name == *target_ws
        }) {
            hypr_log::scoped(
                format!(
                    "restore: swap {monitor} with {} (holder of {target_ws})",
                    holder.name
                ),
                || hyprland::swap_active_workspaces(monitor, &holder.name),
            );
        } else {
            hypr_log::scoped(
                format!("restore: move {monitor} → {target_ws}"),
                || hyprland::switch_workspace_on_monitor(monitor, target_ws),
            );
        }
    }
}

fn monitor_at_target(current: Option<&str>, target: &str, new_allowed: &[String]) -> bool {
    let Some(current) = current else {
        return false;
    };
    current == target && new_allowed.iter().any(|name| name == target)
}

fn monitor_needs_move(
    current: Option<&str>,
    target: &str,
    new_allowed: &[String],
    old_allowed: &[String],
) -> bool {
    let Some(current) = current else {
        return true;
    };
    if old_allowed.iter().any(|name| name == current) {
        return true;
    }
    !monitor_at_target(Some(current), target, new_allowed)
}

/// Resolve the relative workspace slot for a monitor entering a taskspace.
///
/// Uses the saved per-monitor layout when available; otherwise assigns slots by
/// physical monitor order (primary → slot 1, secondary → slot 2, …).
fn resolve_monitor_slot(
    state: &SessionState,
    dest_key: &str,
    monitor_name: &str,
    monitor_index: usize,
    max_slots: usize,
) -> i32 {
    if let Some(slot) = state
        .last_monitor_workspace
        .get(dest_key)
        .and_then(|map| map.get(monitor_name))
        .copied()
    {
        return slot;
    }

    let slot = monitor_index + 1;
    slot.min(max_slots.max(1)) as i32
}

fn relative_to_name(state: &SessionState, relative: i32) -> Option<String> {
    let names = allowed_workspace_names(state);
    workspace_name_at_relative(&names, relative)
}

fn workspace_name_at_relative(names: &[String], relative: i32) -> Option<String> {
    let idx = (relative - 1).max(0) as usize;
    names.get(idx).cloned().or_else(|| names.first().cloned())
}

fn focus_relative_in_taskspace(state: &SessionState, relative: i32) -> Option<String> {
    let name = relative_to_name(state, relative)?;
    hypr_log::scoped(format!("focus_relative_in_taskspace slot {relative} → {name}"), || {
        hyprland::switch_workspace_for_navigation(&name);
    });
    Some(name)
}

fn relative_slot_in_allowed(workspace_name: &str, allowed: &[String]) -> Option<i32> {
    allowed
        .iter()
        .position(|n| n == workspace_name)
        .map(|i| (i + 1) as i32)
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
    use std::collections::HashMap;

    #[test]
    fn workspace_go_syncs_default_taskspace_for_global_slot() {
        use crate::models::{ContextMode, Task, TaskStatus};

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
        let name = workspace_go(&mut state, 1);
        assert_eq!(name.as_deref(), Some("1"));
        assert_eq!(state.context_mode, ContextMode::Default);
        assert!(state.current_task_id.is_none());
    }

    #[test]
    fn move_window_to_relative_uses_global_workspace_name() {
        use crate::models::{ContextMode, Task, TaskStatus};

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
        assert_eq!(
            move_window_to_relative(&state, 1).as_deref(),
            Some("1")
        );
        assert_eq!(
            move_window_to_relative(&state, 2).as_deref(),
            Some("auth-fix-2")
        );
    }

    #[test]
    fn set_taskspace_default_clears_task() {
        let mut state = SessionState {
            context_mode: ContextMode::Task,
            current_task_id: Some("test-task".into()),
            default_workspace_count: 10,
            ..Default::default()
        };
        set_taskspace(&mut state, ContextMode::Default, None).unwrap();
        assert_eq!(state.context_mode, ContextMode::Default);
        assert!(state.current_task_id.is_none());
    }

    #[test]
    fn clear_navigation_memory_resets_layout_fields() {
        let mut state = SessionState {
            last_monitor_workspace: HashMap::from([(
                "task:auth-fix".into(),
                HashMap::from([("DP-2".into(), 4)]),
            )]),
            last_workspace: HashMap::from([("task:auth-fix".into(), 4)]),
            ..Default::default()
        };
        clear_navigation_memory(&mut state);
        assert!(state.last_monitor_workspace.is_empty());
        assert_eq!(state.last_workspace.get("default"), Some(&1));
    }

    #[test]
    fn relative_slot_in_allowed_matches_exact_name() {
        let allowed = vec!["auth-fix-1".into(), "auth-fix-2".into()];
        assert_eq!(relative_slot_in_allowed("auth-fix-2", &allowed), Some(2));
        assert_eq!(relative_slot_in_allowed("7", &["1".into(), "7".into()]), Some(2));
        assert_eq!(relative_slot_in_allowed("7", &allowed), None);
        assert_eq!(relative_slot_in_allowed("3", &allowed), None);
    }

    #[test]
    fn resolve_monitor_slot_uses_saved_layout() {
        let state = SessionState {
            last_monitor_workspace: HashMap::from([(
                "task:auth-fix".into(),
                HashMap::from([("eDP-1".into(), 3), ("DP-2".into(), 5)]),
            )]),
            ..Default::default()
        };
        let slot = resolve_monitor_slot(&state, "task:auth-fix", "eDP-1", 0, 10);
        assert_eq!(slot, 3);
    }

    #[test]
    fn resolve_monitor_slot_enumerates_by_monitor_index() {
        let state = SessionState::default();
        assert_eq!(resolve_monitor_slot(&state, "task:new", "eDP-1", 0, 10), 1);
        assert_eq!(resolve_monitor_slot(&state, "task:new", "DP-2", 1, 10), 2);
        assert_eq!(resolve_monitor_slot(&state, "task:new", "HDMI-A-1", 2, 2), 2);
    }

    #[test]
    fn resolve_monitor_slot_saved_beats_enumeration() {
        let state = SessionState {
            last_monitor_workspace: HashMap::from([(
                "task:auth-fix".into(),
                HashMap::from([("DP-2".into(), 4)]),
            )]),
            ..Default::default()
        };
        let slot = resolve_monitor_slot(&state, "task:auth-fix", "DP-2", 1, 10);
        assert_eq!(slot, 4);
    }

    #[test]
    fn sort_monitors_by_layout_orders_top_left_first() {
        let monitors = sort_monitors_by_layout(vec![
            Monitor {
                name: "DP-2".into(),
                workspace_name: "1".into(),
                focused: false,
                x: 1920,
                y: 0,
            },
            Monitor {
                name: "eDP-1".into(),
                workspace_name: "2".into(),
                focused: true,
                x: 0,
                y: 0,
            },
        ]);
        assert_eq!(monitors[0].name, "eDP-1");
        assert_eq!(monitors[1].name, "DP-2");
    }

    #[test]
    fn monitor_at_target_requires_membership_in_new_allowed() {
        let allowed = vec!["1".into(), "2".into()];
        assert!(monitor_at_target(Some("2"), "2", &allowed));
        assert!(!monitor_at_target(Some("auth-fix-2"), "2", &allowed));
        assert!(!monitor_at_target(Some("2"), "1", &allowed));
    }

    #[test]
    fn monitor_needs_move_when_still_on_old_taskspace() {
        let new_allowed = vec!["1".into(), "2".into()];
        let old_allowed = vec!["auth-fix-1".into(), "auth-fix-2".into()];
        assert!(monitor_needs_move(
            Some("auth-fix-2"),
            "2",
            &new_allowed,
            &old_allowed,
        ));
        assert!(!monitor_needs_move(Some("2"), "2", &new_allowed, &old_allowed));
    }
}
