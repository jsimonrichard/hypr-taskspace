//! Unix-socket RPC server — single owner of session state.

use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::atomic::{AtomicBool, Ordering};

static SHUTDOWN: AtomicBool = AtomicBool::new(false);

extern "C" fn handle_shutdown(_: i32) {
    SHUTDOWN.store(true, Ordering::Relaxed);
}
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde_json::{json, Value};

use crate::daemon::client::{daemon_pid_path, daemon_socket_path};
use crate::error::{LaeError, Result};
use crate::service::TaskService;
use crate::xdg::ensure_parent;

pub struct DaemonServer {
    service: Arc<Mutex<TaskService>>,
    stop: Arc<AtomicBool>,
}

impl DaemonServer {
    pub fn new() -> Result<Self> {
        Ok(Self {
            service: Arc::new(Mutex::new(TaskService::with_defaults()?)),
            stop: Arc::new(AtomicBool::new(false)),
        })
    }

    pub fn run_foreground(self) -> Result<()> {
        let socket_path = daemon_socket_path()?;
        ensure_parent(&socket_path)?;
        if socket_path.exists() {
            fs::remove_file(&socket_path).map_err(|source| LaeError::Write {
                path: socket_path.clone(),
                source,
            })?;
        }

        write_pid_file()?;

        let listener = UnixListener::bind(&socket_path).map_err(|source| LaeError::Write {
            path: socket_path.clone(),
            source,
        })?;
        fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o600)).map_err(
            |source| LaeError::Write {
                path: socket_path.clone(),
                source,
            },
        )?;
        listener
            .set_nonblocking(true)
            .map_err(|e| LaeError::Other(e.to_string()))?;

        eprintln!("lae daemon listening on {}", socket_path.display());

        unsafe {
            libc::signal(
                libc::SIGTERM,
                handle_shutdown as *const () as libc::sighandler_t,
            );
            libc::signal(
                libc::SIGINT,
                handle_shutdown as *const () as libc::sighandler_t,
            );
        }

        // Fast DB sync first; Hyprland slot rename runs in the background.
        {
            let svc = self
                .service
                .lock()
                .map_err(|e| LaeError::Other(e.to_string()))?;
            svc.initialize()?;
        }
        let service = self.service.clone();
        thread::spawn(move || {
            if let Ok(svc) = service.lock() {
                if let Err(err) = svc.provision_default_workspaces() {
                    eprintln!("lae daemon: workspace provision: {err}");
                }
            }
        });

        while !self.stop.load(Ordering::Relaxed) && !SHUTDOWN.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((stream, _)) => {
                    let service = self.service.clone();
                    thread::spawn(move || handle_client(stream, service));
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(50));
                }
                Err(_e) if self.stop.load(Ordering::Relaxed) => break,
                Err(e) => {
                    eprintln!("lae daemon accept error: {e}");
                    thread::sleep(Duration::from_millis(200));
                }
            }
        }

        cleanup_runtime_files();
        Ok(())
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn write_pid_file() -> Result<()> {
    let path = daemon_pid_path()?;
    ensure_parent(&path)?;
    fs::write(&path, format!("{}\n", std::process::id())).map_err(|source| LaeError::Write {
        path,
        source,
    })
}

fn cleanup_runtime_files() {
    if let Ok(path) = daemon_socket_path() {
        let _ = fs::remove_file(path);
    }
    if let Ok(path) = daemon_pid_path() {
        let _ = fs::remove_file(path);
    }
}

fn handle_client(mut stream: UnixStream, service: Arc<Mutex<TaskService>>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));

    let request = match read_request(&mut stream) {
        Some(v) => v,
        None => return,
    };

    let method = request
        .get("method")
        .and_then(|m| m.as_str())
        .unwrap_or("");
    let params = request
        .get("params")
        .cloned()
        .unwrap_or_else(|| json!({}));

    let response = match dispatch(service, method, params) {
        Ok(result) => json!({ "ok": true, "result": result }),
        Err(err) => json!({ "ok": false, "error": err.to_string() }),
    };

    let line = match serde_json::to_string(&response) {
        Ok(s) => format!("{s}\n"),
        Err(_) => return,
    };
    let _ = stream.write_all(line.as_bytes());
}

fn read_request(stream: &mut UnixStream) -> Option<Value> {
    let mut buf = Vec::new();
    let mut scratch = [0u8; 65536];
    loop {
        match stream.read(&mut scratch) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&scratch[..n]);
                if buf.contains(&b'\n') {
                    break;
                }
            }
            Err(_) => return None,
        }
    }
    let line = String::from_utf8_lossy(&buf);
    serde_json::from_str(line.lines().next()?.trim()).ok()
}

