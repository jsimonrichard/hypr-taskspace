//! Task and taskspace orchestration (direct mode — no daemon socket).

use chrono::Utc;

use crate::config::LaeConfig;
use crate::error::{LaeError, Result};
use crate::hyprland;
use crate::models::{ContextMode, SessionState, Task, TaskStatus};
use crate::registry::Registry;
use crate::waybar::refresh_modules_cache;
use crate::workspace_nav;
use crate::workspaces::default_taskspace_workspace_names;
use crate::xdg::{ensure_parent, lae_runtime_dir};

pub struct TaskService {
    registry: Registry,
    config: LaeConfig,
}

impl TaskService {
    pub fn with_defaults() -> Result<Self> {
        let config = crate::config::load_config()?;
        let registry = Registry::new(None, config.clone())?;
        Ok(Self { registry, config })
    }

    pub fn load_state(&self) -> Result<SessionState> {
        self.registry.load_state()
    }

    pub fn save_state(&self, state: &SessionState) -> Result<()> {
        self.registry.save_state(state)?;
        self.write_runtime_files(state, true)
    }

    /// Persist state after a workspace switch — skips legacy Waybar JSON cache rebuild.
    /// The CFFI module updates from Hyprland socket events; cache refresh is unnecessary here.
    fn persist_workspace_switch(&self, state: &SessionState) -> Result<()> {
        self.registry.save_state(state)?;
        self.write_runtime_files(state, false)
    }

    fn write_runtime_files(&self, state: &SessionState, refresh_cache: bool) -> Result<()> {
        if let Ok(dir) = lae_runtime_dir() {
            ensure_parent(&dir.join("_"))?;
            let _ = std::fs::write(dir.join("context"), state.taskspace_label());
        }
        if refresh_cache {
            let _ = refresh_modules_cache(&self.registry, false);
        }
        Ok(())
    }

    pub fn initialize(&self) -> Result<()> {
        let mut state = self.load_state()?;
        state.default_workspace_count = self.config.default_workspace_count;
        if hyprland::available() && self.config.hyprland_enabled {
            workspace_nav::setup_default_taskspace_workspaces(self.config.default_workspace_count);
        }
        self.save_state(&state)
    }

    pub fn context_default(&self) -> Result<()> {
        let mut state = self.load_state()?;
        workspace_nav::set_taskspace(&mut state, ContextMode::Default, None)
            .map_err(|e| crate::error::LaeError::Other(e))?;
        self.save_state(&state)
    }

    pub fn context_global(&self) -> Result<()> {
        let mut state = self.load_state()?;
        workspace_nav::set_taskspace(&mut state, ContextMode::Global, None)
            .map_err(|e| crate::error::LaeError::Other(e))?;
        self.save_state(&state)
    }

    pub fn context_restore(&self) -> Result<()> {
        let mut state = self.load_state()?;
        workspace_nav::restore_taskspace(&mut state);
        self.save_state(&state)
    }

    pub fn toggle_global(&self) -> Result<()> {
        let mut state = self.load_state()?;
        workspace_nav::toggle_global(&mut state);
        self.save_state(&state)
    }

    pub fn workspace_go(&self, relative: i32) -> Result<Option<String>> {
        let mut state = self.load_state()?;
        let name = workspace_nav::workspace_go(&mut state, relative);
        if name.is_some() {
            self.persist_workspace_switch(&state)?;
        }
        Ok(name)
    }

    pub fn workspace_next(&self) -> Result<Option<String>> {
        let mut state = self.load_state()?;
        let name = workspace_nav::workspace_next(&mut state);
        if name.is_some() {
            self.persist_workspace_switch(&state)?;
        }
        Ok(name)
    }

    pub fn workspace_prev(&self) -> Result<Option<String>> {
        let mut state = self.load_state()?;
        let name = workspace_nav::workspace_prev(&mut state);
        if name.is_some() {
            self.persist_workspace_switch(&state)?;
        }
        Ok(name)
    }

