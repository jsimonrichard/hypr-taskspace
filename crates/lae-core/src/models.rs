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
    pub branch: Option<String>,
    pub container_name: String,
    pub workspace_count: u32,
    pub browser_profile: Option<String>,
    pub created_at: DateTime<Utc>,
    pub last_active_at: DateTime<Utc>,
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
}