fn dispatch(
    service: Arc<Mutex<TaskService>>,
    method: &str,
    params: Value,
) -> Result<Value> {
    let svc = service.lock().map_err(|e| LaeError::Other(e.to_string()))?;

    match method {
        "ping" => Ok(json!({ "pong": true })),

        "get_state" => {
            let state = svc.load_state()?;
            Ok(serde_json::to_value(state).map_err(|e| LaeError::Other(e.to_string()))?)
        }

        "context_default" => {
            svc.context_default()?;
            Ok(json!({ "label": svc.taskspace_label()? }))
        }
        "set_context" => {
            let mode = params
                .get("mode")
                .and_then(|m| m.as_str())
                .unwrap_or("default");
            match mode {
                "default" => svc.context_default()?,
                "global" => svc.context_default()?,
                "task" => {
                    let task_id = params
                        .get("task_id")
                        .and_then(|t| t.as_str())
                        .ok_or_else(|| LaeError::Other("task_id required".into()))?;
                    svc.switch_task(task_id)?;
                }
                other => return Err(LaeError::Other(format!("Unknown context mode: {other}"))),
            }
            Ok(json!({ "label": svc.taskspace_label()? }))
        }

        "workspace_go" | "desktop_go" => {
            let relative = params
                .get("relative")
                .and_then(|v| v.as_i64())
                .ok_or_else(|| LaeError::Other("relative required".into()))? as i32;
            let ws = svc.remember_workspace_go(relative)?;
            Ok(json!({ "workspace": ws }))
        }
        "workspace_remember" => {
            let relative = params
                .get("relative")
                .and_then(|v| v.as_i64())
                .ok_or_else(|| LaeError::Other("relative required".into()))? as i32;
            let ws = svc.remember_workspace_go(relative)?;
            Ok(json!({ "workspace": ws }))
        }
        "workspace_remember_goto" => {
            let name = params
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| LaeError::Other("name required".into()))?;
            let ws = svc.remember_workspace_goto(name)?;
            Ok(json!({ "workspace": ws }))
        }
        "workspace_next" | "desktop_next" => {
            let state = svc.load_state()?;
            let relative = crate::workspace_nav::workspace_next_relative(&state)
                .ok_or_else(|| LaeError::Other("no next workspace".into()))?;
            let ws = svc.remember_workspace_go(relative)?;
            Ok(json!({ "workspace": ws }))
        }
        "workspace_prev" | "desktop_prev" => {
            let state = svc.load_state()?;
            let relative = crate::workspace_nav::workspace_prev_relative(&state)
                .ok_or_else(|| LaeError::Other("no previous workspace".into()))?;
            let ws = svc.remember_workspace_go(relative)?;
            Ok(json!({ "workspace": ws }))
        }
        "workspace_goto" | "desktop_goto" => {
            let name = params
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| LaeError::Other("name required".into()))?;
            let ws = svc.remember_workspace_goto(name)?;
            Ok(json!({ "workspace": ws }))
        }

        "create_task" => {
            let name = params
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| LaeError::Other("name required".into()))?;
            let switch = params
                .get("switch")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            let repo_id = params.get("repo_id").and_then(|v| v.as_str());
            let task = svc.create_task(name, switch, repo_id)?;
            Ok(serde_json::to_value(task).map_err(|e| LaeError::Other(e.to_string()))?)
        }
        "switch_task" => {
            let task_id = params
                .get("task_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| LaeError::Other("task_id required".into()))?;
            let task = svc.switch_task(task_id)?;
            Ok(serde_json::to_value(task).map_err(|e| LaeError::Other(e.to_string()))?)
        }
        "archive_task" => {
            let task_id = params
                .get("task_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| LaeError::Other("task_id required".into()))?;
            svc.archive_task(task_id)?;
            Ok(json!({ "archived": task_id }))
        }
        "delete_task" => {
            let task_id = params
                .get("task_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| LaeError::Other("task_id required".into()))?;
            svc.delete_task(task_id)?;
            Ok(json!({ "deleted": task_id }))
        }
        "preview_task_teardown" => {
            let task_id = params
                .get("task_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| LaeError::Other("task_id required".into()))?;
            let preview = svc.preview_task_teardown(task_id)?;
            Ok(serde_json::to_value(preview).map_err(|e| LaeError::Other(e.to_string()))?)
        }
        "resolve_task" => {
            let name_or_id = params
                .get("name_or_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| LaeError::Other("name_or_id required".into()))?;
            let task = svc.resolve_task(name_or_id)?;
            Ok(serde_json::to_value(task).map_err(|e| LaeError::Other(e.to_string()))?)
        }
        "tasks_for_menu" => {
            let items = svc.tasks_for_menu()?;
            Ok(serde_json::to_value(items).map_err(|e| LaeError::Other(e.to_string()))?)
        }
        "taskspace_label" => Ok(json!({ "label": svc.taskspace_label()? })),

        _ => Err(LaeError::Other(format!("Unknown method: {method}"))),
    }
}

/// Stop a running daemon via SIGTERM using the pid file.
pub fn stop_daemon() -> Result<bool> {
    let path = daemon_pid_path()?;
    if !path.is_file() {
        cleanup_runtime_files();
        return Ok(false);
    }
    let pid: i32 = fs::read_to_string(&path)
        .map_err(|source| LaeError::Read {
            path: path.clone(),
            source,
        })?
        .trim()
        .parse()
        .map_err(|_| LaeError::Other("invalid daemon pid file".into()))?;

    unsafe {
        libc::kill(pid, libc::SIGTERM);
    }

    for _ in 0..50 {
        if !path.is_file() {
            return Ok(true);
        }
        thread::sleep(Duration::from_millis(100));
    }

    cleanup_runtime_files();
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatch_ping() {
        let server = DaemonServer::new().unwrap();
        let result = dispatch(server.service.clone(), "ping", json!({})).unwrap();
        assert_eq!(result["pong"], true);
    }
}
