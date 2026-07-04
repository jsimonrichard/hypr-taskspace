//! State change notifications for immediate Waybar updates.
//!
//! Any process that mutates `state.db` should call [`publish`] after a successful write.
//! The Waybar CFFI module binds [`state_events_socket_path`] and listens for events;
//! [`read_state_rev`] provides a fallback when the socket is unavailable.

use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::waybar::notify_waybar;
use crate::xdg::{ensure_parent, tsk_runtime_dir};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StateChangeKind {
    /// Taskspace / context mode changed (label + workspace strip).
    Taskspace,
    /// Active workspace or last-workspace map changed.
    Workspace,
    /// Full bar rebuild (tasks added/removed, config, etc.).
    Full,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct StateEvent {
    kind: StateChangeKind,
    rev: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    workspace: Option<String>,
}

pub fn state_events_socket_path() -> Result<PathBuf, crate::error::TskError> {
    Ok(tsk_runtime_dir()?.join("state-events.sock"))
}

fn state_rev_path() -> Result<PathBuf, crate::error::TskError> {
    Ok(tsk_runtime_dir()?.join("state.rev"))
}

/// Monotonic counter bumped on every [`publish`]; survives socket unavailability.
pub fn read_state_rev() -> u64 {
    state_rev_path()
        .ok()
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

fn bump_state_rev() -> u64 {
    let rev = read_state_rev().wrapping_add(1).max(1);
    if let Ok(path) = state_rev_path() {
        let _ = ensure_parent(&path);
        let tmp = path.with_extension("tmp");
        if fs::write(&tmp, format!("{rev}\n")).is_ok() {
            let _ = fs::rename(&tmp, &path);
        }
    }
    rev
}

/// Notify subscribers that session state changed after `state.db` was written.
pub fn publish(kind: StateChangeKind) {
    publish_with_workspace(kind, None);
}

pub fn publish_with_workspace(kind: StateChangeKind, workspace: Option<&str>) {
    let rev = bump_state_rev();
    let event = StateEvent {
        kind,
        rev,
        workspace: workspace.map(str::to_string),
    };
    let _ = send_event(&event);
    notify_waybar();
}

fn send_event(event: &StateEvent) -> std::io::Result<()> {
    let path = state_events_socket_path()
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::NotFound))?;
    let mut stream = UnixStream::connect(path)?;
    let line = serde_json::to_string(event)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    stream.write_all(format!("{line}\n").as_bytes())?;
    stream.flush()?;
    Ok(())
}

pub type StateEventCallback = Arc<dyn Fn(StateChangeKind, u64, Option<String>) + Send + Sync>;

/// Binds the state-events socket (Waybar module). CLI tools publish by connecting.
pub struct StateEventListener {
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl StateEventListener {
    pub fn start(on_event: StateEventCallback) -> Option<Self> {
        let path = state_events_socket_path().ok()?;
        ensure_parent(&path).ok()?;
        if path.exists() {
            let _ = fs::remove_file(&path);
        }
        let listener = UnixListener::bind(&path).ok()?;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
        listener.set_nonblocking(true).ok()?;

        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = stop.clone();
        let handle = thread::spawn(move || run_server(listener, stop_flag, on_event));
        Some(Self {
            stop,
            thread: Some(handle),
        })
    }
}

impl Drop for StateEventListener {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
        if let Ok(path) = state_events_socket_path() {
            let _ = fs::remove_file(path);
        }
    }
}

fn run_server(listener: UnixListener, stop: Arc<AtomicBool>, on_event: StateEventCallback) {
    while !stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((mut stream, _)) => {
                let _ = stream.set_read_timeout(Some(Duration::from_millis(200)));
                if let Some(event) = read_one_event(&mut stream) {
                    on_event(event.kind, event.rev, event.workspace);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(50));
            }
            Err(_) if stop.load(Ordering::Relaxed) => break,
            Err(_) => thread::sleep(Duration::from_millis(200)),
        }
    }
}

fn read_one_event(stream: &mut UnixStream) -> Option<StateEvent> {
    let mut buf = String::new();
    let mut scratch = [0u8; 512];
    loop {
        match stream.read(&mut scratch) {
            Ok(0) => break,
            Ok(n) => {
                buf.push_str(&String::from_utf8_lossy(&scratch[..n]));
                if buf.contains('\n') {
                    break;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(_) => return None,
        }
    }
    let line = buf.lines().next()?.trim();
    serde_json::from_str(line).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_event_json_roundtrip() {
        let event = StateEvent {
            kind: StateChangeKind::Taskspace,
            rev: 7,
            workspace: None,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert_eq!(
            serde_json::from_str::<StateEvent>(&json).unwrap(),
            event
        );
    }
}
