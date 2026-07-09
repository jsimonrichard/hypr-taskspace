//! Run checkout-local hooks when a task is created or restored.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::config::TskConfig;
use crate::error::Result;
use crate::hyprland;
use crate::models::{SessionState, Task};
use crate::repos::{load_repo_config, normalize_repo_path, RepoConfig};
use crate::task_paths::is_managed_task_checkout;
use crate::task_repo::TaskRepoSetup;
use crate::vcs::{vcs_kind_at, VcsKind};
use crate::workspace_nav::preferred_on_start_monitor;
use crate::workspaces::primary_task_workspace;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TaskHook {
    Create,
    Restore,
}

impl TaskHook {
    fn env_name(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Restore => "restore",
        }
    }
}

/// Run the create hook after a new task is provisioned.
pub fn run_on_create_after_create(
    task: &Task,
    setup: &TaskRepoSetup,
    hyprland_enabled: bool,
    state: &SessionState,
) -> Result<()> {
    run_task_hook(TaskHook::Create, task, setup, hyprland_enabled, state)
}

/// Run the restore hook after an archived task is reactivated.
pub fn run_on_restore_after_restore(
    task: &Task,
    config: &TskConfig,
    hyprland_enabled: bool,
    state: &SessionState,
) -> Result<()> {
    let setup = setup_for_restored_task(task, config);
    run_task_hook(TaskHook::Restore, task, &setup, hyprland_enabled, state)
}

fn run_task_hook(
    hook: TaskHook,
    task: &Task,
    setup: &TaskRepoSetup,
    hyprland_enabled: bool,
    state: &SessionState,
) -> Result<()> {
    if !hooks_enabled() {
        return Ok(());
    }

    let Some((source_root, repo_config)) = hook_config(hook, setup)? else {
        return Ok(());
    };

    let task_workspace = primary_task_workspace(
        &task.id,
        state.default_workspace_count,
        &state.global_workspace_slots,
    );
    let monitor = preferred_on_start_monitor(
        state,
        &task.id,
        repo_config.on_start_monitor.as_deref(),
    );

    if hyprland_enabled && hyprland::available() {
        prepare_hyprland_for_hook(&task_workspace, monitor.as_deref());
    }

    run_hook_script(
        hook,
        task,
        setup,
        &source_root,
        &repo_config,
        &task_workspace,
        monitor.as_deref(),
    )
}

fn hook_config(hook: TaskHook, setup: &TaskRepoSetup) -> Result<Option<(PathBuf, RepoConfig)>> {
    let source_root = match setup {
        TaskRepoSetup::Linked { source_root, .. } | TaskRepoSetup::Direct { source_root } => {
            normalize_repo_path(source_root)
        }
        TaskRepoSetup::Scratch => return Ok(None),
    };

    let config = load_repo_config(&source_root)?.unwrap_or_default();

    let script = match hook {
        TaskHook::Create => config.on_create_script_at(&source_root),
        TaskHook::Restore => config.on_restore_script_at(&source_root),
    };
    if script.is_none() {
        return Ok(None);
    }

    Ok(Some((source_root, config)))
}

fn setup_for_restored_task(task: &Task, config: &TskConfig) -> TaskRepoSetup {
    if task.source_repo_path.is_none() {
        return TaskRepoSetup::Scratch;
    }
    let source_root = normalize_repo_path(task.source_repo_path.as_ref().unwrap());
    if is_managed_task_checkout(&task.repo_path, &config.tasks_base_dir, &task.id) {
        let kind = vcs_kind_at(&source_root).unwrap_or(VcsKind::Git);
        TaskRepoSetup::Linked {
            source_root,
            kind,
        }
    } else {
        TaskRepoSetup::Direct { source_root }
    }
}

fn prepare_hyprland_for_hook(task_workspace: &str, monitor: Option<&str>) {
    if let Some(monitor) = monitor {
        hyprland::switch_workspace_on_monitor(monitor, task_workspace);
    } else {
        hyprland::switch_workspace_on_current_monitor(task_workspace);
    }
}

