use std::process::Command;

use serde_json::Value;

use crate::error::{LaeError, Result};
use crate::trace::Span;

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

pub fn available() -> bool {
    which::which("hyprctl").is_ok() && has_instance()
}

fn has_instance() -> bool {
    std::env::var("HYPRLAND_INSTANCE_SIGNATURE").is_ok()
        && std::env::var("XDG_RUNTIME_DIR").is_ok()
}

pub fn hyprctl_json(args: &[&str]) -> Result<Value> {
    if !available() {
        return Err(LaeError::HyprlandUnavailable);
    }
    let output = Command::new("hyprctl")
        .arg("-j")
        .args(args)
        .output()
        .map_err(|e| LaeError::Hyprctl(e.to_string()))?;
    if !output.status.success() {
        return Err(LaeError::Hyprctl(
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    if stdout.trim().is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_str(&stdout).map_err(|e| LaeError::Hyprctl(e.to_string()))
}

pub fn dispatch(args: &[&str]) {
    if !available() {
        return;
    }
    let detail = args.join(" ");
    let _span = Span::begin("cli", "hyprland", &format!("dispatch {detail}"));
    let _ = Command::new("hyprctl").arg("dispatch").args(args).status();
}

pub fn get_active_workspace() -> Result<Option<Workspace>> {
    if !available() {
        return Ok(None);
    }
    let data = hyprctl_json(&["activeworkspace"])?;
    parse_workspace_value(&data).map(Some)
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

pub fn switch_workspace(name: &str) {
    if name.chars().all(|c| c.is_ascii_digit()) {
        dispatch(&["workspace", name]);
    } else {
        dispatch(&["workspace", &format!("name:{name}")]);
    }
}

pub fn keyword(args: &[&str]) {
    if !available() {
        return;
    }
    let _ = Command::new("hyprctl").arg("keyword").args(args).status();
}

pub fn rename_workspace(ws_id: i32, name: &str) {
    keyword(&["workspace", &format!("{ws_id},name:{name}")]);
}

pub fn ensure_workspaces(names: &[String]) {
    for name in names {
        if name.chars().all(|c| c.is_ascii_digit()) {
            if let Ok(ws_id) = name.parse::<i32>() {
                rename_workspace(ws_id, name);
            }
        }
        switch_workspace(name);
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
