//! Taskspace-scoped environment variables for spawned processes.
//!
//! These vars snapshot the taskspace context at spawn time so long-running
//! processes do not depend on global CLI/daemon state that may change later.

use std::path::Path;
use std::process::Command;

use crate::models::{ContextMode, SessionState, Task};
use crate::repos::normalize_repo_path;
use crate::task_paths::is_managed_task_checkout;
use crate::workspaces::primary_task_workspace;

/// Environment for the default taskspace.
pub fn build_default_taskspace_env() -> Vec<(String, String)> {
    vec![
        ("TSK_TASKSPACE".into(), "default".into()),
        ("TSK_CONTEXT_MODE".into(), ContextMode::Default.as_str().into()),
    ]
}

/// Environment for the active taskspace described by `state`.
pub fn build_taskspace_env(state: &SessionState, tasks_base_dir: &Path) -> Vec<(String, String)> {
    if state.context_mode == ContextMode::Task {
        if let Some(task_id) = state.current_task_id.as_deref() {
            if let Some(task) = state.tasks.get(task_id) {
                return build_task_env(state, task, tasks_base_dir, None);
            }
        }
    }
    build_default_taskspace_env()
}

/// Environment for a specific task (used when a process is owned by that task).
pub fn build_task_env(
    state: &SessionState,
    task: &Task,
    tasks_base_dir: &Path,
    worktree: Option<bool>,
) -> Vec<(String, String)> {
    let primary_non_global_workspace = primary_task_workspace(
        &task.id,
        state.default_workspace_count,
        &state.global_workspace_slots,
    );
    let is_worktree = worktree.unwrap_or_else(|| {
        task.source_repo_path.is_some()
            && is_managed_task_checkout(&task.repo_path, tasks_base_dir, &task.id)
    });

    let mut env = vec![
        (
            "TSK_TASKSPACE".into(),
            format!("task:{}", task.id),
        ),
        (
            "TSK_CONTEXT_MODE".into(),
            ContextMode::Task.as_str().into(),
        ),
        ("TSK_TASK_ID".into(), task.id.clone()),
        ("TSK_TASK_NAME".into(), task.name.clone()),
        (
            "TSK_TASK_REPO".into(),
            task.repo_path.to_string_lossy().into_owned(),
        ),
        ("TSK_PRIMARY_NON_GLOBAL_WORKSPACE".into(), primary_non_global_workspace),
        (
            "TSK_WORKTREE".into(),
            if is_worktree { "1" } else { "0" }.into(),
        ),
    ];

    if let Some(source) = task.source_repo_path.as_ref() {
        env.push((
            "TSK_SOURCE_REPO".into(),
            normalize_repo_path(source).to_string_lossy().into_owned(),
        ));
    }

    env
}

pub fn apply_env(cmd: &mut Command, env: &[(String, String)]) {
    for (key, value) in env {
        cmd.env(key, value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::TaskStatus;
    use chrono::Utc;
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn env_map(vars: &[(String, String)]) -> HashMap<String, String> {
        vars.iter().cloned().collect()
    }

    fn test_task(id: &str, repo_path: PathBuf, source: Option<PathBuf>) -> Task {
        let now = Utc::now();
        Task {
            id: id.into(),
            name: "Example".into(),
            status: TaskStatus::Active,
            repo_url: None,
            repo_path,
            source_repo_path: source,
            branch: None,
            container_name: format!("tsk-{id}"),
            workspace_count: 10,
            browser_profile: None,
            created_at: now,
            last_active_at: now,
            agent_notes_path: None,
            ports: vec![],
        }
    }

    #[test]
    fn default_taskspace_env() {
        let env = env_map(&build_default_taskspace_env());
        assert_eq!(env["TSK_TASKSPACE"], "default");
        assert_eq!(env["TSK_CONTEXT_MODE"], "default");
    }

    #[test]
    fn task_env_includes_repo_and_source() {
        let base = PathBuf::from("/tmp/tsk-tasks");
        let source = PathBuf::from("/home/user/project");
        let repo = base.join("tabc123").join("workspace").join("project");
        let task = test_task("tabc123", repo.clone(), Some(source.clone()));
        let state = SessionState {
            default_workspace_count: 10,
            global_workspace_slots: vec![1],
            ..Default::default()
        };

        let env = env_map(&build_task_env(&state, &task, &base, None));
        assert_eq!(env["TSK_TASKSPACE"], "task:tabc123");
        assert_eq!(env["TSK_CONTEXT_MODE"], "task");
        assert_eq!(env["TSK_TASK_ID"], "tabc123");
        assert_eq!(env["TSK_TASK_REPO"], repo.display().to_string());
        assert_eq!(env["TSK_SOURCE_REPO"], source.display().to_string());
        assert_eq!(env["TSK_PRIMARY_NON_GLOBAL_WORKSPACE"], "tabc123-2");
        assert_eq!(env["TSK_WORKTREE"], "1");
    }

    #[test]
    fn taskspace_env_from_state_uses_current_task() {
        let base = PathBuf::from("/tmp/tsk-tasks");
        let task = test_task(
            "tabc123",
            base.join("tabc123").join("workspace"),
            None,
        );
        let mut tasks = HashMap::new();
        tasks.insert(task.id.clone(), task);
        let state = SessionState {
            context_mode: ContextMode::Task,
            current_task_id: Some("tabc123".into()),
            tasks,
            ..Default::default()
        };

        let env = env_map(&build_taskspace_env(&state, &base));
        assert_eq!(env["TSK_TASKSPACE"], "task:tabc123");
        assert_eq!(env["TSK_TASK_ID"], "tabc123");
    }

    #[test]
    fn taskspace_env_falls_back_to_default() {
        let state = SessionState {
            context_mode: ContextMode::Default,
            ..Default::default()
        };
        let env = env_map(&build_taskspace_env(&state, Path::new("/tmp/tsk-tasks")));
        assert_eq!(env["TSK_TASKSPACE"], "default");
    }
}