fn run_hook_script(
    hook: TaskHook,
    task: &Task,
    setup: &TaskRepoSetup,
    source_root: &Path,
    config: &RepoConfig,
    task_workspace: &str,
    monitor: Option<&str>,
) -> Result<()> {
    let script_rel = match hook {
        TaskHook::Create => config.on_create_script_at(source_root),
        TaskHook::Restore => config.on_restore_script_at(source_root),
    }
    .unwrap_or("");
    let script_path = source_root.join(script_rel);
    if !script_path.is_file() {
        eprintln!(
            "tsk: {} hook script not found: {}",
            hook.env_name(),
            script_path.display()
        );
        return Ok(());
    }

    let is_worktree = matches!(setup, TaskRepoSetup::Linked { .. });
    let mut cmd = command_for_script(&script_path);
    cmd.current_dir(&task.repo_path)
        .env("TSK_TASK_ID", &task.id)
        .env("TSK_TASK_NAME", &task.name)
        .env(
            "TSK_TASK_REPO",
            task.repo_path.to_string_lossy().as_ref(),
        )
        .env("TSK_SOURCE_REPO", source_root.to_string_lossy().as_ref())
        .env("TSK_TASK_WORKSPACE", task_workspace)
        .env("TSK_WORKTREE", if is_worktree { "1" } else { "0" })
        .env("TSK_TASK_HOOK", hook.env_name());
    if let Some(monitor) = monitor {
        cmd.env("TSK_ON_START_MONITOR", monitor);
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());

    match cmd.spawn() {
        Ok(child) => {
            let hook_name = hook.env_name().to_string();
            std::thread::spawn(move || {
                if let Ok(output) = child.wait_with_output() {
                    if !output.status.success() {
                        let stderr = String::from_utf8_lossy(&output.stderr);
                        eprintln!(
                            "tsk: {hook_name} hook script {} exited with {}: {}",
                            script_path.display(),
                            output.status,
                            stderr.trim()
                        );
                    }
                }
            });
        }
        Err(err) => {
            eprintln!(
                "tsk: failed to run {} hook script {}: {err}",
                hook.env_name(),
                script_path.display()
            );
        }
    }

    Ok(())
}

fn hooks_enabled() -> bool {
    if matches!(
        std::env::var("TSK_DISABLE_ON_START").as_deref(),
        Ok("1") | Ok("true") | Ok("TRUE")
    ) {
        return false;
    }
    if cfg!(test) {
        return std::env::var("TSK_ENABLE_ON_START").as_deref() == Ok("1");
    }
    true
}

fn command_for_script(script_path: &Path) -> Command {
    let is_shell_script = script_path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| matches!(ext, "sh" | "bash" | "zsh"));
    if is_shell_script || !is_executable(script_path) {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
        let mut cmd = Command::new(shell);
        cmd.arg(script_path);
        cmd
    } else {
        Command::new(script_path)
    }
}

