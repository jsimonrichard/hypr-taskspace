//! Daemon RPC client — used by the CLI when the daemon is running.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::{TskError, Result};
use crate::hypr_log;
use crate::hyprland;
use crate::models::{SessionState, Task, TaskStatus};
use crate::service::{MenuTask, TaskService};
use crate::workspace_nav;
use crate::config::load_config;
use crate::xdg::resolve_daemon_socket_path;

const RPC_TIMEOUT: Duration = Duration::from_secs(5);
const PING_TIMEOUT: Duration = Duration::from_millis(300);

const DAEMON_REQUIRED_MSG: &str = "tsk daemon is not running — run `tsk daemon start`";

#[derive(Debug, Deserialize)]
pub struct DaemonResponse {
    pub ok: bool,
    #[serde(default)]
    pub result: Value,
    #[serde(default)]
    pub error: Option<String>,
}

pub fn daemon_socket_path() -> Result<PathBuf> {
    let cfg = load_config()?;
    Ok(resolve_daemon_socket_path(&cfg.daemon_socket))
}

pub fn daemon_pid_path() -> Result<PathBuf> {
    let socket = daemon_socket_path()?;
    Ok(socket.with_file_name("daemon.pid"))
}

pub fn is_daemon_running() -> bool {
    ping_daemon().unwrap_or(false)
}

/// Error returned when a state mutation is attempted without the daemon.
pub fn ensure_daemon() -> Result<()> {
    if !daemon_socket_path().is_ok_and(|p| p.exists()) {
        return Err(TskError::Other(DAEMON_REQUIRED_MSG.into()));
    }
    if is_daemon_running() {
        Ok(())
    } else {
        Err(TskError::Other(DAEMON_REQUIRED_MSG.into()))
    }
}

pub fn ping_daemon() -> Result<bool> {
    if !daemon_socket_path().is_ok_and(|p| p.exists()) {
        return Ok(false);
    }
    match daemon_request_with_timeout("ping", json!({}), PING_TIMEOUT) {
        Ok(v) => Ok(v.get("pong").and_then(|p| p.as_bool()).unwrap_or(false)),
        Err(_) => Ok(false),
    }
}

pub fn daemon_request(method: &str, params: Value) -> Result<Value> {
    daemon_request_with_timeout(method, params, RPC_TIMEOUT)
}

fn daemon_request_with_timeout(method: &str, params: Value, timeout: Duration) -> Result<Value> {
    let path = daemon_socket_path()?;
    if !path.exists() {
        return Err(TskError::Other("tsk daemon is not running".into()));
    }

    let mut stream = UnixStream::connect(&path).map_err(|e| {
        TskError::Other(format!(
            "failed to connect to tsk daemon at {}: {e}",
            path.display()
        ))
    })?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|e| TskError::Other(e.to_string()))?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(|e| TskError::Other(e.to_string()))?;

    let payload = serde_json::to_string(&json!({ "method": method, "params": params }))
        .map_err(|e| TskError::Other(e.to_string()))?;
    stream
        .write_all(format!("{payload}\n").as_bytes())
        .map_err(|e| TskError::Other(e.to_string()))?;

    let response = read_response_line(&mut stream)?;
    let parsed: DaemonResponse =
        serde_json::from_str(&response).map_err(|e| TskError::Other(e.to_string()))?;
    if parsed.ok {
        Ok(parsed.result)
    } else {
        Err(TskError::Other(
            parsed.error.unwrap_or_else(|| "daemon error".into()),
        ))
    }
}

fn read_response_line(stream: &mut UnixStream) -> Result<String> {
    let mut buf = Vec::new();
    let mut scratch = [0u8; 4096];
    loop {
        let n = stream
            .read(&mut scratch)
            .map_err(|e| TskError::Other(e.to_string()))?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&scratch[..n]);
        if buf.contains(&b'\n') {
            break;
        }
    }
    let line = String::from_utf8_lossy(&buf);
    Ok(line.lines().next().unwrap_or("").trim().to_string())
}

/// CLI facade — routes to the daemon when available, otherwise uses `TaskService` directly.
pub struct DaemonClient {
    direct: TaskService,
}

impl DaemonClient {
    pub fn with_defaults() -> Result<Self> {
        Ok(Self {
            direct: TaskService::with_defaults()?,
        })
    }

    pub fn direct(&self) -> &TaskService {
        &self.direct
    }

    pub fn load_state(&self) -> Result<SessionState> {
        if is_daemon_running() {
            let v = daemon_request("get_state", json!({}))?;
            serde_json::from_value(v).map_err(|e| TskError::Other(e.to_string()))
        } else {
            self.direct.load_state()
        }
    }

    pub fn context_default(&self) -> Result<()> {
        ensure_daemon()?;
        daemon_request("context_default", json!({})).map(|_| ())
    }

    pub fn workspace_go(&self, relative: i32) -> Result<Option<String>> {
        self.hyprctl_then_remember(relative)
    }

    pub fn remember_workspace_go(&self, relative: i32) -> Result<Option<String>> {
        if is_daemon_running() {
            let v = daemon_request("workspace_remember", json!({ "relative": relative }))?;
            Ok(v.get("workspace")
                .and_then(|w| w.as_str())
                .map(str::to_string))
        } else {
            self.direct.remember_workspace_go(relative)
        }
    }

    pub fn workspace_next(&self) -> Result<Option<String>> {
        let relative = {
            let state = self.direct.load_state()?;
            workspace_nav::workspace_next_relative(&state)
        };
        let Some(relative) = relative else {
            return Ok(None);
        };
        self.hyprctl_then_remember(relative)
    }

