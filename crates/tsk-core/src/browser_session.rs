//! Persist Chromium tabs per task (shared host profile, tsk-owned session).

use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::browser::{is_browser_class, open_new_window_with_urls, task_chromium_profile_dir};
use crate::config::TskConfig;
use crate::error::{Result, TskError};
use crate::hyprland::{self, HyprWindow};
use crate::models::{ContextMode, SessionState, Task, TaskStatus};
use crate::task_cleanup::{clients_for_task, task_data_dir};
use crate::window_registry::client_workspace_name;
use crate::workspaces::primary_task_workspace;
use crate::xdg::ensure_parent;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LiveTab {
    pub url: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LiveWindow {
    pub id: i64,
    #[serde(default)]
    pub focused: bool,
    #[serde(default)]
    pub tabs: Vec<LiveTab>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LiveWindows {
    pub updated_at: DateTime<Utc>,
    pub windows: Vec<LiveWindow>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionWindow {
    pub workspace: String,
    pub urls: Vec<String>,
    #[serde(default)]
    pub title: String,
}

fn default_pending() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskBrowserSession {
    pub task_id: String,
    pub saved_at: DateTime<Utc>,
    pub windows: Vec<SessionWindow>,
    /// Waiting for the first Chromium launch in this taskspace after archive.
    #[serde(default = "default_pending")]
    pub pending: bool,
}

pub fn live_windows_path(cfg: &TskConfig) -> PathBuf {
    cfg.data_dir.join("chromium/live-windows.json")
}

pub fn session_path(cfg: &TskConfig, task_id: &str) -> PathBuf {
    task_data_dir(cfg, task_id).join(".tsk/browser-session.json")
}

pub fn ingest_native_message(cfg: &TskConfig, raw: &[u8]) -> Result<()> {
    let value: Value = serde_json::from_slice(raw)
        .map_err(|e| TskError::Other(format!("native host JSON: {e}")))?;
    if value.get("op").and_then(|v| v.as_str()) != Some("windows") {
        return Ok(());
    }
    let windows: Vec<LiveWindow> = match value.get("windows") {
        Some(w) => serde_json::from_value(w.clone())
            .map_err(|e| TskError::Other(format!("native host windows: {e}")))?,
        None => Vec::new(),
    };
    let live = LiveWindows {
        updated_at: Utc::now(),
        windows,
    };
    write_live_windows(cfg, &live)?;
    if let Err(err) = refresh_snapshots_from_live(cfg, &live) {
        eprintln!("tsk chromium-host: snapshot: {err}");
    }
    Ok(())
}

pub fn write_live_windows(cfg: &TskConfig, live: &LiveWindows) -> Result<()> {
    let path = live_windows_path(cfg);
    ensure_parent(&path)?;
    let body = serde_json::to_string_pretty(live)
        .map_err(|e| TskError::Other(format!("live windows JSON: {e}")))?;
    fs::write(&path, format!("{body}\n")).map_err(|source| TskError::Write { path, source })
}

pub fn read_live_windows(cfg: &TskConfig) -> Result<Option<LiveWindows>> {
    read_json(&live_windows_path(cfg))
}

pub fn read_task_session(cfg: &TskConfig, task_id: &str) -> Result<Option<TaskBrowserSession>> {
    read_json(&session_path(cfg, task_id))
}

pub fn capture_and_save(cfg: &TskConfig, task: &Task) -> Result<TaskBrowserSession> {
    hyprland::ensure_instance_env();
    let captured = capture_for_task(cfg, task)?;
    let windows = if captured.windows.is_empty() {
        read_task_session(cfg, &task.id)?
            .map(|s| s.windows)
            .unwrap_or_default()
    } else {
        captured.windows
    };
    let session = TaskBrowserSession {
        task_id: task.id.clone(),
        saved_at: Utc::now(),
        windows,
        pending: true,
    };
    save_task_session(cfg, &session)?;
    Ok(session)
}

/// Update per-task session files from the latest live window list.
pub fn refresh_snapshots_from_live(cfg: &TskConfig, live: &LiveWindows) -> Result<()> {
    hyprland::ensure_instance_env();
    let registry = crate::registry::Registry::new(None, cfg.clone())?;
    let state = registry.load_state()?;
    for task in state.tasks.values() {
        if task.status == TaskStatus::Archived {
            continue;
        }
        if read_task_session(cfg, &task.id)?.is_some_and(|s| s.pending) {
            continue;
        }
        let windows = snapshot_windows_for_task(cfg, &state, task, live)?;
        if windows.is_empty() {
            continue;
        }
        save_task_session(
            cfg,
            &TaskBrowserSession {
                task_id: task.id.clone(),
                saved_at: Utc::now(),
                windows,
                pending: false,
            },
        )?;
    }
    Ok(())
}

fn snapshot_windows_for_task(
    cfg: &TskConfig,
    state: &SessionState,
    task: &Task,
    live: &LiveWindows,
) -> Result<Vec<SessionWindow>> {
    let hypr = clients_for_task(cfg, task)?
        .into_iter()
        .filter(|c| is_browser_class(&c.class_name))
        .collect::<Vec<_>>();
    let mut windows = assign_session_windows(&hypr, &live.windows);
    if windows.is_empty() {
        windows = fallback_current_task_windows(state, task, live);
    }
    Ok(windows)
}

fn fallback_current_task_windows(
    state: &SessionState,
    task: &Task,
    live: &LiveWindows,
) -> Vec<SessionWindow> {
    if state.context_mode != ContextMode::Task
        || state.current_task_id.as_deref() != Some(task.id.as_str())
    {
        return Vec::new();
    }
    let workspace = primary_task_workspace(
        &task.id,
        state.default_workspace_count,
        &state.global_workspace_slots,
    );
    if live.windows.len() == 1 {
        return vec![session_window_from_live(&workspace, &live.windows[0])];
    }
    live.windows
        .iter()
        .find(|w| w.focused)
        .map(|w| vec![session_window_from_live(&workspace, w)])
        .unwrap_or_default()
}

pub fn save_task_session(cfg: &TskConfig, session: &TaskBrowserSession) -> Result<()> {
    let path = session_path(cfg, &session.task_id);
    ensure_parent(&path)?;
    let body = serde_json::to_string_pretty(session)
        .map_err(|e| TskError::Other(format!("browser session JSON: {e}")))?;
    fs::write(&path, format!("{body}\n")).map_err(|source| TskError::Write { path, source })
}

pub fn capture_for_task(cfg: &TskConfig, task: &Task) -> Result<TaskBrowserSession> {
    let hypr = clients_for_task(cfg, task)?
        .into_iter()
        .filter(|c| is_browser_class(&c.class_name))
        .collect::<Vec<_>>();
    let live = read_live_windows(cfg)?.unwrap_or(LiveWindows {
        updated_at: Utc::now(),
        windows: Vec::new(),
    });
    Ok(TaskBrowserSession {
        task_id: task.id.clone(),
        saved_at: Utc::now(),
        windows: assign_session_windows(&hypr, &live.windows),
        pending: true,
    })
}

/// Reopen a saved session (CLI / tests). Marks it consumed so Walker will not
/// open the same windows again.
pub fn restore_saved(cfg: &TskConfig, task: &Task) -> Result<usize> {
    let Some(session) = read_task_session(cfg, &task.id)? else {
        return Ok(0);
    };
    if session.windows.iter().all(|w| w.urls.is_empty()) {
        return Ok(0);
    }
    let opened = restore_session(cfg, task, &session)?;
    mark_session_consumed(cfg, &task.id)?;
    Ok(opened)
}

/// First Chromium launch with no task window: reopen the last saved tabs.
pub fn restore_pending(cfg: &TskConfig, task: &Task) -> Result<usize> {
    let Some(session) = read_task_session(cfg, &task.id)? else {
        return Ok(0);
    };
    if session.windows.iter().all(|w| w.urls.is_empty()) {
        return Ok(0);
    }
    let opened = restore_session(cfg, task, &session)?;
    if opened > 0 {
        mark_session_consumed(cfg, &task.id)?;
    }
    Ok(opened)
}

pub fn mark_session_consumed(cfg: &TskConfig, task_id: &str) -> Result<()> {
    let Some(mut session) = read_task_session(cfg, task_id)? else {
        return Ok(());
    };
    if !session.pending {
        return Ok(());
    }
    session.pending = false;
    save_task_session(cfg, &session)
}

pub fn restore_session(
    cfg: &TskConfig,
    task: &Task,
    session: &TaskBrowserSession,
) -> Result<usize> {
    let profile = task_chromium_profile_dir(task, cfg);
    let mut opened = 0;
    for window in &session.windows {
        let urls: Vec<&str> = window
            .urls
            .iter()
            .map(String::as_str)
            .filter(|url| !url.is_empty())
            .collect();
        if urls.is_empty() {
            continue;
        }
        open_new_window_with_urls(cfg, &window.workspace, &urls, profile.as_deref())?;
        opened += 1;
    }
    Ok(opened)
}

pub fn assign_session_windows(hypr: &[HyprWindow], live: &[LiveWindow]) -> Vec<SessionWindow> {
    let mut unused: Vec<&LiveWindow> = live.iter().collect();
    let mut out = Vec::new();

    for client in hypr {
        let workspace = client_workspace_name(client);
        if let Some(idx) = best_live_match(client, &unused) {
            let live = unused.remove(idx);
            out.push(session_window_from_live(&workspace, live));
        }
    }

    if out.is_empty() && hypr.len() == 1 {
        let workspace = client_workspace_name(&hypr[0]);
        if live.len() == 1 {
            out.push(session_window_from_live(&workspace, &live[0]));
        } else if let Some(focused) = live.iter().find(|w| w.focused) {
            out.push(session_window_from_live(&workspace, focused));
        }
    }

    out
}

fn session_window_from_live(workspace: &str, live: &LiveWindow) -> SessionWindow {
    let title = live
        .tabs
        .iter()
        .find(|t| t.active)
        .or_else(|| live.tabs.first())
        .map(|t| t.title.clone())
        .unwrap_or_default();
    SessionWindow {
        workspace: workspace.to_string(),
        urls: live.tabs.iter().map(|t| t.url.clone()).collect(),
        title,
    }
}

fn best_live_match(client: &HyprWindow, unused: &[&LiveWindow]) -> Option<usize> {
    unused
        .iter()
        .enumerate()
        .filter_map(|(idx, live)| {
            let score = title_match_score(&client.title, live)?;
            Some((idx, score))
        })
        .max_by_key(|(_, score)| *score)
        .map(|(idx, _)| idx)
}

fn title_match_score(hypr_title: &str, live: &LiveWindow) -> Option<u32> {
    let hypr = normalize_title(hypr_title);
    if hypr.is_empty() {
        return None;
    }
    live.tabs
        .iter()
        .filter_map(|tab| {
            let tab_title = normalize_title(&tab.title);
            if tab_title.is_empty() {
                return None;
            }
            if hypr == tab_title {
                return Some(if tab.active { 300 } else { 200 });
            }
            if hypr.contains(&tab_title) || tab_title.contains(&hypr) {
                return Some(if tab.active { 150 } else { 100 });
            }
            None
        })
        .max()
}

pub fn normalize_title(title: &str) -> String {
    let mut t = title.trim().to_lowercase();
    for suffix in [
        " - chromium",
        " – chromium",
        " — chromium",
        " - google chrome",
        " – google chrome",
        " — google chrome",
    ] {
        if let Some(stripped) = t.strip_suffix(suffix) {
            t = stripped.trim_end().to_string();
        }
    }
    strip_trailing_count(&t)
}

fn strip_trailing_count(title: &str) -> String {
    let Some(open) = title.rfind(" (") else {
        return title.to_string();
    };
    let rest = &title[open + 2..];
    if rest.ends_with(')')
        && rest[..rest.len() - 1]
            .chars()
            .all(|c| c.is_ascii_digit() || c == ',' || c == ' ')
    {
        return title[..open].trim_end().to_string();
    }
    title.to_string()
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<Option<T>> {
    if !path.is_file() {
        return Ok(None);
    }
    let raw = fs::read_to_string(path).map_err(|source| TskError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let value = serde_json::from_str(&raw).map_err(|source| TskError::Parse {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(Some(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hypr(title: &str, workspace: &str) -> HyprWindow {
        HyprWindow {
            address: format!("0x{title}"),
            title: title.into(),
            class_name: "chromium".into(),
            workspace: 1,
            workspace_name: workspace.into(),
            pid: Some(1),
        }
    }

    fn live(title: &str, url: &str) -> LiveWindow {
        LiveWindow {
            id: 1,
            focused: true,
            tabs: vec![LiveTab {
                url: url.into(),
                title: title.into(),
                active: true,
            }],
        }
    }

    #[test]
    fn normalize_strips_chromium_suffix() {
        assert_eq!(normalize_title("Docs - Chromium"), "docs");
        assert_eq!(normalize_title("Docs — Chromium"), "docs");
        assert_eq!(normalize_title("Inbox (12) - Chromium"), "inbox");
        assert_eq!(normalize_title("Docs"), "docs");
    }

    #[test]
    fn assign_matches_by_title() {
        let hypr_windows = vec![
            hypr("Rust docs - Chromium", "task-2"),
            hypr("Inbox", "task-3"),
        ];
        let lives = vec![
            live("Inbox", "https://mail.example"),
            live("Rust docs", "https://doc.rust-lang.org"),
        ];
        let assigned = assign_session_windows(&hypr_windows, &lives);
        assert_eq!(assigned.len(), 2);
        assert_eq!(assigned[0].workspace, "task-2");
        assert_eq!(assigned[0].urls, vec!["https://doc.rust-lang.org"]);
        assert_eq!(assigned[1].workspace, "task-3");
        assert_eq!(assigned[1].urls, vec!["https://mail.example"]);
    }

    #[test]
    fn assign_falls_back_when_only_one_window_each() {
        let assigned = assign_session_windows(
            &[hypr("something else", "task-1")],
            &[live("Unrelated title", "https://example.com")],
        );
        assert_eq!(assigned.len(), 1);
        assert_eq!(assigned[0].urls, vec!["https://example.com"]);
        assert_eq!(assigned[0].workspace, "task-1");
    }

    #[test]
    fn assign_falls_back_to_focused_when_one_hypr_and_many_live() {
        let extra = LiveWindow {
            id: 2,
            focused: false,
            tabs: vec![LiveTab {
                url: "https://other.test".into(),
                title: "Other".into(),
                active: true,
            }],
        };
        let focused = LiveWindow {
            id: 1,
            focused: true,
            tabs: vec![LiveTab {
                url: "https://focused.test".into(),
                title: "No overlap".into(),
                active: true,
            }],
        };
        let assigned =
            assign_session_windows(&[hypr("something else", "tid-2")], &[extra, focused]);
        assert_eq!(assigned.len(), 1);
        assert_eq!(assigned[0].urls, vec!["https://focused.test"]);
    }

    #[test]
    fn missing_pending_field_defaults_true() {
        let session: TaskBrowserSession = serde_json::from_str(
            r#"{"task_id":"t","saved_at":"2026-08-14T00:00:00Z","windows":[]}"#,
        )
        .unwrap();
        assert!(session.pending);
    }

    #[test]
    fn mark_consumed_clears_pending() {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = TskConfig::default();
        cfg.tasks_base_dir = dir.path().to_path_buf();
        let session = TaskBrowserSession {
            task_id: "task-1".into(),
            saved_at: Utc::now(),
            windows: vec![SessionWindow {
                workspace: "task-1".into(),
                urls: vec!["https://example.com".into()],
                title: "Example".into(),
            }],
            pending: true,
        };
        save_task_session(&cfg, &session).unwrap();
        mark_session_consumed(&cfg, "task-1").unwrap();
        let saved = read_task_session(&cfg, "task-1").unwrap().unwrap();
        assert!(!saved.pending);
        assert_eq!(saved.windows[0].urls, vec!["https://example.com"]);
    }

    #[test]
    fn ingest_windows_message_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = TskConfig::default();
        cfg.data_dir = dir.path().to_path_buf();
        let raw = br#"{"op":"windows","windows":[{"id":7,"focused":true,"tabs":[{"url":"https://x.test","title":"X","active":true}]}]}"#;
        ingest_native_message(&cfg, raw).unwrap();
        let live = read_live_windows(&cfg).unwrap().unwrap();
        assert_eq!(live.windows.len(), 1);
        assert_eq!(live.windows[0].tabs[0].url, "https://x.test");
    }

    fn sample_task(id: &str) -> Task {
        let now = Utc::now();
        Task {
            id: id.into(),
            name: id.into(),
            status: TaskStatus::Active,
            repo_url: None,
            repo_path: "/tmp".into(),
            source_repo_path: None,
            branch: None,
            container_name: format!("tsk-{id}"),
            container_isolation: false,
            workspace_count: 10,
            browser_profile: None,
            created_at: now,
            last_active_at: now,
            listed_at: now,
            agent_notes_path: None,
            ports: vec![],
        }
    }

    #[test]
    fn ingest_writes_current_task_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = TskConfig::default();
        cfg.data_dir = dir.path().to_path_buf();
        cfg.tasks_base_dir = dir.path().join("tasks");
        let task = sample_task("tabc1234");
        let registry = crate::registry::Registry::new(None, cfg.clone()).unwrap();
        let mut state = registry.load_state().unwrap();
        state.context_mode = ContextMode::Task;
        state.current_task_id = Some(task.id.clone());
        state.tasks.insert(task.id.clone(), task.clone());
        registry.save_state(&state).unwrap();

        let raw = br#"{"op":"windows","windows":[{"id":7,"focused":true,"tabs":[{"url":"https://auto.test","title":"Auto","active":true}]}]}"#;
        ingest_native_message(&cfg, raw).unwrap();
        let session = read_task_session(&cfg, &task.id).unwrap().unwrap();
        assert!(!session.pending);
        assert_eq!(session.windows[0].urls, vec!["https://auto.test"]);
    }

    #[test]
    fn ingest_does_not_overwrite_pending_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = TskConfig::default();
        cfg.data_dir = dir.path().to_path_buf();
        cfg.tasks_base_dir = dir.path().join("tasks");
        let task = sample_task("tpend001");
        let registry = crate::registry::Registry::new(None, cfg.clone()).unwrap();
        let mut state = registry.load_state().unwrap();
        state.context_mode = ContextMode::Task;
        state.current_task_id = Some(task.id.clone());
        state.tasks.insert(task.id.clone(), task.clone());
        registry.save_state(&state).unwrap();
        save_task_session(
            &cfg,
            &TaskBrowserSession {
                task_id: task.id.clone(),
                saved_at: Utc::now(),
                windows: vec![SessionWindow {
                    workspace: "tpend001-2".into(),
                    urls: vec!["https://kept.test".into()],
                    title: "Kept".into(),
                }],
                pending: true,
            },
        )
        .unwrap();

        let raw = br#"{"op":"windows","windows":[{"id":1,"focused":true,"tabs":[{"url":"https://new.test","title":"New","active":true}]}]}"#;
        ingest_native_message(&cfg, raw).unwrap();
        let session = read_task_session(&cfg, &task.id).unwrap().unwrap();
        assert!(session.pending);
        assert_eq!(session.windows[0].urls, vec!["https://kept.test"]);
    }

    #[test]
    fn archive_keeps_last_snapshot_when_capture_empty() {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = TskConfig::default();
        cfg.tasks_base_dir = dir.path().to_path_buf();
        cfg.data_dir = dir.path().to_path_buf();
        let task = sample_task("tarch001");
        save_task_session(
            &cfg,
            &TaskBrowserSession {
                task_id: task.id.clone(),
                saved_at: Utc::now(),
                windows: vec![SessionWindow {
                    workspace: "tarch001-2".into(),
                    urls: vec!["https://kept.test".into()],
                    title: "Kept".into(),
                }],
                pending: false,
            },
        )
        .unwrap();
        let session = capture_and_save(&cfg, &task).unwrap();
        assert!(session.pending);
        assert_eq!(session.windows[0].urls, vec!["https://kept.test"]);
    }
}