    pub fn workspace_goto(&self, name: &str) -> Result<Option<String>> {
        let mut state = self.load_state()?;
        let result = workspace_nav::workspace_goto_name(&mut state, name);
        if result.is_some() {
            self.persist_workspace_switch(&state)?;
        }
        Ok(result)
    }

    pub fn create_task(&self, name: &str, switch: bool) -> Result<Task> {
        let mut state = self.load_state()?;
        let active_count = state
            .tasks
            .values()
            .filter(|t| t.status != TaskStatus::Archived)
            .count();
        if active_count >= self.config.max_tasks as usize {
            return Err(LaeError::Other(format!(
                "Maximum task limit ({}) reached",
                self.config.max_tasks
            )));
        }

        let task_id = self.registry.unique_task_id(&state, name);
        let task_home = self.config.tasks_base_dir.join(&task_id);
        let agent_dir = task_home.join(".lae");
        std::fs::create_dir_all(&agent_dir).map_err(|source| LaeError::Write {
            path: agent_dir.clone(),
            source,
        })?;
        let notes_path = agent_dir.join("agent-notes.md");
        if !notes_path.is_file() {
            std::fs::write(
                &notes_path,
                format!("# {name}\n\nTask notes for agent and human.\n"),
            )
            .map_err(|source| LaeError::Write {
                path: notes_path.clone(),
                source,
            })?;
        }
        let repo_path = task_home.join("repo");
        std::fs::create_dir_all(&repo_path).map_err(|source| LaeError::Write {
            path: repo_path.clone(),
            source,
        })?;

        let now = Utc::now();
        let mut task = Task {
            id: task_id.clone(),
            name: name.to_string(),
            status: TaskStatus::Active,
            repo_url: None,
            repo_path,
            branch: None,
            container_name: format!("lae-{task_id}"),
            workspace_count: self.config.workspaces_per_task,
            browser_profile: None,
            created_at: now,
            last_active_at: now,
            agent_notes_path: Some(notes_path),
            ports: vec![],
        };
        self.registry.touch_task(&mut task);
        state.tasks.insert(task_id.clone(), task.clone());

        if hyprland::available() && self.config.hyprland_enabled {
            workspace_nav::setup_task_workspaces(&task);
        }

        if switch {
            self.save_state(&state)?;
            self.switch_task(&task_id)
        } else {
            self.save_state(&state)?;
            Ok(task)
        }
    }

    pub fn archive_task(&self, task_id: &str) -> Result<()> {
        let mut state = self.load_state()?;
        if !state.tasks.contains_key(task_id) {
            return Err(LaeError::Other(format!("Unknown task: {task_id}")));
        }
        if let Some(task) = state.tasks.get_mut(task_id) {
            task.status = TaskStatus::Archived;
        }
        if state.current_task_id.as_deref() == Some(task_id) {
            workspace_nav::set_taskspace(&mut state, ContextMode::Default, None)
                .map_err(LaeError::Other)?;
        }
        self.save_state(&state)
    }

    pub fn switch_task(&self, task_id: &str) -> Result<Task> {
        let mut state = self.load_state()?;
        let task = state
            .tasks
            .get(task_id)
            .cloned()
            .ok_or_else(|| crate::error::LaeError::Other(format!("Unknown task: {task_id}")))?;
        if task.status == TaskStatus::Archived {
            return Err(crate::error::LaeError::Other(format!(
                "Task is archived: {task_id}"
            )));
        }
        let mut task = task;
        task.status = TaskStatus::Active;
        task.last_active_at = Utc::now();
        state.tasks.insert(task.id.clone(), task.clone());

        workspace_nav::set_taskspace(&mut state, ContextMode::Task, Some(task_id))
            .map_err(|e| crate::error::LaeError::Other(e))?;
        state
            .last_workspace
            .entry(format!("task:{task_id}"))
            .or_insert(1);

        if hyprland::available() {
            hyprland::switch_workspace(&task.main_workspace());
        }
        self.save_state(&state)?;
        Ok(task)
    }

