//! Task and taskspace orchestration — writes `state.db` and publishes bar updates.

use std::path::Path;

use chrono::Utc;

use crate::config::LaeConfig;
use crate::error::{LaeError, Result};
use crate::hyprland;
use crate::models::{ContextMode, SessionState, Task, TaskStatus};
use crate::registry::Registry;
use crate::state_notify::{self, StateChangeKind};
use crate::workspace_nav;
use crate::workspaces::{default_taskspace_workspace_names, task_taskspace_workspace_names};
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
        self.commit_state(state, None)
    }

    /// Persist state and notify Waybar subscribers.
    fn commit_state(
        &self,
        state: &SessionState,
        change: Option<StateChangeKind>,
    ) -> Result<()> {
        self.registry.save_state(state)?;
        self.write_runtime_files(state)?;
        if let Some(kind) = change {
            state_notify::publish(kind);
        }
        Ok(())
    }

    /// Persist state after a workspace switch (no state-events publish).
    fn persist_workspace_switch(&self, state: &SessionState) -> Result<()> {
        self.registry.save_state(state)?;
        self.write_runtime_files(state)
    }

    fn write_runtime_files(&self, state: &SessionState) -> Result<()> {
        if let Ok(dir) = lae_runtime_dir() {
            ensure_parent(&dir.join("_"))?;
            let _ = std::fs::write(dir.join("context"), state.taskspace_label());
        }
        crate::workspace_slots::write_slot_cache(state);
        Ok(())
    }

    pub fn initialize(&self) -> Result<()> {
        let mut state = self.load_state()?;
        state.default_workspace_count = self.config.default_workspace_count;
        self.commit_state(&state, None)
    }

    /// Rename default numeric slots — run once after daemon start (background).
    pub fn provision_default_workspaces(&self) -> Result<()> {
        if hyprland::available() && self.config.hyprland_enabled {
            workspace_nav::setup_default_taskspace_workspaces(self.config.default_workspace_count);
        }
        Ok(())
    }

    /// Drop remembered last-workspace and per-monitor layout state.
    pub fn reset_navigation_layout(&self) -> Result<()> {
        let mut state = self.load_state()?;
        workspace_nav::clear_navigation_memory(&mut state);
        workspace_nav::clear_runtime_slot_cache();
        self.commit_state(&state, Some(StateChangeKind::Full))
    }

    pub fn context_default(&self) -> Result<()> {
        let mut state = self.load_state()?;
        workspace_nav::set_taskspace(&mut state, ContextMode::Default, None)
            .map_err(|e| crate::error::LaeError::Other(e))?;
        self.commit_state(&state, Some(StateChangeKind::Taskspace))
    }

    pub fn workspace_go(&self, relative: i32) -> Result<Option<String>> {
        let mut state = self.load_state()?;
        let name = workspace_nav::workspace_name_for_relative(&state, relative);
        if let Some(ref target) = name {
            hyprland::switch_workspace_for_navigation(target);
            workspace_nav::remember_workspace(&mut state, relative);
            self.persist_workspace_switch(&state)?;
        }
        Ok(name)
    }

    /// Hyprland-only workspace switch for keybind hot path (no state write).
    pub fn workspace_dispatch(&self, relative: i32) -> Result<Option<String>> {
        let name = crate::workspace_slots::read_slot_target(relative).or_else(|| {
            self.load_state()
                .ok()
                .and_then(|state| workspace_nav::workspace_name_for_relative(&state, relative))
        });
        if let Some(ref target) = name {
            hyprland::switch_workspace_for_navigation(target);
        }
        Ok(name)
    }

    pub fn remember_workspace_go(&self, relative: i32) -> Result<Option<String>> {
        let mut state = self.load_state()?;
        let name = workspace_nav::workspace_name_for_relative(&state, relative);
        if name.is_some() {
            workspace_nav::remember_workspace(&mut state, relative);
            self.persist_workspace_switch(&state)?;
        }
        Ok(name)
    }

    pub fn remember_workspace_goto(&self, name: &str) -> Result<Option<String>> {
        let mut state = self.load_state()?;
        let allowed = crate::workspaces::allowed_workspace_names(&state);
        if !allowed.iter().any(|n| n == name) {
            return Ok(None);
        }
        if let Some(idx) = allowed.iter().position(|n| n == name) {
            workspace_nav::remember_workspace(&mut state, (idx + 1) as i32);
            self.persist_workspace_switch(&state)?;
        }
        Ok(Some(name.to_string()))
    }

    pub fn workspace_next(&self) -> Result<Option<String>> {
        let state = self.load_state()?;
        let relative = workspace_nav::workspace_next_relative(&state);
        drop(state);
        if let Some(rel) = relative {
            return self.workspace_go(rel);
        }
        Ok(None)
    }

    pub fn workspace_prev(&self) -> Result<Option<String>> {
        let state = self.load_state()?;
        let relative = workspace_nav::workspace_prev_relative(&state);
        drop(state);
        if let Some(rel) = relative {
            return self.workspace_go(rel);
        }
        Ok(None)
    }

    pub fn workspace_goto(&self, name: &str) -> Result<Option<String>> {
        let mut state = self.load_state()?;
        let allowed = crate::workspaces::allowed_workspace_names(&state);
        if !allowed.iter().any(|n| n == name) {
            return Ok(None);
        }
        hyprland::switch_workspace_for_navigation(name);
        let allowed = crate::workspaces::allowed_workspace_names(&state);
        if let Some(idx) = allowed.iter().position(|n| n == name) {
            workspace_nav::remember_workspace(&mut state, (idx + 1) as i32);
            self.persist_workspace_switch(&state)?;
        }
        Ok(Some(name.to_string()))
    }

    pub fn create_task(
        &self,
        name: &str,
        switch: bool,
        repo: crate::task_repo::TaskRepoSource,
        cwd: Option<&Path>,
    ) -> Result<Task> {
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

        let (repo_path, create_repo_dir) = repo.resolve(&task_home, cwd)?;
        let repo_url = repo.resolve_url();

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
        if create_repo_dir {
            std::fs::create_dir_all(&repo_path).map_err(|source| LaeError::Write {
                path: repo_path.clone(),
                source,
            })?;
        }

        let now = Utc::now();
        let mut task = Task {
            id: task_id.clone(),
            name: name.to_string(),
            status: TaskStatus::Active,
            repo_url,
            repo_path,
            branch: None,
            container_name: format!("{}-{task_id}", self.config.container_prefix),
            workspace_count: self.config.default_workspace_count,
            browser_profile: None,
            created_at: now,
            last_active_at: now,
            agent_notes_path: Some(notes_path),
            ports: vec![],
        };
        self.registry.touch_task(&mut task);
        state.tasks.insert(task_id.clone(), task.clone());

        if hyprland::available() && self.config.hyprland_enabled && !switch {
            workspace_nav::setup_task_workspaces(&task_id, self.config.default_workspace_count);
        }

        if switch {
            self.commit_state(&state, Some(StateChangeKind::Full))?;
            self.switch_task(&task_id)
        } else {
            self.commit_state(&state, Some(StateChangeKind::Full))?;
            Ok(task)
        }
    }

    pub fn archive_task(&self, task_id: &str) -> Result<()> {
        let mut state = self.load_state()?;
        let task = state
            .tasks
            .get(task_id)
            .cloned()
            .ok_or_else(|| LaeError::Other(format!("Unknown task: {task_id}")))?;
        if task.status == TaskStatus::Archived {
            return Err(LaeError::Other(format!("Task is already archived: {task_id}")));
        }

        let was_current = crate::task_cleanup::is_active_task_context(&state, &task);
        if was_current {
            workspace_nav::set_taskspace(&mut state, ContextMode::Default, None)
                .map_err(LaeError::Other)?;
            self.commit_state(&state, Some(StateChangeKind::Taskspace))?;
        }

        let _closed = crate::task_cleanup::close_task_windows(&task)?;
        if let Err(err) = crate::task_cleanup::stop_task_container(&task) {
            eprintln!(
                "lae: archive task {}: stop container {}: {err}",
                task.id, task.container_name
            );
        }
        crate::task_cleanup::purge_task_windows(&mut state, task_id);

        if let Some(entry) = state.tasks.get_mut(task_id) {
            entry.status = TaskStatus::Archived;
        }

        self.commit_state(&state, Some(StateChangeKind::Full))
    }

    pub fn delete_task(&self, task_id: &str) -> Result<()> {
        let mut state = self.load_state()?;
        let task = state
            .tasks
            .get(task_id)
            .cloned()
            .ok_or_else(|| LaeError::Other(format!("Unknown task: {task_id}")))?;

        if crate::task_cleanup::is_active_task_context(&state, &task) {
            workspace_nav::set_taskspace(&mut state, ContextMode::Default, None)
                .map_err(LaeError::Other)?;
            self.commit_state(&state, Some(StateChangeKind::Taskspace))?;
        }

        let _closed = crate::task_cleanup::close_task_windows(&task)?;
        if let Err(err) = crate::task_cleanup::stop_task_container(&task) {
            eprintln!(
                "lae: delete task {}: stop container {}: {err}",
                task.id, task.container_name
            );
        }
        if let Err(err) = crate::task_cleanup::remove_task_container(&task) {
            eprintln!(
                "lae: delete task {}: remove container {}: {err}",
                task.id, task.container_name
            );
        }
        if let Err(err) = crate::task_cleanup::remove_task_data_dir(&self.config, &task) {
            eprintln!("lae: delete task {}: remove data dir: {err}", task.id);
        }
        crate::task_cleanup::purge_task_windows(&mut state, task_id);
        crate::task_cleanup::purge_task_session_keys(&mut state, task_id);
        state.tasks.remove(task_id);

        self.commit_state(&state, Some(StateChangeKind::Full))
    }

    pub fn preview_task_teardown(&self, task_id: &str) -> Result<crate::task_cleanup::TaskTeardownPreview> {
        let state = self.load_state()?;
        let task = state
            .tasks
            .get(task_id)
            .cloned()
            .ok_or_else(|| LaeError::Other(format!("Unknown task: {task_id}")))?;
        crate::task_cleanup::preview_teardown(&self.config, &task)
    }

    pub fn list_archived_tasks(&self) -> Result<Vec<Task>> {
        let state = self.load_state()?;
        Ok(state
            .tasks
            .values()
            .filter(|t| t.status == TaskStatus::Archived)
            .cloned()
            .collect())
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

        self.commit_state(&state, Some(StateChangeKind::Taskspace))?;
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
                workspaces: task_taskspace_workspace_names(&state, &task.id),
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

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
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
        let task = svc
            .create_task(
                "Auth Fix",
                false,
                crate::task_repo::TaskRepoSource::Scratch,
                None,
            )
            .unwrap();
        assert!(task.id.starts_with('t'));
        assert_eq!(task.name, "Auth Fix");
        assert_eq!(task.workspace_count, 10);
        assert_eq!(task.workspace_names().len(), 10);
        assert_eq!(task.workspace_names()[0], format!("{}-1", task.id));
        assert_eq!(task.workspace_names()[9], format!("{}-10", task.id));
        assert!(task.agent_notes_path.as_ref().unwrap().is_file());
        assert!(task.repo_path.is_dir());

        let state = svc.load_state().unwrap();
        assert!(state.tasks.contains_key(&task.id));
        assert_eq!(state.context_mode, ContextMode::Default);
    }

    #[test]
    fn create_task_with_switch_enters_taskspace() {
        let dir = tempdir().unwrap();
        let svc = test_service(dir.path());
        let task = svc
            .create_task(
                "billing",
                true,
                crate::task_repo::TaskRepoSource::Scratch,
                None,
            )
            .unwrap();
        let state = svc.load_state().unwrap();
        assert_eq!(state.context_mode, ContextMode::Task);
        assert_eq!(state.current_task_id.as_deref(), Some(task.id.as_str()));
    }

    #[test]
    fn archive_task_leaves_default_taskspace() {
        let dir = tempdir().unwrap();
        let svc = test_service(dir.path());
        let task = svc
            .create_task(
                "temp",
                true,
                crate::task_repo::TaskRepoSource::Scratch,
                None,
            )
            .unwrap();
        svc.archive_task(&task.id).unwrap();
        let state = svc.load_state().unwrap();
        assert_eq!(state.tasks.get(&task.id).unwrap().status, TaskStatus::Archived);
        assert_eq!(state.context_mode, ContextMode::Default);
        assert!(state.current_task_id.is_none());
        assert!(svc.tasks_for_menu().unwrap().iter().all(|t| t.id != task.id));
        assert!(dir.path().join("tasks").join(&task.id).is_dir());
    }

    #[test]
    fn delete_task_removes_record_and_data() {
        let dir = tempdir().unwrap();
        let svc = test_service(dir.path());
        let task = svc
            .create_task(
                "gone",
                false,
                crate::task_repo::TaskRepoSource::Scratch,
                None,
            )
            .unwrap();
        let task_home = dir.path().join("tasks").join(&task.id);
        assert!(task_home.is_dir());

        svc.delete_task(&task.id).unwrap();
        let state = svc.load_state().unwrap();
        assert!(!state.tasks.contains_key(&task.id));
        assert!(!task_home.exists());
    }

    #[test]
    fn delete_task_while_current_leaves_default_taskspace() {
        let dir = tempdir().unwrap();
        let svc = test_service(dir.path());
        let task = svc
            .create_task(
                "current",
                true,
                crate::task_repo::TaskRepoSource::Scratch,
                None,
            )
            .unwrap();
        svc.delete_task(&task.id).unwrap();
        let state = svc.load_state().unwrap();
        assert!(!state.tasks.contains_key(&task.id));
        assert_eq!(state.context_mode, ContextMode::Default);
        assert!(state.current_task_id.is_none());
    }

    #[test]
    fn create_task_with_path_uses_checkout_not_scratch() {
        let dir = tempdir().unwrap();
        let checkout = dir.path().join("checkout");
        std::fs::create_dir_all(checkout.join(".git")).unwrap();
        let svc = test_service(dir.path());
        let task = svc
            .create_task(
                "My Feature",
                false,
                crate::task_repo::TaskRepoSource::Path(checkout.clone()),
                None,
            )
            .unwrap();
        assert_eq!(task.name, "My Feature");
        assert!(task.id.starts_with('t'));
        assert_eq!(task.repo_path, checkout);
        let scratch = dir.path().join("tasks").join(&task.id).join("repo");
        assert!(!scratch.exists() || !scratch.is_dir());
    }
}
