use std::cell::Cell;
use std::collections::HashSet;
use std::io::ErrorKind;
use std::process::{Command, ExitStatus, Stdio};
use std::sync::Mutex;

use serde_json::Value;

use crate::error::{TskError, Result};
use crate::hypr_log;
use crate::terminal::{TUI_WINDOW_CLASS, TUI_WINDOW_TITLE};
use crate::trace::Span;

/// Serialize all hyprctl IPC — the daemon handles requests on multiple threads.
static HYPR_IPC: Mutex<()> = Mutex::new(());

thread_local! {
    static HYPR_IPC_DEPTH: Cell<u32> = const { Cell::new(0) };
}

fn with_hypr_ipc<R>(f: impl FnOnce() -> R) -> R {
    let depth = HYPR_IPC_DEPTH.with(|d| d.get());
    if depth > 0 {
        return f();
    }
    let _guard = HYPR_IPC.lock().unwrap_or_else(|e| e.into_inner());
    HYPR_IPC_DEPTH.with(|d| d.set(1));
    let result = f();
    HYPR_IPC_DEPTH.with(|d| d.set(0));
    result
}

/// Hyprland dispatch target for a workspace **name** (including default slots `"1"`…`"10"`).
/// Numeric names must use `name:` — bare `3` is a per-monitor index, not workspace `"3"`.
pub fn workspace_dispatch_arg(name: &str) -> String {
    format!("name:{name}")
}