fn is_executable(path: &Path) -> bool {
    path.metadata()
        .ok()
        .is_some_and(|meta| meta.permissions().mode() & 0o111 != 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::TaskStatus;
    use chrono::Utc;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn with_hooks_enabled<F: FnOnce()>(f: F) {
        std::env::set_var("TSK_ENABLE_ON_START", "1");
        f();
        std::env::remove_var("TSK_ENABLE_ON_START");
    }

    fn test_state(global_slots: Vec<u32>) -> SessionState {
        SessionState {
            default_workspace_count: 10,
            global_workspace_slots: global_slots,
            ..Default::default()
        }
    }

    fn test_task(dir: &Path, id: &str, repo_path: PathBuf) -> Task {
        let now = Utc::now();
        Task {
            id: id.into(),
            name: "Test Task".into(),
            status: TaskStatus::Active,
            repo_url: None,
            repo_path,
            source_repo_path: Some(dir.to_path_buf()),
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

    fn test_config(dir: &Path) -> TskConfig {
        let mut cfg = TskConfig::default();
        cfg.tasks_base_dir = dir.join("tasks");
        cfg
    }

    #[test]
    fn on_create_uses_first_non_global_task_workspace() {
        with_hooks_enabled(|| {
            let dir = tempfile::tempdir().unwrap();
            let source = dir.path().join("project");
            let task_repo = dir.path().join("task-checkout");
            fs::create_dir_all(source.join(".tsk")).unwrap();
            fs::create_dir_all(&task_repo).unwrap();

            let marker = dir.path().join("ran.env");
            fs::write(
                source.join(".tsk/on-start.sh"),
                format!(
                    "#!/bin/sh\n\
                     printf '%s' \"$TSK_TASK_WORKSPACE\" > {marker}\n",
                    marker = marker.display()
                ),
            )
            .unwrap();
            fs::write(
                crate::repos::repo_config_path(&source),
                "on_start = \".tsk/on-start.sh\"\n",
            )
            .unwrap();

            let task = test_task(&source, "tabc123", task_repo.clone());
            let setup = TaskRepoSetup::Linked {
                source_root: source.clone(),
                kind: crate::vcs::VcsKind::Git,
            };
            run_on_create_after_create(&task, &setup, false, &test_state(vec![1])).unwrap();

            for _ in 0..50 {
                if marker.is_file() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }

            assert_eq!(fs::read_to_string(&marker).unwrap(), "tabc123-2");
        });
    }

    #[test]
    fn on_create_runs_script_with_env_vars() {
        with_hooks_enabled(|| {
            let dir = tempfile::tempdir().unwrap();
            let source = dir.path().join("project");
            let task_repo = dir.path().join("task-checkout");
            fs::create_dir_all(source.join(".tsk")).unwrap();
            fs::create_dir_all(&task_repo).unwrap();

            let marker = dir.path().join("ran.env");
            fs::write(
                source.join(".tsk/on-start.sh"),
                format!(
                    "#!/bin/sh\n\
                     printf '%s\\n' \"$TSK_TASK_ID\" \"$TSK_TASK_REPO\" \"$TSK_SOURCE_REPO\" \"$TSK_WORKTREE\" \"$TSK_TASK_WORKSPACE\" \"$TSK_TASK_HOOK\" > {marker}\n",
                    marker = marker.display()
                ),
            )
            .unwrap();
            fs::write(
                crate::repos::repo_config_path(&source),
                "on_start = \".tsk/on-start.sh\"\n",
            )
            .unwrap();

            let task = test_task(&source, "tabc123", task_repo.clone());
            let setup = TaskRepoSetup::Linked {
                source_root: source.clone(),
                kind: crate::vcs::VcsKind::Git,
            };
            run_on_create_after_create(&task, &setup, false, &test_state(vec![])).unwrap();

            for _ in 0..50 {
                if marker.is_file() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }

            let contents = fs::read_to_string(&marker).unwrap();
            let lines: Vec<_> = contents.lines().collect();
            assert_eq!(lines[0], "tabc123");
            assert_eq!(lines[1], task_repo.display().to_string());
            assert_eq!(lines[2], source.display().to_string());
            assert_eq!(lines[3], "1");
            assert_eq!(lines[4], "tabc123-1");
            assert_eq!(lines[5], "create");
        });
    }

    #[test]
    fn on_start_runs_without_repo_toml() {
        with_hooks_enabled(|| {
            let dir = tempfile::tempdir().unwrap();
            let source = dir.path().join("project");
            let task_repo = dir.path().join("task-checkout");
            fs::create_dir_all(source.join(".tsk")).unwrap();
            fs::create_dir_all(&task_repo).unwrap();

            let marker = dir.path().join("default.env");
            fs::write(
                source.join(".tsk/on-start.sh"),
                format!("touch {}\n", marker.display()),
            )
            .unwrap();

            let task = test_task(&source, "tabc123", task_repo);
            let setup = TaskRepoSetup::Linked {
                source_root: source.clone(),
                kind: crate::vcs::VcsKind::Git,
            };
            run_on_create_after_create(&task, &setup, false, &test_state(vec![])).unwrap();

            for _ in 0..50 {
                if marker.is_file() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            assert!(marker.is_file());
        });
    }

    #[test]
    fn on_start_runs_on_create() {
        with_hooks_enabled(|| {
            let dir = tempfile::tempdir().unwrap();
            let source = dir.path().join("project");
            let task_repo = dir.path().join("task-checkout");
            fs::create_dir_all(source.join(".tsk")).unwrap();
            fs::create_dir_all(&task_repo).unwrap();

            let marker = dir.path().join("legacy.env");
            fs::write(
                source.join(".tsk/on-start.sh"),
                format!("touch {}\n", marker.display()),
            )
            .unwrap();
            fs::write(
                crate::repos::repo_config_path(&source),
                "on_start = \".tsk/on-start.sh\"\n",
            )
            .unwrap();

            let task = test_task(&source, "tabc123", task_repo);
            let setup = TaskRepoSetup::Linked {
                source_root: source.clone(),
                kind: crate::vcs::VcsKind::Git,
            };
            run_on_create_after_create(&task, &setup, false, &test_state(vec![])).unwrap();

            for _ in 0..50 {
                if marker.is_file() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            assert!(marker.is_file());
        });
    }

    #[test]
    fn on_start_runs_on_restore_by_default() {
        with_hooks_enabled(|| {
            let dir = tempfile::tempdir().unwrap();
            let source = dir.path().join("project");
            let task_repo = dir
                .path()
                .join("tasks")
                .join("tabc123")
                .join("workspace")
                .join("project");
            fs::create_dir_all(source.join(".tsk")).unwrap();
            fs::create_dir_all(&task_repo).unwrap();

            let marker = dir.path().join("restore.env");
            fs::write(
                source.join(".tsk/on-start.sh"),
                format!(
                    "#!/bin/sh\n\
                     printf '%s' \"$TSK_TASK_HOOK\" > {marker}\n",
                    marker = marker.display()
                ),
            )
            .unwrap();
            fs::write(
                crate::repos::repo_config_path(&source),
                "on_start = \".tsk/on-start.sh\"\n",
            )
            .unwrap();

            let task = test_task(&source, "tabc123", task_repo);
            run_on_restore_after_restore(
                &task,
                &test_config(dir.path()),
                false,
                &test_state(vec![]),
            )
            .unwrap();

            for _ in 0..50 {
                if marker.is_file() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }

            assert_eq!(fs::read_to_string(&marker).unwrap(), "restore");
        });
    }

    #[test]
    fn on_restore_override_takes_precedence() {
        with_hooks_enabled(|| {
            let dir = tempfile::tempdir().unwrap();
            let source = dir.path().join("project");
            let task_repo = dir
                .path()
                .join("tasks")
                .join("tabc123")
                .join("workspace")
                .join("project");
            fs::create_dir_all(source.join(".tsk")).unwrap();
            fs::create_dir_all(&task_repo).unwrap();

            let marker = dir.path().join("restore.env");
            fs::write(source.join(".tsk/on-start.sh"), "touch should-not-run\n").unwrap();
            fs::write(
                source.join(".tsk/on-restore.sh"),
                format!("touch {}\n", marker.display()),
            )
            .unwrap();
            fs::write(
                crate::repos::repo_config_path(&source),
                "on_start = \".tsk/on-start.sh\"\non_restore = \".tsk/on-restore.sh\"\n",
            )
            .unwrap();

            let task = test_task(&source, "tabc123", task_repo);
            run_on_restore_after_restore(
                &task,
                &test_config(dir.path()),
                false,
                &test_state(vec![]),
            )
            .unwrap();

            for _ in 0..50 {
                if marker.is_file() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }

            assert!(marker.is_file());
        });
    }

    #[test]
    fn on_restore_runs_configured_script() {
        with_hooks_enabled(|| {
            let dir = tempfile::tempdir().unwrap();
            let source = dir.path().join("project");
            let task_repo = dir
                .path()
                .join("tasks")
                .join("tabc123")
                .join("workspace")
                .join("project");
            fs::create_dir_all(source.join(".tsk")).unwrap();
            fs::create_dir_all(&task_repo).unwrap();

            let marker = dir.path().join("restore.env");
            fs::write(
                source.join(".tsk/on-restore.sh"),
                format!(
                    "#!/bin/sh\n\
                     printf '%s' \"$TSK_TASK_HOOK\" > {marker}\n",
                    marker = marker.display()
                ),
            )
            .unwrap();
            fs::write(
                crate::repos::repo_config_path(&source),
                "on_restore = \".tsk/on-restore.sh\"\n",
            )
            .unwrap();

            let task = test_task(&source, "tabc123", task_repo);
            run_on_restore_after_restore(
                &task,
                &test_config(dir.path()),
                false,
                &test_state(vec![]),
            )
            .unwrap();

            for _ in 0..50 {
                if marker.is_file() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }

            assert_eq!(fs::read_to_string(&marker).unwrap(), "restore");
        });
    }

    #[test]
    fn on_create_skips_scratch_tasks() {
        with_hooks_enabled(|| {
            let dir = tempfile::tempdir().unwrap();
            let source = dir.path().join("project");
            fs::create_dir_all(source.join(".tsk")).unwrap();
            let marker = dir
                .path()
                .join(format!(
                    "scratch-{}.env",
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_nanos()
                ));
            fs::write(
                source.join(".tsk/on-start.sh"),
                format!("touch {}\n", marker.display()),
            )
            .unwrap();
            fs::write(
                crate::repos::repo_config_path(&source),
                "on_start = \".tsk/on-start.sh\"\n",
            )
            .unwrap();

            let task = test_task(&source, "tscratch", dir.path().join("scratch"));
            run_on_create_after_create(&task, &TaskRepoSetup::Scratch, false, &test_state(vec![]))
                .unwrap();
            std::thread::sleep(std::time::Duration::from_millis(100));
            assert!(!marker.exists());
        });
    }
}
