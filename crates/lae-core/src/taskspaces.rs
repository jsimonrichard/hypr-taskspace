use std::collections::HashSet;

use crate::models::{ContextMode, SessionState};

pub const DEFAULT_MIN_VISIBLE_WORKSPACES: u32 = 5;

pub fn visible_default_workspace_count(
    state: &SessionState,
    allowed: &[String],
    active_rel: i32,
    occupied: &HashSet<i32>,
) -> u32 {
    let total = allowed.len() as u32;
    if state.context_mode == ContextMode::Task {
        return total;
    }

    // Default taskspace: show at least DEFAULT_MIN_VISIBLE_WORKSPACES slots.
    let highest_occupied = occupied.iter().copied().max().unwrap_or(0) as u32;
    let active = active_rel.max(0) as u32;
    let visible = active
        .max(highest_occupied)
        .max(DEFAULT_MIN_VISIBLE_WORKSPACES);
    visible.min(total).min(10)
}