    pub fn resolve_task(&self, name_or_id: &str) -> Result<Task> {
        let state = self.load_state()?;
        self.registry
            .get_task(&state, name_or_id)
            .cloned()
            .ok_or_else(|| crate::error::LaeError::Other(format!("Unknown task: {name_or_id}")))
    }

    pub fn list_active_tasks(&self) -> Result<Vec<Task>> {
        let state = self.load_state()?;
        Ok(state
            .tasks
            .values()
            .filter(|t| t.status != TaskStatus::Archived)
            .cloned()
            .collect())
    }

    pub fn tasks_for_menu(&self) -> Result<Vec<MenuTask>> {
        let state = self.load_state()?;
        let mut items = Vec::new();

        items.push(MenuTask {
            id: "default".into(),
            name: "default".into(),
            kind: "default".into(),
            workspaces: default_taskspace_workspace_names(state.default_workspace_count),
            current: state.context_mode == ContextMode::Default,
            status: "system".into(),
        });

        for task in state.tasks.values() {
            if task.status == TaskStatus::Archived {
                continue;
            }
            items.push(MenuTask {
                id: task.id.clone(),
                name: task.name.clone(),
                kind: "task".into(),
                workspaces: task.workspace_names(),
                current: state.context_mode == ContextMode::Task
                    && state.current_task_id.as_deref() == Some(task.id.as_str()),
                status: task.status.as_str().into(),
            });
        }
        Ok(items)
    }

    pub fn taskspace_label(&self) -> Result<String> {
        Ok(self.load_state()?.taskspace_label())
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct MenuTask {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub workspaces: Vec<String>,
    pub current: bool,
    pub status: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::LaeConfig;
    use tempfile::tempdir;

    fn test_service(dir: &std::path::Path) -> TaskService {
        let mut config = LaeConfig::default();
        config.tasks_base_dir = dir.join("tasks");
        let db = dir.join("state.db");
        let registry = Registry::new(Some(db), config.clone()).unwrap();
        TaskService { registry, config }
    }

    #[test]
    fn create_task_registers_workspaces_without_switch() {
        let dir = tempdir().unwrap();
        let svc = test_service(dir.path());
        let task = svc.create_task("Auth Fix", false).unwrap();
        assert_eq!(task.id, "auth-fix");
        assert_eq!(task.workspace_count, 3);
        assert_eq!(task.workspace_names(), vec!["auth-fix-1", "auth-fix-2", "auth-fix-3"]);
        assert!(task.agent_notes_path.as_ref().unwrap().is_file());
        assert!(task.repo_path.is_dir());

        let state = svc.load_state().unwrap();
        assert!(state.tasks.contains_key("auth-fix"));
        assert_eq!(state.context_mode, ContextMode::Default);
    }

    #[test]
    fn create_task_with_switch_enters_taskspace() {
        let dir = tempdir().unwrap();
        let svc = test_service(dir.path());
        let task = svc.create_task("billing", true).unwrap();
        let state = svc.load_state().unwrap();
        assert_eq!(state.context_mode, ContextMode::Task);
        assert_eq!(state.current_task_id.as_deref(), Some(task.id.as_str()));
    }

    #[test]
    fn archive_task_leaves_default_taskspace() {
        let dir = tempdir().unwrap();
        let svc = test_service(dir.path());
        let task = svc.create_task("temp", true).unwrap();
        svc.archive_task(&task.id).unwrap();
        let state = svc.load_state().unwrap();
        assert_eq!(state.tasks.get(&task.id).unwrap().status, TaskStatus::Archived);
        assert_eq!(state.context_mode, ContextMode::Default);
        assert!(state.current_task_id.is_none());
        assert!(svc.tasks_for_menu().unwrap().iter().all(|t| t.id != task.id));
    }
}