    pub fn workspace_prev(&self) -> Result<Option<String>> {
        let relative = {
            let state = self.direct.load_state()?;
            workspace_nav::workspace_prev_relative(&state)
        };
        let Some(relative) = relative else {
            return Ok(None);
        };
        self.hyprctl_then_remember(relative)
    }

    pub fn workspace_goto(&self, name: &str) -> Result<Option<String>> {
        hypr_log::scoped(format!("daemon client workspace_goto {name}"), || {
            hyprland::switch_workspace_for_navigation(name);
        });
        if is_daemon_running() {
            spawn_daemon_request("workspace_remember_goto", json!({ "name": name }));
            Ok(Some(name.to_string()))
        } else {
            self.direct.remember_workspace_goto(name)
        }
    }

    fn hyprctl_then_remember(&self, relative: i32) -> Result<Option<String>> {
        if let Some(name) = crate::workspace_slots::switch_slot(relative) {
            self.sync_workspace_remember(relative)?;
            return Ok(Some(name));
        }
        let name = {
            let state = self.direct.load_state()?;
            workspace_nav::workspace_name_for_relative(&state, relative)
        };
        let Some(name) = name else {
            return Ok(None);
        };
        hypr_log::scoped(format!("daemon client hyprctl_then_remember slot {relative} → {name}"), || {
            hyprland::switch_workspace_for_navigation(&name);
        });
        self.sync_workspace_remember(relative)?;
        Ok(Some(name))
    }

    fn sync_workspace_remember(&self, relative: i32) -> Result<()> {
        if is_daemon_running() {
            spawn_daemon_request("workspace_remember", json!({ "relative": relative }));
        } else {
            let _ = self.direct.remember_workspace_go(relative)?;
        }
        Ok(())
    }

    pub fn create_task(
        &self,
        name: &str,
        switch: bool,
        repo: crate::task_repo::TaskRepoSource,
        repo_options: crate::task_repo::TaskRepoOptions,
    ) -> Result<Task> {
        ensure_daemon()?;
        let cwd = std::env::current_dir().ok();
        let mut body = json!({
            "name": name,
            "switch": switch,
            "create_worktree": repo_options.create_worktree,
        });
        if let Value::Object(mut repo_params) = repo.to_daemon_params(cwd.as_deref()) {
            body.as_object_mut().unwrap().append(&mut repo_params);
        }
        let v = daemon_request("create_task", body)?;
        serde_json::from_value(v).map_err(|e| TskError::Other(e.to_string()))
    }

    pub fn switch_task(&self, task_id: &str) -> Result<Task> {
        ensure_daemon()?;
        let v = daemon_request("switch_task", json!({ "task_id": task_id }))?;
        serde_json::from_value(v).map_err(|e| TskError::Other(e.to_string()))
    }

    pub fn archive_task(&self, task_id: &str) -> Result<()> {
        ensure_daemon()?;
        daemon_request("archive_task", json!({ "task_id": task_id })).map(|_| ())
    }

    pub fn delete_task(&self, task_id: &str) -> Result<()> {
        ensure_daemon()?;
        daemon_request("delete_task", json!({ "task_id": task_id })).map(|_| ())
    }

    pub fn open_terminal(&self, task_id: Option<&str>, host: bool) -> Result<()> {
        if is_daemon_running() {
            let mut body = json!({ "host": host });
            if let Some(task_id) = task_id {
                body["task_id"] = json!(task_id);
            }
            daemon_request("open_terminal", body).map(|_| ())
        } else {
            self.direct.open_terminal(task_id, host)
        }
    }

    pub fn preview_task_teardown(
        &self,
        task_id: &str,
    ) -> Result<crate::task_cleanup::TaskTeardownPreview> {
        ensure_daemon()?;
        let v = daemon_request("preview_task_teardown", json!({ "task_id": task_id }))?;
        serde_json::from_value(v).map_err(|e| TskError::Other(e.to_string()))
    }

    pub fn resolve_task(&self, name_or_id: &str) -> Result<Task> {
        if is_daemon_running() {
            let v = daemon_request("resolve_task", json!({ "name_or_id": name_or_id }))?;
            serde_json::from_value(v).map_err(|e| TskError::Other(e.to_string()))
        } else {
            self.direct.resolve_task(name_or_id)
        }
    }

    pub fn tasks_for_menu(&self) -> Result<Vec<MenuTask>> {
        if is_daemon_running() {
            let v = daemon_request("tasks_for_menu", json!({}))?;
            serde_json::from_value(v).map_err(|e| TskError::Other(e.to_string()))
        } else {
            self.direct.tasks_for_menu()
        }
    }

    pub fn taskspace_label(&self) -> Result<String> {
        if is_daemon_running() {
            let v = daemon_request("taskspace_label", json!({}))?;
            Ok(v.get("label")
                .and_then(|l| l.as_str())
                .unwrap_or("default")
                .to_string())
        } else {
            self.direct.taskspace_label()
        }
    }

    pub fn list_active_tasks(&self) -> Result<Vec<Task>> {
        if is_daemon_running() {
            let state = self.load_state()?;
            Ok(state
                .tasks
                .values()
                .filter(|t| t.status != TaskStatus::Archived)
                .cloned()
                .collect())
        } else {
            self.direct.list_active_tasks()
        }
    }

    pub fn list_archived_tasks(&self) -> Result<Vec<Task>> {
        if is_daemon_running() {
            let state = self.load_state()?;
            Ok(state
                .tasks
                .values()
                .filter(|t| t.status == TaskStatus::Archived)
                .cloned()
                .collect())
        } else {
            self.direct.list_archived_tasks()
        }
    }
}

fn spawn_daemon_request(method: &str, params: Value) {
    let method = method.to_string();
    std::thread::spawn(move || {
        let _ = daemon_request(&method, params);
    });
}
