use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::workspaces::task_workspace_names;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextMode {
    #[default]
    Default,
    Task,
}

impl ContextMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Task => "task",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Active,
    Idle,
    Archived,
}

impl TaskStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Idle => "idle",
            Self::Archived => "archived",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub name: String,
    pub status: TaskStatus,
    pub repo_url: Option<String>,
    pub repo_path: PathBuf,
    /// Registered checkout the task was created from (git/jj source root).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_repo_path: Option<PathBuf>,
    pub branch: Option<String>,
    pub container_name: String,
    /// When true, terminals / editors / browsers launch via Distrobox enter.
    #[serde(default)]
    pub container_isolation: bool,
    pub workspace_count: u32,
    pub browser_profile: Option<String>,
    pub created_at: DateTime<Utc>,
    pub last_active_at: DateTime<Utc>,
    /// When the task was created or last restored from archive (TUI list order).
    pub listed_at: DateTime<Utc>,
    pub agent_notes_path: Option<PathBuf>,
    #[serde(default)]
    pub ports: Vec<u16>,
}

impl Task {
    pub fn workspace_names(&self) -> Vec<String> {
        task_workspace_names(&self.id, self.workspace_count)
    }

    pub fn main_workspace(&self) -> String {
        self.workspace_names()
            .into_iter()
            .next()
            .unwrap_or_else(|| "1".into())
    }

    /// Newest `listed_at` first; name then id break ties.
    pub fn cmp_list_order(&self, other: &Self) -> Ordering {
        other
            .listed_at
            .cmp(&self.listed_at)
            .then_with(|| self.name.to_lowercase().cmp(&other.name.to_lowercase()))
            .then_with(|| self.id.cmp(&other.id))
    }

    /// Newest `last_active_at` first; name then id break ties.
    pub fn cmp_access_order(&self, other: &Self) -> Ordering {
        other
            .last_active_at
            .cmp(&self.last_active_at)
            .then_with(|| self.name.to_lowercase().cmp(&other.name.to_lowercase()))
            .then_with(|| self.id.cmp(&other.id))
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WindowRecord {
    pub hypr_address: String,
    pub task_id: Option<String>,
    pub title: String,
    #[serde(rename = "class")]
    pub class_name: String,
    pub workspace: i32,
    pub workspace_name: String,
    /// Hyprland workspace name where the window was first registered (recovery target).
    #[serde(default)]
    pub home_workspace_name: String,
    pub pid: Option<i32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionState {
    pub context_mode: ContextMode,
    pub current_task_id: Option<String>,
    #[serde(default)]
    pub last_workspace: HashMap<String, i32>,
    /// Per taskspace key: monitor name → relative workspace slot (1-based).
    #[serde(default)]
    pub last_monitor_workspace: HashMap<String, HashMap<String, i32>>,
    pub default_workspace_count: u32,
    /// 1-based slots that map to default (numeric) Hyprland workspaces in any taskspace.
    #[serde(default)]
    pub global_workspace_slots: Vec<u32>,
    #[serde(default)]
    pub tasks: HashMap<String, Task>,
    #[serde(default)]
    pub windows: HashMap<String, WindowRecord>,
}

impl SessionState {
    pub fn taskspace_key(&self) -> String {
        if self.context_mode == ContextMode::Task {
            if let Some(id) = &self.current_task_id {
                return format!("task:{id}");
            }
        }
        self.context_mode.as_str().into()
    }

    pub fn taskspace_label(&self) -> String {
        self.taskspace_key()
    }
}

/// Opaque task identifier (not derived from the display name).
pub fn generate_task_id() -> String {
    let mut bytes = [0u8; 4];
    if std::fs::File::open("/dev/urandom")
        .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut bytes))
        .is_err()
    {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let pid = std::process::id() as u128;
        let mixed = nanos ^ (pid << 32);
        bytes.copy_from_slice(&mixed.to_le_bytes()[..4]);
    }
    format!(
        "t{}",
        bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
    )
}

pub fn slugify(name: &str) -> String {
    let slug: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let trimmed = slug.trim_matches('-');
    if trimmed.is_empty() {
        "task".into()
    } else {
        trimmed.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_basic() {
        assert_eq!(slugify("My Task"), "my-task");
    }

    #[test]
    fn generate_task_id_is_opaque() {
        let id = generate_task_id();
        assert!(id.starts_with('t'));
        assert_eq!(id.len(), 9);
        assert_ne!(id, slugify("Some Name"));
    }

    fn sample_task(id: &str, name: &str, listed_at: DateTime<Utc>) -> Task {
        Task {
            id: id.into(),
            name: name.into(),
            status: TaskStatus::Active,
            repo_url: None,
            repo_path: PathBuf::from("/tmp"),
            source_repo_path: None,
            branch: None,
            container_name: format!("tsk-{id}"),
            container_isolation: false,
            workspace_count: 3,
            browser_profile: None,
            created_at: listed_at,
            last_active_at: listed_at,
            listed_at,
            agent_notes_path: None,
            ports: vec![],
        }
    }

    #[test]
    fn cmp_list_order_newest_listed_first() {
        let t0 = Utc::now();
        let older = sample_task("told", "older", t0);
        let newer = sample_task("tnew", "newer", t0 + chrono::Duration::seconds(1));
        let mut tasks = [&older, &newer];
        tasks.sort_by(|a, b| a.cmp_list_order(b));
        assert_eq!(tasks[0].id, "tnew");
        assert_eq!(tasks[1].id, "told");
    }

    #[test]
    fn cmp_list_order_breaks_ties_by_name() {
        let t0 = Utc::now();
        let zeta = sample_task("tz", "zeta", t0);
        let alpha = sample_task("ta", "alpha", t0);
        let mut tasks = [&zeta, &alpha];
        tasks.sort_by(|a, b| a.cmp_list_order(b));
        assert_eq!(tasks[0].id, "ta");
        assert_eq!(tasks[1].id, "tz");
    }

    #[test]
    fn cmp_access_order_newest_active_first() {
        let t0 = Utc::now();
        let mut older = sample_task("told", "older", t0);
        let mut newer = sample_task("tnew", "newer", t0);
        older.last_active_at = t0;
        newer.last_active_at = t0 + chrono::Duration::seconds(1);
        let mut tasks = [&older, &newer];
        tasks.sort_by(|a, b| a.cmp_access_order(b));
        assert_eq!(tasks[0].id, "tnew");
        assert_eq!(tasks[1].id, "told");
    }
}
