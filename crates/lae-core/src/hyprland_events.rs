//! Hyprland `.socket2.sock` event listener — instant workspace/window notifications.

use std::io::Read;
use std::os::unix::fs::FileTypeExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::xdg;

pub type EventCallback = Arc<dyn Fn(&str, &str) + Send + Sync>;

#[derive(Debug, Clone)]
pub struct Socket2Diagnostic {
    pub available: bool,
    pub path: Option<PathBuf>,
    pub hyprland_instance_signature: Option<String>,
    pub xdg_runtime_dir: Option<PathBuf>,
    pub candidates: Vec<(PathBuf, String)>,
    pub reason: String,
}

/// Whether `path` is a Hyprland event socket (not a regular file).
pub fn is_socket2_path(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|meta| meta.file_type().is_socket())
        .unwrap_or(false)
}

/// Candidate event-socket paths, newest Hyprland layout first.
pub fn socket2_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let (Ok(sig), Ok(runtime)) = (
        std::env::var("HYPRLAND_INSTANCE_SIGNATURE"),
        xdg::runtime_dir(),
    ) {
        out.push(
            runtime
                .join("hypr")
                .join(sig)
                .join(".socket2.sock"),
        );
    }
    if let Ok(sig) = std::env::var("HYPRLAND_INSTANCE_SIGNATURE") {
        out.push(PathBuf::from("/tmp/hypr").join(sig).join(".socket2.sock"));
    }
    out
}

pub fn socket2_path() -> Option<PathBuf> {
    socket2_candidates()
        .into_iter()
        .find(|path| is_socket2_path(path))
}

pub fn diagnose_socket2() -> Socket2Diagnostic {
    let sig = std::env::var("HYPRLAND_INSTANCE_SIGNATURE").ok();
    let runtime = xdg::runtime_dir().ok();

    let mut candidates = Vec::new();
    for path in socket2_candidates() {
        let status = if is_socket2_path(&path) {
            "socket".into()
        } else if path.exists() {
            "exists but not a socket".into()
        } else {
            "missing".into()
        };
        candidates.push((path, status));
    }

    if sig.is_none() {
        return Socket2Diagnostic {
            available: false,
            path: None,
            hyprland_instance_signature: None,
            xdg_runtime_dir: runtime,
            candidates,
            reason: "HYPRLAND_INSTANCE_SIGNATURE is not set in this process".into(),
        };
    }

    if runtime.is_none() {
        return Socket2Diagnostic {
            available: false,
            path: None,
            hyprland_instance_signature: sig,
            xdg_runtime_dir: None,
            candidates,
            reason: "XDG_RUNTIME_DIR is not set in this process".into(),
        };
    }

    if let Some(path) = socket2_path() {
        return Socket2Diagnostic {
            available: true,
            path: Some(path),
            hyprland_instance_signature: sig,
            xdg_runtime_dir: runtime,
            candidates,
            reason: "ok".into(),
        };
    }

    let reason = if candidates.iter().any(|(_, s)| s == "exists but not a socket") {
        "path exists but is not a Unix socket".into()
    } else {
        "no .socket2.sock found at expected paths (is Hyprland running?)".into()
    };

    Socket2Diagnostic {
        available: false,
        path: None,
        hyprland_instance_signature: sig,
        xdg_runtime_dir: runtime,
        candidates,
        reason,
    }
}

/// Parse `workspacev2` socket2 payload: `WORKSPACEID,WORKSPACENAME`
pub fn parse_workspace_v2(payload: &str) -> Option<(i32, String)> {
    let (id_raw, name) = payload.split_once(',')?;
    let id = id_raw.trim().parse().ok()?;
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    Some((id, name.to_string()))
}

/// Parse `focusedmonv2` payload: `MONITOR,WORKSPACEID`
pub fn parse_focusedmon_v2(payload: &str) -> Option<i32> {
    let (_, id_raw) = payload.split_once(',')?;
    id_raw.trim().parse().ok()
}

/// Uses `workspacev2` payload only: `WORKSPACEID,WORKSPACENAME`.
pub fn is_workspace_focus_event(event: &str) -> bool {
    event == "workspacev2"
}

/// Monitor focus carries workspace id — same fast path as native Waybar.
pub fn is_monitor_focus_event(event: &str) -> bool {
    matches!(event, "focusedmon" | "focusedmonv2")
}

/// Events that need a full refresh (occupied slots, visibility, task label).
pub fn is_full_refresh_event(event: &str) -> bool {
    matches!(
        event,
        "openwindow"
            | "closewindow"
            | "movewindow"
            | "movewindowv2"
            | "createworkspace"
            | "createworkspacev2"
            | "destroyworkspace"
            | "destroyworkspacev2"
    )
}

pub struct HyprlandEventListener {
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl HyprlandEventListener {
    pub fn start(on_event: EventCallback) -> Option<Self> {
        let path = socket2_path()?;
        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = stop.clone();
        let handle = thread::spawn(move || run_listener(&path, stop_flag, on_event));
        Some(Self {
            stop,
            thread: Some(handle),
        })
    }
}

impl Drop for HyprlandEventListener {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

fn run_listener(path: &std::path::Path, stop: Arc<AtomicBool>, on_event: EventCallback) {
    while !stop.load(Ordering::Relaxed) {
        match UnixStream::connect(path) {
            Ok(mut stream) => {
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                read_events(&mut stream, &stop, &on_event);
            }
            Err(_) => {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                thread::sleep(Duration::from_secs(1));
            }
        }
    }
}

fn read_events(stream: &mut UnixStream, stop: &AtomicBool, on_event: &EventCallback) {
    let mut buf = String::new();
    let mut scratch = [0u8; 4096];
    while !stop.load(Ordering::Relaxed) {
        match stream.read(&mut scratch) {
            Ok(0) => break,
            Ok(n) => {
                buf.push_str(&String::from_utf8_lossy(&scratch[..n]));
                while let Some(pos) = buf.find('\n') {
                    let line = buf[..pos].trim().to_string();
                    buf.drain(..=pos);
                    parse_line(&line, on_event);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
            Err(_) => break,
        }
    }
}

fn parse_line(line: &str, on_event: &EventCallback) {
    if line.is_empty() || !line.contains(">>") {
        return;
    }
    let Some((event, payload)) = line.split_once(">>") else {
        return;
    };
    on_event(event.trim(), payload.trim());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_focus_events_use_workspace_v2() {
        assert!(is_workspace_focus_event("workspacev2"));
        assert!(!is_full_refresh_event("workspacev2"));
    }

    #[test]
    fn parse_workspace_v2_payload() {
        assert_eq!(
            parse_workspace_v2("3,code"),
            Some((3, "code".into()))
        );
        assert_eq!(
            parse_workspace_v2("2,2"),
            Some((2, "2".into()))
        );
    }

    #[test]
    fn unix_socket_is_not_is_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.sock");
        let _listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        assert!(path.exists());
        assert!(!path.is_file(), "Unix sockets must not pass Path::is_file()");
        assert!(is_socket2_path(&path));
    }
}
