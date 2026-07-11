//! Suppress Hyprland `workspacev2` feedback that reverts a just-finished taskspace switch.
//!
//! Keybind hot paths can focus a leaving taskspace workspace (stale slot cache) while
//! `set_taskspace` is still running. When that event is handled after the intentional
//! switch commits, `sync_external_workspace` would otherwise pull context back.

use std::collections::HashSet;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::models::SessionState;

struct RecentIntentionalSwitch {
    dest_key: String,
    old_allowed: HashSet<String>,
    at: Instant,
}

static RECENT: Mutex<Option<RecentIntentionalSwitch>> = Mutex::new(None);

const GUARD_TTL: Duration = Duration::from_secs(2);

pub fn record(dest_key: &str, old_allowed: &[String]) {
    let mut guard = RECENT.lock().expect("taskspace switch guard lock");
    *guard = Some(RecentIntentionalSwitch {
        dest_key: dest_key.to_string(),
        old_allowed: old_allowed.iter().cloned().collect(),
        at: Instant::now(),
    });
}

/// Whether an external workspace focus should be ignored because it would revert a
/// taskspace switch that just completed.
pub fn should_ignore_external_revert(state: &SessionState, workspace_name: &str) -> bool {
    let guard = RECENT.lock().expect("taskspace switch guard lock");
    let Some(recent) = guard.as_ref() else {
        return false;
    };
    if recent.at.elapsed() > GUARD_TTL {
        return false;
    }
    if state.taskspace_key() != recent.dest_key {
        return false;
    }
    recent.old_allowed.contains(workspace_name)
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
            container_isolation: false,
            workspace_count: 10,
            browser_profile: None,
            created_at: chrono::Utc::now(),
            last_active_at: chrono::Utc::now(),
            agent_notes_path: None,
            ports: vec![],
        };
        SessionState {
            context_mode: ContextMode::Task,
            current_task_id: Some("billing".into()),
            default_workspace_count: 10,
            tasks: HashMap::from([
                ("auth-fix".into(), task),
                (
                    "billing".into(),
                    Task {
                        id: "billing".into(),
                        name: "Billing".into(),
                        status: TaskStatus::Active,
                        repo_url: None,
                        repo_path: PathBuf::from("/tmp/billing/repo"),
                        source_repo_path: None,
                        branch: None,
                        container_name: "tsk-billing".into(),
                        container_isolation: false,
                        workspace_count: 10,
                        browser_profile: None,
                        created_at: chrono::Utc::now(),
                        last_active_at: chrono::Utc::now(),
                        agent_notes_path: None,
                        ports: vec![],
                    },
                ),
            ]),
            ..Default::default()
        }
    }

    #[test]
    fn ignores_leaving_taskspace_workspace_after_intentional_switch() {
        let old_allowed = vec!["auth-fix-1".into(), "auth-fix-2".into()];
        record("task:billing", &old_allowed);

        let state = task_state();
        assert!(should_ignore_external_revert(&state, "auth-fix-2"));
        assert!(!should_ignore_external_revert(&state, "billing-2"));
    }

    #[test]
    fn guard_does_not_apply_to_wrong_destination() {
        record("task:billing", &["auth-fix-1".into()]);
        let mut state = task_state();
        state.current_task_id = Some("auth-fix".into());
        assert!(!should_ignore_external_revert(&state, "auth-fix-1"));
    }
}
