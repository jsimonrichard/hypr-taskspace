//! Run checkout-local `on_start` hooks when a task is created.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::error::Result;
use crate::hyprland;
use crate::models::{SessionState, Task};
use crate::repos::{load_repo_config, normalize_repo_path, RepoConfig};
use crate::task_repo::TaskRepoSetup;
use crate::workspace_nav::preferred_on_start_monitor;
use crate::workspaces::primary_task_workspace;

pub fn run_on_start_after_create(
    task: &Task,
    setup: &TaskRepoSetup,
    hyprland_enabled: bool,
    state: &SessionState,
) -> Result<()> {
    let Some((source_root, config)) = on_start_config(setup)? else {
        return Ok(());
    };

    let task_workspace = primary_task_workspace(
        &task.id,
        state.default_workspace_count,
        &state.global_workspace_slots,
    );
    let on_start_monitor =
        preferred_on_start_monitor(state, &task.id, config.on_start_monitor.as_deref());

    if hyprland_enabled && hyprland::available() {
        prepare_hyprland_for_on_start(&task_workspace, on_start_monitor.as_deref());
    }

    run_on_start_hook(
        task,
        setup,
        &source_root,
        &config,
        &task_workspace,
        on_start_monitor.as_deref(),
    )
}

fn on_start_config(setup: &TaskRepoSetup) -> Result<Option<(PathBuf, RepoConfig)>> {
    if !on_start_enabled() {
        return Ok(None);
    }

    let source_root = match setup {
        TaskRepoSetup::Linked { source_root, .. } | TaskRepoSetup::Direct { source_root } => {
            normalize_repo_path(source_root)
        }
        TaskRepoSetup::Scratch => return Ok(None),
    };

    let Some(config) = load_repo_config(&source_root)? else {
        return Ok(None);
    };
    if config
        .on_start
        .as_ref()
        .is_none_or(|path| path.trim().is_empty())
    {
        return Ok(None);
    }

    Ok(Some((source_root, config)))
}

fn prepare_hyprland_for_on_start(task_workspace: &str, monitor: Option<&str>) {
    if let Some(monitor) = monitor {
        hyprland::switch_workspace_on_monitor(monitor, task_workspace);
    } else {
        hyprland::switch_workspace_on_current_monitor(task_workspace);
    }
}

fn run_on_start_hook(
    task: &Task,
    setup: &TaskRepoSetup,
    source_root: &Path,
    config: &RepoConfig,
    task_workspace: &str,
    on_start_monitor: Option<&str>,
) -> Result<()> {
    let script_rel = config.on_start.as_deref().unwrap_or("").trim();
    let script_path = source_root.join(script_rel);
    if !script_path.is_file() {
        eprintln!(
            "tsk: on_start script not found: {}",
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
        .env("TSK_WORKTREE", if is_worktree { "1" } else { "0" });
    if let Some(monitor) = on_start_monitor {
        cmd.env("TSK_ON_START_MONITOR", monitor);
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());

    match cmd.spawn() {
        Ok(child) => {
            std::thread::spawn(move || {
                if let Ok(output) = child.wait_with_output() {
                    if !output.status.success() {
                        let stderr = String::from_utf8_lossy(&output.stderr);
                        eprintln!(
                            "tsk: on_start script {} exited with {}: {}",
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
                "tsk: failed to run on_start script {}: {err}",
                script_path.display()
            );
        }
    }

    Ok(())
}

fn on_start_enabled() -> bool {
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

    fn with_on_start_enabled<F: FnOnce()>(f: F) {
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

    #[test]
    fn on_start_uses_first_non_global_task_workspace() {
        with_on_start_enabled(|| {
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
            run_on_start_after_create(&task, &setup, false, &test_state(vec![1])).unwrap();

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
    fn on_start_runs_script_with_env_vars() {
        with_on_start_enabled(|| {
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
                     printf '%s\\n' \"$TSK_TASK_ID\" \"$TSK_TASK_REPO\" \"$TSK_SOURCE_REPO\" \"$TSK_WORKTREE\" \"$TSK_TASK_WORKSPACE\" > {marker}\n",
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
            run_on_start_after_create(&task, &setup, false, &test_state(vec![])).unwrap();

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
        });
    }

    #[test]
    fn on_start_skips_scratch_tasks() {
        with_on_start_enabled(|| {
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
            run_on_start_after_create(&task, &TaskRepoSetup::Scratch, false, &test_state(vec![]))
                .unwrap();
            std::thread::sleep(std::time::Duration::from_millis(100));
            assert!(!marker.exists());
        });
    }
}
