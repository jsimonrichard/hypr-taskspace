//! Hyprland IPC command log — every `hyprctl` invocation and why it ran.
//!
//! - Log file: `$XDG_RUNTIME_DIR/lae/hyprctl.log` (override with `LAE_HYPR_LOG_FILE`)
//! - Disable with `LAE_HYPR_LOG=0`
//! - Inspect: `lae debug hypr log show`

use std::cell::RefCell;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

thread_local! {
    static REASON_STACK: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

static WRITER: Mutex<()> = Mutex::new(());

pub fn enabled() -> bool {
    static ENV: OnceLock<bool> = OnceLock::new();
    *ENV.get_or_init(|| {
        std::env::var("LAE_HYPR_LOG")
            .map(|v| !matches!(v.to_ascii_lowercase().as_str(), "0" | "false" | "no" | "off"))
            .unwrap_or(true)
    })
}

pub fn log_path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("LAE_HYPR_LOG_FILE") {
        return Some(PathBuf::from(path));
    }
    crate::xdg::lae_runtime_dir()
        .ok()
        .map(|dir| dir.join("hyprctl.log"))
}

pub fn hypr_log_path() -> Option<PathBuf> {
    log_path()
}

pub fn current_reason() -> String {
    REASON_STACK.with(|stack| {
        let stack = stack.borrow();
        if stack.is_empty() {
            "(no reason)".into()
        } else {
            stack.join(" → ")
        }
    })
}

pub fn push_reason(reason: impl Into<String>) {
    REASON_STACK.with(|stack| stack.borrow_mut().push(reason.into()));
}

pub fn pop_reason() {
    REASON_STACK.with(|stack| {
        stack.borrow_mut().pop();
    });
}

struct ReasonGuard;

impl Drop for ReasonGuard {
    fn drop(&mut self) {
        pop_reason();
    }
}

/// Push a reason for the duration of `f` (supports nesting).
pub fn scoped<R>(reason: impl Into<String>, f: impl FnOnce() -> R) -> R {
    push_reason(reason);
    let _guard = ReasonGuard;
    f()
}

pub fn log(kind: &str, command: &str) {
    if !enabled() {
        return;
    }
    let Some(path) = log_path() else {
        return;
    };
    let reason = current_reason();
    let wall = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let pid = std::process::id();
    let line = format!("{wall} pid={pid} [{kind}] {command} | {reason}\n");
    let _guard = WRITER.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = file.write_all(line.as_bytes());
    }
}

/// Log a navigation decision that does not invoke hyprctl directly.
pub fn note(message: impl AsRef<str>) {
    log("note", message.as_ref());
}

pub fn clear_log() -> crate::error::Result<()> {
    let Some(path) = log_path() else {
        return Ok(());
    };
    if path.is_file() {
        fs::remove_file(&path).map_err(|source| crate::error::LaeError::Write {
            path: path.clone(),
            source,
        })?;
    }
    Ok(())
}

pub fn tail_raw(limit: usize) -> crate::error::Result<String> {
    let Some(path) = log_path() else {
        return Ok(String::new());
    };
    if !path.is_file() {
        return Ok(String::new());
    }
    let raw = fs::read_to_string(&path).map_err(|source| crate::error::LaeError::Read {
        path: path.clone(),
        source,
    })?;
    Ok(raw
        .lines()
        .rev()
        .take(limit)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reason_stack_nests() {
        scoped("outer", || {
            assert!(current_reason().contains("outer"));
            scoped("inner", || {
                assert_eq!(current_reason(), "outer → inner");
            });
            assert_eq!(current_reason(), "outer");
        });
        assert_eq!(current_reason(), "(no reason)");
    }
}
