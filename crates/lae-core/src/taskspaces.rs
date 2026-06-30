use std::collections::HashSet;

use crate::models::SessionState;

pub const DEFAULT_MIN_VISIBLE_WORKSPACES: u32 = 5;

pub fn visible_default_workspace_count(
    _state: &SessionState,
    allowed: &[String],
    active_rel: i32,
    occupied: &HashSet<i32>,
) -> u32 {
    let total = allowed.len() as u32;
    let highest_occupied = occupied.iter().copied().max().unwrap_or(0) as u32;
    let active = active_rel.max(0) as u32;
    let visible = active
        .max(highest_occupied)
        .max(DEFAULT_MIN_VISIBLE_WORKSPACES);
    visible.min(total).min(10)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ContextMode, SessionState};

    fn state(mode: ContextMode) -> SessionState {
        SessionState {
            context_mode: mode,
            current_task_id: if mode == ContextMode::Task {
                Some("auth-fix".into())
            } else {
                None
            },
            ..Default::default()
        }
    }

    #[test]
    fn task_mode_collapses_empty_high_slots() {
        let allowed: Vec<String> = (1..=10).map(|n| format!("auth-fix-{n}")).collect();
        let occupied = HashSet::from([1, 3]);
        let visible = visible_default_workspace_count(&state(ContextMode::Task), &allowed, 1, &occupied);
        assert_eq!(visible, DEFAULT_MIN_VISIBLE_WORKSPACES);
    }

    #[test]
    fn task_mode_expands_for_active_slot_beyond_min() {
        let allowed: Vec<String> = (1..=10).map(|n| format!("auth-fix-{n}")).collect();
        let visible = visible_default_workspace_count(&state(ContextMode::Task), &allowed, 8, &HashSet::new());
        assert_eq!(visible, 8);
    }
}
