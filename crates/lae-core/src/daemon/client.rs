//! Daemon RPC client — used by the CLI when the daemon is running.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::{LaeError, Result};
use crate::hyprland;
use crate::models::{SessionState, Task, TaskStatus};
use crate::service::{MenuTask, TaskService};
use crate::workspace_nav;
use crate::xdg::lae_runtime_dir;

const RPC_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Deserialize)]
pub struct DaemonResponse {
    pub ok: bool,
    #[serde(default)]
    pub result: Value,
    #[serde(default)]
    pub error: Option<String>,
}

pub fn daemon_socket_path() -> Result<PathBuf> {
    Ok(lae_runtime_dir()?.join("daemon.sock"))
}

pub fn daemon_pid_path() -> Result<PathBuf> {
    Ok(lae_runtime_dir()?.join("daemon.pid"))
}

pub fn is_daemon_running() -> bool {
    ping_daemon().unwrap_or(false)
}

pub fn ping_daemon() -> Result<bool> {
    if !daemon_socket_path().is_ok_and(|p| p.exists()) {
        return Ok(false);
    }
    match daemon_request("ping", json!({})) {
        Ok(v) => Ok(v.get("pong").and_then(|p| p.as_bool()).unwrap_or(false)),
        Err(_) => Ok(false),
    }
}

pub fn daemon_request(method: &str, params: Value) -> Result<Value> {
    let path = daemon_socket_path()?;
    if !path.exists() {
        return Err(LaeError::Other("lae daemon is not running".into()));
    }

    let mut stream = UnixStream::connect(&path).map_err(|e| {
        LaeError::Other(format!(
            "failed to connect to lae daemon at {}: {e}",
            path.display()
        ))
    })?;
    stream
        .set_read_timeout(Some(RPC_TIMEOUT))
        .map_err(|e| LaeError::Other(e.to_string()))?;
    stream
        .set_write_timeout(Some(RPC_TIMEOUT))
        .map_err(|e| LaeError::Other(e.to_string()))?;

    let payload = serde_json::to_string(&json!({ "method": method, "params": params }))
        .map_err(|e| LaeError::Other(e.to_string()))?;
    stream
        .write_all(payload.as_bytes())
        .map_err(|e| LaeError::Other(e.to_string()))?;

    let response = read_response_line(&mut stream)?;
    let parsed: DaemonResponse =
        serde_json::from_str(&response).map_err(|e| LaeError::Other(e.to_string()))?;
    if parsed.ok {
        Ok(parsed.result)
    } else {
        Err(LaeError::Other(
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
            .map_err(|e| LaeError::Other(e.to_string()))?;
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

    fn rpc_or_direct(
        &self,
        method: &str,
        params: Value,
        fallback: impl FnOnce(&TaskService) -> Result<()>,
    ) -> Result<()> {
        if is_daemon_running() {
            daemon_request(method, params).map(|_| ())
        } else {
            fallback(&self.direct)
        }
    }

    pub fn load_state(&self) -> Result<SessionState> {
        if is_daemon_running() {
            let v = daemon_request("get_state", json!({}))?;
            serde_json::from_value(v).map_err(|e| LaeError::Other(e.to_string()))
        } else {
            self.direct.load_state()
        }
    }

    pub fn context_default(&self) -> Result<()> {
        self.rpc_or_direct("context_default", json!({}), |s| s.context_default())
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
        hyprland::switch_workspace(name);
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
        hyprland::switch_workspace(&name);
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

    pub fn create_task(&self, name: &str, switch: bool) -> Result<Task> {
        if is_daemon_running() {
            let v = daemon_request(
                "create_task",
                json!({ "name": name, "switch": switch }),
            )?;
            serde_json::from_value(v).map_err(|e| LaeError::Other(e.to_string()))
        } else {
            self.direct.create_task(name, switch)
        }
    }

    pub fn switch_task(&self, task_id: &str) -> Result<Task> {
        if is_daemon_running() {
            let v = daemon_request("switch_task", json!({ "task_id": task_id }))?;
            serde_json::from_value(v).map_err(|e| LaeError::Other(e.to_string()))
        } else {
            self.direct.switch_task(task_id)
        }
    }

    pub fn archive_task(&self, task_id: &str) -> Result<()> {
        self.rpc_or_direct(
            "archive_task",
            json!({ "task_id": task_id }),
            |s| s.archive_task(task_id),
        )
    }

    pub fn resolve_task(&self, name_or_id: &str) -> Result<Task> {
        if is_daemon_running() {
            let v = daemon_request("resolve_task", json!({ "name_or_id": name_or_id }))?;
            serde_json::from_value(v).map_err(|e| LaeError::Other(e.to_string()))
        } else {
            self.direct.resolve_task(name_or_id)
        }
    }

    pub fn tasks_for_menu(&self) -> Result<Vec<MenuTask>> {
        if is_daemon_running() {
            let v = daemon_request("tasks_for_menu", json!({}))?;
            serde_json::from_value(v).map_err(|e| LaeError::Other(e.to_string()))
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
}

fn spawn_daemon_request(method: &str, params: Value) {
    let method = method.to_string();
    std::thread::spawn(move || {
        let _ = daemon_request(&method, params);
    });
}