#[derive(Debug, Clone)]
pub struct HyprWindow {
    pub address: String,
    pub title: String,
    pub class_name: String,
    pub workspace: i32,
    pub workspace_name: String,
    pub pid: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspace {
    pub id: i32,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Monitor {
    pub name: String,
    pub workspace_name: String,
    pub focused: bool,
    /// Top-left corner in the compositor layout (used for stable monitor ordering).
    pub x: i32,
    pub y: i32,
}

pub fn available() -> bool {
    which::which("hyprctl").is_ok() && has_instance()
}

/// Whether hyprctl dispatches may mutate the live compositor (disabled under `cfg(test)`).
pub fn mutations_enabled() -> bool {
    if cfg!(test) {
        return false;
    }
    !matches!(
        std::env::var("TSK_DISABLE_HYPRLAND").as_deref(),
        Ok("1") | Ok("true") | Ok("TRUE")
    )
}

fn has_instance() -> bool {
    std::env::var("HYPRLAND_INSTANCE_SIGNATURE").is_ok()
        && std::env::var("XDG_RUNTIME_DIR").is_ok()
}

pub fn hyprctl_json(args: &[&str]) -> Result<Value> {
    if !available() {
        return Err(TskError::HyprlandUnavailable);
    }
    with_hypr_ipc(|| {
        hypr_log::log("query", &format!("hyprctl -j {}", args.join(" ")));
        let output = retry_on_interrupted(|| {
            Command::new("hyprctl")
                .arg("-j")
                .args(args)
                .output()
        })
        .map_err(|e| TskError::Hyprctl(e.to_string()))?;
        if !output.status.success() {
            return Err(TskError::Hyprctl(
                String::from_utf8_lossy(&output.stderr).into_owned(),
            ));
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        if stdout.trim().is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_str(&stdout).map_err(|e| TskError::Hyprctl(e.to_string()))
    })
}

fn retry_on_interrupted<T, F>(mut f: F) -> std::io::Result<T>
where
    F: FnMut() -> std::io::Result<T>,
{
    loop {
        match f() {
            Ok(value) => return Ok(value),
            Err(err) if err.kind() == ErrorKind::Interrupted => continue,
            Err(err) => return Err(err),
        }
    }
}

fn hyprctl_status(args: &[&str]) -> std::io::Result<ExitStatus> {
    hypr_log::log("dispatch", &format!("hyprctl {}", args.join(" ")));
    retry_on_interrupted(|| {
        Command::new("hyprctl")
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
    })
}

pub fn dispatch(args: &[&str]) {
    dispatch_sync(args);
}

/// Fire-and-forget — provisioning only; not used for interactive workspace switches.
pub fn dispatch_async(args: &[&str]) {
    if !available() || !mutations_enabled() {
        return;
    }
    let detail = args.join(" ");
    hypr_log::log("dispatch_async", &format!("hyprctl dispatch {detail}"));
    let _span = Span::begin("cli", "hyprland", &format!("dispatch {detail}"));
    let _ = Command::new("hyprctl")
        .arg("dispatch")
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

pub fn dispatch_sync(args: &[&str]) {
    if !available() || !mutations_enabled() {
        return;
    }
    with_hypr_ipc(|| {
        let detail = args.join(" ");
        let _span = Span::begin("cli", "hyprland", &format!("dispatch {detail}"));
        let mut hypr_args = vec!["dispatch"];
        hypr_args.extend(args);
        let _ = hyprctl_status(&hypr_args);
    });
}

pub fn get_active_workspace() -> Result<Option<Workspace>> {
    if !available() {
        return Ok(None);
    }
    let data = hyprctl_json(&["activeworkspace"])?;
    parse_workspace_value(&data).map(Some)
}

pub fn list_workspaces() -> Result<Vec<Workspace>> {
    if !available() {
        return Ok(Vec::new());
    }
    let data = hyprctl_json(&["workspaces"])?;
    let Some(items) = data.as_array() else {
        return Ok(Vec::new());
    };
    Ok(items
        .iter()
        .filter_map(|item| parse_workspace_value(item).ok())
        .collect())
}

pub fn get_clients() -> Result<Vec<HyprWindow>> {
    let data = hyprctl_json(&["clients"])?;
    let Some(items) = data.as_array() else {
        return Ok(Vec::new());
    };
    Ok(items.iter().filter_map(parse_client).collect())
}

fn parse_workspace_value(data: &Value) -> Result<Workspace> {
    Ok(Workspace {
        id: data.get("id").and_then(|v| v.as_i64()).unwrap_or(0) as i32,
        name: data
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .into(),
    })
}

fn parse_client(item: &Value) -> Option<HyprWindow> {
    let workspace = item.get("workspace")?;
    let (ws_id, ws_name) = if workspace.is_object() {
        (
            workspace.get("id")?.as_i64()? as i32,
            workspace
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .into(),
        )
    } else {
        (workspace.as_i64()? as i32, String::new())
    };
    Some(HyprWindow {
        address: item.get("address")?.as_str()?.into(),
        title: item
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .into(),
        class_name: item
            .get("class")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .into(),
        workspace: ws_id,
        workspace_name: ws_name,
        pid: item.get("pid").and_then(|v| v.as_i64()).map(|v| v as i32),
    })
}

pub fn list_monitors() -> Result<Vec<Monitor>> {
    if !available() {
        return Ok(Vec::new());
    }
    let data = hyprctl_json(&["monitors"])?;
    let Some(items) = data.as_array() else {
        return Ok(Vec::new());
    };
    Ok(items
        .iter()
        .filter_map(|item| {
            let name = item.get("name")?.as_str()?.to_string();
            let ws = item.get("activeWorkspace")?;
            let workspace_name = ws
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let focused = item.get("focused").and_then(|v| v.as_bool()).unwrap_or(false);
            let x = item.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let y = item.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            Some(Monitor {
                name,
                workspace_name,
                focused,
                x,
                y,
            })
        })
        .collect())
}

pub fn focus_monitor(name: &str) {
    hypr_log::scoped(format!("focus_monitor {name}"), || {
        dispatch_sync(&["focusmonitor", name]);
    });
}

pub fn swap_active_workspaces(monitor_a: &str, monitor_b: &str) {
    hypr_log::scoped(format!("swap_active_workspaces {monitor_a} ↔ {monitor_b}"), || {
        dispatch_sync(&["swapactiveworkspaces", monitor_a, monitor_b]);
    });
}

pub fn focused_monitor_name() -> Option<String> {
    list_monitors()
        .ok()
        .and_then(|monitors| monitors.into_iter().find(|m| m.focused).map(|m| m.name))
}

/// Close floating task-manager terminals (they belong to the taskspace being left).
pub fn close_tsk_tui_windows() -> usize {
    if !available() {
        return 0;
    }
    hypr_log::scoped("close_tsk_tui_windows", || {
        let Ok(clients) = get_clients() else {
            return 0;
        };
        let mut closed = 0;
        for client in clients {
            if client.title == TUI_WINDOW_TITLE || client.class_name == TUI_WINDOW_CLASS {
                close_window(&client.address);
                closed += 1;
            }
        }
        closed
    })
}

pub fn switch_workspace_on_monitor(monitor: &str, workspace: &str) {
    hypr_log::scoped(format!("switch_workspace_on_monitor {monitor} → {workspace}"), || {
        focus_monitor(monitor);
        switch_workspace_on_current_monitor(workspace);
    });
}

pub fn switch_workspace_on_current_monitor(name: &str) {
    let target = workspace_dispatch_arg(name);
    hypr_log::scoped(format!("focusworkspaceoncurrentmonitor {target}"), || {
        dispatch_sync(&["focusworkspaceoncurrentmonitor", &target]);
    });
}

/// Switch to a workspace globally (may focus the monitor where it last appeared).
/// Use for provisioning; prefer [`switch_workspace_for_navigation`] for user navigation.
pub fn switch_workspace(name: &str) {
    let target = workspace_dispatch_arg(name);
    hypr_log::scoped(format!("workspace {target}"), || {
        dispatch_sync(&["workspace", &target]);
    });
}

/// Switch to a workspace on the **focused** monitor (moves it between monitors).
pub fn switch_workspace_on_focused_monitor(name: &str) {
    switch_workspace_on_current_monitor(name);
}

/// Within-taskspace navigation: if the workspace is already visible on another
/// monitor, focus that monitor; otherwise bring the workspace to the focused one.
pub fn switch_workspace_for_navigation(name: &str) {
    hypr_log::scoped(format!("switch_workspace_for_navigation {name}"), || {
        match navigation_strategy(name) {
            NavigationStrategy::FocusExistingMonitor => {
                hypr_log::note(format!("workspace {name} visible on another monitor — focusing it"));
                switch_workspace(name);
            }
            NavigationStrategy::OnFocusedMonitor => {
                switch_workspace_on_current_monitor(name);
            }
            NavigationStrategy::MoveToFocusedMonitor => {
                hypr_log::note(format!("workspace {name} not visible — moving to focused monitor"));
                switch_workspace_on_focused_monitor(name);
            }
        }
    });
}

/// Move the active window to a workspace by **name** (Omarchy SUPER+SHIFT+number).
pub fn move_active_window_to_workspace(name: &str) {
    let target = workspace_dispatch_arg(name);
    hypr_log::scoped(format!("move_active_window_to_workspace {name}"), || {
        dispatch_sync(&["movetoworkspace", &target]);
    });
}

/// Move a specific window without changing the active workspace.
pub fn move_window_to_workspace_silent(address: &str, name: &str) {
    if !available() {
        return;
    }
    let addr = address.strip_prefix("0x").unwrap_or(address);
    let target = workspace_dispatch_arg(name);
    hypr_log::scoped(format!("move_window_to_workspace_silent 0x{addr} → {name}"), || {
        dispatch_sync(&[
            "movetoworkspacesilent",
            &target,
            &format!("address:0x{addr}"),
        ]);
    });
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NavigationStrategy {
    FocusExistingMonitor,
    OnFocusedMonitor,
    MoveToFocusedMonitor,
}

fn navigation_strategy(name: &str) -> NavigationStrategy {
    decide_navigation_strategy(
        &list_monitors().unwrap_or_default(),
        name,
    )
}

fn decide_navigation_strategy(monitors: &[Monitor], name: &str) -> NavigationStrategy {
    let Some(holder) = monitors.iter().find(|m| m.workspace_name == name) else {
        return NavigationStrategy::MoveToFocusedMonitor;
    };
    if holder.focused {
        NavigationStrategy::OnFocusedMonitor
    } else {
        NavigationStrategy::FocusExistingMonitor
    }
}

pub fn close_window(address: &str) {
    if !available() {
        return;
    }
    let addr = address.strip_prefix("0x").unwrap_or(address);
    hypr_log::scoped(format!("close_window 0x{addr}"), || {
        dispatch_sync(&["closewindow", &format!("address:0x{addr}")]);
    });
}

pub fn focus_window(address: &str) {
    if !available() {
        return;
    }
    let addr = address.strip_prefix("0x").unwrap_or(address);
    hypr_log::scoped(format!("focus_window 0x{addr}"), || {
        dispatch_sync(&["focuswindow", &format!("address:0x{addr}")]);
    });
}

pub fn keyword(args: &[&str]) {
    if !available() {
        return;
    }
    with_hypr_ipc(|| {
        hypr_log::log("keyword", &format!("hyprctl keyword {}", args.join(" ")));
        let _ = Command::new("hyprctl")
            .arg("keyword")
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    });
}

pub fn rename_workspace(ws_id: i32, name: &str) {
    hypr_log::scoped(format!("rename_workspace id={ws_id} name={name}"), || {
        keyword(&["workspace", &format!("{ws_id},name:{name}")]);
    });
}

pub fn ensure_workspaces(names: &[String]) {
    if names.is_empty() {
        return;
    }

    hypr_log::scoped(format!("ensure_workspaces ({} names)", names.len()), || {
        ensure_workspaces_inner(names);
    });
}

fn ensure_workspaces_inner(names: &[String]) {
    let previous = get_active_workspace()
        .ok()
        .flatten()
        .map(|ws| ws.name)
        .filter(|n| !n.is_empty());

    let existing: HashSet<String> = list_workspaces()
        .ok()
        .map(|workspaces| workspaces.into_iter().map(|ws| ws.name).collect())
        .unwrap_or_default();

    let mut created_named = false;
    for name in names {
        if name.chars().all(|c| c.is_ascii_digit()) {
            if let Ok(ws_id) = name.parse::<i32>() {
                rename_workspace(ws_id, name);
            }
            continue;
        }
        if existing.contains(name) {
            continue;
        }
        // Create at most one named workspace here; others appear on first navigation.
        if !created_named {
            hypr_log::scoped(format!("create named workspace {name}"), || {
                switch_workspace(name);
            });
            created_named = true;
        }
    }

    if let Some(prev) = previous {
        let active = get_active_workspace()
            .ok()
            .flatten()
            .map(|ws| ws.name)
            .unwrap_or_default();
        // Only restore when the previous workspace is in the set we are provisioning.
        // Restoring across taskspaces emits a `workspacev2` that the daemon later treats
        // as an external switch (bounce / feedback loop). Callers that need to stay put
        // while creating outside the current taskspace should save/restore themselves.
        if active != prev && workspace_in_provision_set(&prev, names) {
            hypr_log::scoped(format!("restore active workspace after ensure: {prev}"), || {
                switch_workspace(&prev);
            });
        }
    }
}

fn workspace_in_provision_set(name: &str, names: &[String]) -> bool {
    if names.iter().any(|n| n == name) {
        return true;
    }
    name.parse::<u32>()
        .ok()
        .map(|n| n.to_string())
        .is_some_and(|numeric| names.iter().any(|n| n == &numeric))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_in_provision_set_matches_names_and_numeric() {
        let names = vec!["1".into(), "2".into(), "auth-1".into()];
        assert!(workspace_in_provision_set("1", &names));
        assert!(workspace_in_provision_set("auth-1", &names));
        assert!(!workspace_in_provision_set("3", &names));
        assert!(!workspace_in_provision_set("other-1", &names));
    }

    #[test]
    fn workspace_dispatch_arg_uses_name_prefix_for_numeric_slots() {
        assert_eq!(workspace_dispatch_arg("3"), "name:3");
        assert_eq!(workspace_dispatch_arg("auth-fix-2"), "name:auth-fix-2");
    }

    #[test]
    fn navigation_strategy_when_visible_on_other_monitor() {
        let monitors = vec![
            Monitor {
                name: "eDP-1".into(),
                workspace_name: "1".into(),
                focused: true,
                x: 0,
                y: 0,
            },
            Monitor {
                name: "DP-2".into(),
                workspace_name: "3".into(),
                focused: false,
                x: 1920,
                y: 0,
            },
        ];
        assert_eq!(
            decide_navigation_strategy(&monitors, "3"),
            NavigationStrategy::FocusExistingMonitor
        );
    }

    #[test]
    fn navigation_strategy_when_not_visible_moves_to_focused() {
        let monitors = vec![
            Monitor {
                name: "eDP-1".into(),
                workspace_name: "1".into(),
                focused: false,
                x: 0,
                y: 0,
            },
            Monitor {
                name: "DP-2".into(),
                workspace_name: "2".into(),
                focused: true,
                x: 1920,
                y: 0,
            },
        ];
        assert_eq!(
            decide_navigation_strategy(&monitors, "3"),
            NavigationStrategy::MoveToFocusedMonitor
        );
    }

    #[test]
    fn navigation_strategy_when_on_focused_monitor() {
        let monitors = vec![Monitor {
            name: "eDP-1".into(),
            workspace_name: "3".into(),
            focused: true,
            x: 0,
            y: 0,
        }];
        assert_eq!(
            decide_navigation_strategy(&monitors, "3"),
            NavigationStrategy::OnFocusedMonitor
        );
    }
}

// `which` helper — avoids an extra dependency
mod which {
    use std::path::PathBuf;

    pub fn which(name: &str) -> Result<PathBuf, ()> {
        std::env::split_paths(&std::env::var_os("PATH").ok_or(())?)
            .find_map(|dir| {
                let path = dir.join(name);
                path.is_file().then_some(path)
            })
            .ok_or(())
    }
}
