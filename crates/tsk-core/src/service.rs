//! Task and taskspace orchestration — writes `state.db` and publishes bar updates.

use std::path::Path;

use chrono::Utc;

use crate::config::TskConfig;
use crate::error::{TskError, Result};
use crate::hypr_log;
use crate::hyprland;
use crate::models::{ContextMode, SessionState, Task, TaskStatus};
use crate::registry::Registry;
use crate::state_notify::{self, StateChangeKind};
use crate::workspace_nav;
use crate::workspaces::{default_taskspace_workspace_names, task_taskspace_workspace_names};
use crate::xdg::{ensure_parent, tsk_runtime_dir};

pub struct TaskService {
    registry: Registry,
    config: TskConfig,
}

impl TaskService {
    pub fn with_defaults() -> Result<Self> {
        Self::with_config(crate::config::load_config()?)
    }

    pub fn with_config(config: TskConfig) -> Result<Self> {
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
        if let Ok(dir) = tsk_runtime_dir() {
            ensure_parent(&dir.join("_"))?;
            let _ = std::fs::write(dir.join("context"), state.taskspace_label());
        }
        crate::workspace_slots::write_slot_cache(state);
        Ok(())
    }

    pub fn initialize(&self) -> Result<()> {
        let mut state = self.load_state()?;
        state.default_workspace_count = self.config.default_workspace_count;
        state.global_workspace_slots = self.config.global_workspace_slots.clone();
        if self.config.hyprland_enabled && hyprland::available() {
            if let Ok(Some(active)) = hyprland::get_active_workspace() {
                if !active.name.is_empty() {
                    let _ = self.apply_external_workspace(&mut state, &active.name)?;
                }
            }
        }
        self.commit_state(&state, None)
    }

    /// Align session state with an external Hyprland workspace focus change.
    ///
    /// When the change crosses taskspaces, other monitors are restored to their
    /// saved slots in the new taskspace and Waybar receives a taskspace update.
    pub fn sync_external_workspace(&self, workspace_name: &str) -> Result<bool> {
        let mut state = self.load_state()?;
        let changed = self.apply_external_workspace(&mut state, workspace_name)?;
        Ok(changed)
    }

    fn apply_external_workspace(
        &self,
        state: &mut SessionState,
        workspace_name: &str,
    ) -> Result<bool> {
        if !self.config.hyprland_enabled {
            return Ok(false);
        }
        if !crate::workspaces::is_managed_workspace_name(state, workspace_name) {
            return Ok(false);
        }

        // Intentional switches (set_taskspace / ensure_workspaces / monitor restore) emit
        // intermediate `workspacev2` events that queue behind the service mutex. By the
        // time we handle them, focus has usually moved on — treating those as external
        // switches causes bounce-back and taskspace feedback loops.
        if let Ok(Some(active)) = hyprland::get_active_workspace() {
            if !active.name.is_empty() && active.name != workspace_name {
                hypr_log::note(format!(
                    "skip stale workspace sync: event={workspace_name} active={}",
                    active.name
                ));
                return Ok(false);
            }
        }

        if crate::context_sync::taskspace_would_change(state, workspace_name) {
            let old_allowed = crate::workspaces::allowed_workspace_names(state);
            let old_key = state.taskspace_key();
            workspace_nav::sync_taskspace_from_external(
                state,
                workspace_name,
                &old_allowed,
                &old_key,
            )
            .map_err(|e| TskError::Other(e))?;
            self.commit_state(state, Some(StateChangeKind::Taskspace))?;
            return Ok(true);
        }

        if crate::context_sync::sync_from_workspace_name(state, workspace_name) {
            self.persist_workspace_switch(state)?;
            return Ok(true);
        }
        Ok(false)
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
            .map_err(|e| crate::error::TskError::Other(e))?;
        self.commit_state(&state, Some(StateChangeKind::Taskspace))
    }

    pub fn workspace_go(&self, relative: i32) -> Result<Option<String>> {
        let mut state = self.load_state()?;
        let name = workspace_nav::workspace_go(&mut state, relative);
        if name.is_some() {
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
        let name = workspace_nav::sync_workspace_slot(&mut state, relative);
        if name.is_some() {
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
        crate::context_sync::sync_from_workspace_name(&mut state, name);
        self.persist_workspace_switch(&state)?;
        Ok(Some(name.to_string()))
    }

    /// Move the active window to a taskspace-scoped workspace (keybind hot path).
    pub fn workspace_move_dispatch(&self, relative: i32) -> Result<Option<String>> {
        if let Some(name) = crate::workspace_slots::move_slot(relative) {
            return Ok(Some(name));
        }
        let state = self.load_state()?;
        Ok(workspace_nav::move_window_to_relative(&state, relative))
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
        let result = workspace_nav::workspace_goto_name(&mut state, name);
        if result.is_some() {
            self.persist_workspace_switch(&state)?;
        }
        Ok(result)
    }

    /// Sync window home tags from Hyprland and move misplaced windows back.
    pub fn restore_windows(&self, dry_run: bool) -> Result<crate::window_registry::RestoreReport> {
        let mut state = self.load_state()?;
        let report = crate::window_registry::restore_windows(&mut state, dry_run)?;
        if !dry_run {
            self.commit_state(&state, Some(StateChangeKind::Full))?;
        }
        Ok(report)
    }

    /// Refresh the window registry from Hyprland (tags new windows at their current workspace).
    pub fn sync_window_registry(&self) -> Result<usize> {
        let mut state = self.load_state()?;
        let count = crate::window_registry::sync_window_registry(&mut state)?;
        self.registry.save_state(&state)?;
        Ok(count)
    }

    pub fn create_task(
        &self,
        name: &str,
        switch: bool,
        repo: crate::task_repo::TaskRepoSource,
        cwd: Option<&Path>,
        repo_options: crate::task_repo::TaskRepoOptions,
    ) -> Result<Task> {
        let mut state = self.load_state()?;
        let active_count = state
            .tasks
            .values()
            .filter(|t| t.status != TaskStatus::Archived)
            .count();
        if active_count >= self.config.max_tasks as usize {
            return Err(TskError::Other(format!(
                "Maximum task limit ({}) reached",
                self.config.max_tasks
            )));
        }

        let task_id = self.registry.unique_task_id(&state, name);
        let task_home = self.config.tasks_base_dir.join(&task_id);

        let resolved = repo.resolve(&task_home, cwd, &repo_options)?;
        let repo_path = resolved.checkout_path.clone();
        let repo_url = repo.resolve_url();

        let agent_dir = task_home.join(".tsk");
        std::fs::create_dir_all(&agent_dir).map_err(|source| TskError::Write {
            path: agent_dir.clone(),
            source,
        })?;
        let notes_path = agent_dir.join("agent-notes.md");
        if !notes_path.is_file() {
            std::fs::write(
                &notes_path,
                format!("# {name}\n\nTask notes for agent and human.\n"),
            )
            .map_err(|source| TskError::Write {
                path: notes_path.clone(),
                source,
            })?;
        }

        crate::task_repo::provision_task_checkout(&resolved, &task_id)?;
        let branch = crate::vcs::current_branch(&repo_path);
        let source_repo_path = match &resolved.setup {
            crate::task_repo::TaskRepoSetup::Linked { source_root, .. }
            | crate::task_repo::TaskRepoSetup::Direct { source_root } => Some(source_root.clone()),
            crate::task_repo::TaskRepoSetup::Scratch => None,
        };

        let now = Utc::now();
        let mut task = Task {
            id: task_id.clone(),
            name: name.to_string(),
            status: TaskStatus::Active,
            repo_url,
            repo_path,
            source_repo_path,
            branch,
            container_name: format!("{}-{task_id}", self.config.container_prefix),
            container_isolation: repo_options.container_isolation,
            workspace_count: self.config.default_workspace_count,
            browser_profile: None,
            created_at: now,
            last_active_at: now,
            agent_notes_path: Some(notes_path),
            ports: vec![],
        };

        if repo_options.container_isolation {
            let image = self.config.distrobox_image.trim();
            if image.is_empty() {
                return Err(TskError::Other(
                    "Distrobox isolation requested, but no image is configured \
                     ([distrobox].image is empty). Set a supported Distrobox image for this host \
                     (Arch: quay.io/toolbx/arch-toolbox:latest)."
                        .into(),
                ));
            }
            if !repo_options.defer_container_create {
                crate::distrobox::create_container(
                    &task.container_name,
                    &task_home,
                    &self.config.distrobox_image,
                )?;
            }
        }

        self.registry.touch_task(&mut task);
        state.tasks.insert(task_id.clone(), task.clone());

        // Deferred Distrobox create also defers switch: `switch_task` closes the TUI
        // (`close_tsk_tui_windows`), which would kill the setup progress UI mid-create.
        // Never provision Hyprland workspaces here — `switch_task` / `set_taskspace` does
        // that. Calling `ensure_workspaces` from create switches focus and races with the
        // workspacev2 listener.
        let do_switch = switch && !repo_options.defer_container_create;
        let run_hook = !repo_options.defer_container_create;

        if do_switch {
            self.commit_state(&state, Some(StateChangeKind::Full))?;
            let task = self.switch_task(&task_id)?;
            if run_hook {
                if let Err(err) = crate::task_on_start::run_on_create_after_create(
                    &task,
                    &resolved.setup,
                    self.config.hyprland_enabled,
                    &state,
                ) {
                    eprintln!("tsk: on_create hook: {err}");
                }
            }
            Ok(task)
        } else {
            self.commit_state(&state, Some(StateChangeKind::Full))?;
            if run_hook {
                if let Err(err) = crate::task_on_start::run_on_create_after_create(
                    &task,
                    &resolved.setup,
                    self.config.hyprland_enabled,
                    &state,
                ) {
                    eprintln!("tsk: on_create hook: {err}");
                }
            }
            Ok(task)
        }
    }

    /// Run the create hook after deferred Distrobox setup finishes.
    pub fn run_on_create_hook(&self, task_id: &str) -> Result<()> {
        let state = self.load_state()?;
        let task = state
            .tasks
            .get(task_id)
            .cloned()
            .ok_or_else(|| TskError::Other(format!("Unknown task: {task_id}")))?;
        let setup = crate::task_on_start::setup_for_task(&task, &self.config);
        crate::task_on_start::run_on_create_after_create(
            &task,
            &setup,
            self.config.hyprland_enabled,
            &state,
        )
    }

    /// Validate and leave the taskspace when archiving the active task. Holds the service lock.
    pub fn prepare_archive(&self, task_id: &str) -> Result<(Task, crate::config::TskConfig)> {
        let mut state = self.load_state()?;
        let task = state
            .tasks
            .get(task_id)
            .cloned()
            .ok_or_else(|| TskError::Other(format!("Unknown task: {task_id}")))?;
        if task.status == TaskStatus::Archived {
            return Err(TskError::Other(format!("Task is already archived: {task_id}")));
        }

        let was_current = crate::task_cleanup::is_active_task_context(&state, &task);
        if was_current {
            workspace_nav::set_taskspace(&mut state, ContextMode::Default, None)
                .map_err(TskError::Other)?;
            self.commit_state(&state, Some(StateChangeKind::Taskspace))?;
        }

        Ok((task, self.config.clone()))
    }

    /// Mark a task archived after teardown. Holds the service lock.
    pub fn complete_archive(&self, task_id: &str) -> Result<()> {
        let mut state = self.load_state()?;
        if !state.tasks.contains_key(task_id) {
            return Err(TskError::Other(format!("Unknown task: {task_id}")));
        }
        crate::task_cleanup::purge_task_windows(&mut state, task_id);
        if let Some(entry) = state.tasks.get_mut(task_id) {
            entry.status = TaskStatus::Archived;
        }
        self.commit_state(&state, Some(StateChangeKind::Full))
    }

    pub fn archive_task(&self, task_id: &str) -> Result<()> {
        let (task, config) = self.prepare_archive(task_id)?;
        crate::task_cleanup::run_archive_teardown(&config, &task)?;
        self.complete_archive(task_id)
    }

    pub fn restore_task(&self, task_id: &str) -> Result<()> {
        let mut state = self.load_state()?;
        let task = state
            .tasks
            .get(task_id)
            .cloned()
            .ok_or_else(|| TskError::Other(format!("Unknown task: {task_id}")))?;
        if task.status != TaskStatus::Archived {
            return Err(TskError::Other(format!("Task is not archived: {task_id}")));
        }

        let data_dir = crate::task_cleanup::task_data_dir(&self.config, task_id);
        if !data_dir.exists() {
            return Err(TskError::Other(format!(
                "Task data directory is missing: {}",
                data_dir.display()
            )));
        }

        if let Err(err) = crate::task_cleanup::reattach_task_checkout(&self.config, &task) {
            return Err(err);
        }
        if task.container_isolation {
            if !crate::distrobox::container_exists(&task.container_name) {
                let task_home = crate::task_cleanup::task_data_dir(&self.config, &task.id);
                if let Err(err) = crate::distrobox::create_container(
                    &task.container_name,
                    &task_home,
                    &self.config.distrobox_image,
                ) {
                    eprintln!(
                        "tsk: restore task {}: create container {}: {err}",
                        task.id, task.container_name
                    );
                }
            }
            if let Err(err) = crate::task_cleanup::start_task_container(&task) {
                eprintln!(
                    "tsk: restore task {}: start container {}: {err}",
                    task.id, task.container_name
                );
            }
        }

        if let Some(entry) = state.tasks.get_mut(task_id) {
            entry.status = TaskStatus::Active;
            entry.last_active_at = Utc::now();
        }

        self.commit_state(&state, Some(StateChangeKind::Full))?;

        if let Err(err) = crate::task_on_start::run_on_restore_after_restore(
            &task,
            &self.config,
            self.config.hyprland_enabled,
            &state,
        ) {
            eprintln!("tsk: on_restore hook: {err}");
        }

        Ok(())
    }

    pub fn delete_task(&self, task_id: &str) -> Result<()> {
        let mut state = self.load_state()?;
        let task = state
            .tasks
            .get(task_id)
            .cloned()
            .ok_or_else(|| TskError::Other(format!("Unknown task: {task_id}")))?;

        if crate::task_cleanup::is_active_task_context(&state, &task) {
            workspace_nav::set_taskspace(&mut state, ContextMode::Default, None)
                .map_err(TskError::Other)?;
            self.commit_state(&state, Some(StateChangeKind::Taskspace))?;
        }

        let _closed = crate::task_cleanup::close_task_windows(&self.config, &task)?;
        if let Err(err) = crate::task_cleanup::stop_task_container(&task) {
            eprintln!(
                "tsk: delete task {}: stop container {}: {err}",
                task.id, task.container_name
            );
        }
        if let Err(err) = crate::task_cleanup::remove_task_container(&task) {
            eprintln!(
                "tsk: delete task {}: remove container {}: {err}",
                task.id, task.container_name
            );
        }
        if let Err(err) = crate::task_cleanup::remove_task_checkout(&self.config, &task) {
            eprintln!("tsk: delete task {}: remove checkout: {err}", task.id);
        }
        if let Err(err) = crate::task_cleanup::remove_task_data_dir(&self.config, &task) {
            eprintln!("tsk: delete task {}: remove data dir: {err}", task.id);
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
            .ok_or_else(|| TskError::Other(format!("Unknown task: {task_id}")))?;
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

    pub fn open_terminal(&self, task_id: Option<&str>, host: bool) -> Result<()> {
        let mut state = self.load_state()?;
        if host {
            let env = crate::task_env::build_default_taskspace_env();
            return crate::terminal::launch_host_terminal(None, &env);
        }

        if let Some(tid) = task_id {
            let task = state
                .tasks
                .get(tid)
                .cloned()
                .ok_or_else(|| TskError::Other(format!("Unknown task: {tid}")))?;
            let env = crate::task_env::build_task_env(&state, &task, &self.config.tasks_base_dir, None);
            return crate::terminal::launch_task_terminal(&task, &env);
        }

        crate::context_sync::sync_from_active_workspace(&mut state);

        if state.context_mode == ContextMode::Task {
            if let Some(tid) = state.current_task_id.as_deref() {
                if let Some(task) = state.tasks.get(tid) {
                    let env = crate::task_env::build_task_env(
                        &state,
                        task,
                        &self.config.tasks_base_dir,
                        None,
                    );
                    return crate::terminal::launch_task_terminal(task, &env);
                }
            }
        }

        let env = crate::task_env::build_taskspace_env(&state, &self.config.tasks_base_dir);
        crate::terminal::launch_host_terminal(None, &env)
    }

    fn resolve_task_for_launch(
        &self,
        state: &mut SessionState,
        task_id: Option<&str>,
    ) -> Result<Task> {
        if let Some(tid) = task_id {
            return state
                .tasks
                .get(tid)
                .cloned()
                .ok_or_else(|| TskError::Other(format!("Unknown task: {tid}")));
        }
        crate::context_sync::sync_from_active_workspace(state);
        if state.context_mode == ContextMode::Task {
            if let Some(tid) = state.current_task_id.as_deref() {
                if let Some(task) = state.tasks.get(tid) {
                    return Ok(task.clone());
                }
            }
        }
        Err(TskError::Other(
            "not in a task taskspace — switch to a task first or pass a task id".into(),
        ))
    }

    pub fn open_editor(&self, task_id: Option<&str>) -> Result<()> {
        let mut state = self.load_state()?;
        let task = self.resolve_task_for_launch(&mut state, task_id)?;
        crate::apps::launch_task_editor(&task, &state)
    }

    pub fn open_browser(&self, task_id: Option<&str>) -> Result<()> {
        let mut state = self.load_state()?;
        let task = self.resolve_task_for_launch(&mut state, task_id)?;
        crate::apps::launch_task_browser(&task, &state)
    }

    pub fn switch_task(&self, task_id: &str) -> Result<Task> {
        let mut state = self.load_state()?;
        let task = state
            .tasks
            .get(task_id)
            .cloned()
            .ok_or_else(|| crate::error::TskError::Other(format!("Unknown task: {task_id}")))?;
        if task.status == TaskStatus::Archived {
            return Err(crate::error::TskError::Other(format!(
                "Task is archived: {task_id}"
            )));
        }
        let mut task = task;
        task.status = TaskStatus::Active;
        task.last_active_at = Utc::now();
        state.tasks.insert(task.id.clone(), task.clone());

        workspace_nav::set_taskspace(&mut state, ContextMode::Task, Some(task_id))
            .map_err(|e| crate::error::TskError::Other(e))?;
        state
            .last_workspace
            .entry(format!("task:{task_id}"))
            .or_insert(1);

        self.commit_state(&state, Some(StateChangeKind::Taskspace))?;
        Ok(task)
    }

    pub fn resolve_task(&self, name_or_id: &str) -> Result<Task> {
        let state = self.load_state()?;
        match self.registry.lookup_task(&state, name_or_id) {
            crate::task_ids::TaskLookup::Found(task) => Ok(task.clone()),
            crate::task_ids::TaskLookup::NotFound => {
                Err(crate::error::TskError::Other(format!("Unknown task: {name_or_id}")))
            }
            crate::task_ids::TaskLookup::AmbiguousPrefix(ids) => {
                let hints: Vec<String> = ids
                    .iter()
                    .map(|id| crate::task_ids::short_task_id(&state, id))
                    .collect();
                Err(crate::error::TskError::Other(format!(
                    "Ambiguous task prefix '{name_or_id}': matches {}",
                    hints.join(", ")
                )))
            }
        }
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
    use crate::config::TskConfig;
    use tempfile::tempdir;

    fn test_service(dir: &std::path::Path) -> TaskService {
        let mut config = TskConfig::default();
        config.tasks_base_dir = dir.join("tasks");
        config.hyprland_enabled = false;
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
                crate::task_repo::TaskRepoOptions::default(),
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
        assert!(!task.repo_path.join(".git").exists());
        assert_eq!(
            task.repo_path,
            dir.path().join("tasks").join(&task.id).join("workspace")
        );

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
                crate::task_repo::TaskRepoOptions::default(),
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
                crate::task_repo::TaskRepoOptions::default(),
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
    fn restore_task_reactivates_archived_task() {
        let dir = tempdir().unwrap();
        let svc = test_service(dir.path());
        let task = svc
            .create_task(
                "paused",
                false,
                crate::task_repo::TaskRepoSource::Scratch,
                None,
                crate::task_repo::TaskRepoOptions::default(),
            )
            .unwrap();
        svc.archive_task(&task.id).unwrap();
        assert!(svc.tasks_for_menu().unwrap().iter().all(|t| t.id != task.id));

        svc.restore_task(&task.id).unwrap();
        let state = svc.load_state().unwrap();
        assert_eq!(state.tasks.get(&task.id).unwrap().status, TaskStatus::Active);
        assert!(svc.tasks_for_menu().unwrap().iter().any(|t| t.id == task.id));
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
                crate::task_repo::TaskRepoOptions::default(),
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
                crate::task_repo::TaskRepoOptions::default(),
            )
            .unwrap();
        svc.delete_task(&task.id).unwrap();
        let state = svc.load_state().unwrap();
        assert!(!state.tasks.contains_key(&task.id));
        assert_eq!(state.context_mode, ContextMode::Default);
        assert!(state.current_task_id.is_none());
    }

    #[test]
    fn create_task_with_path_creates_linked_worktree() {
        let dir = tempdir().unwrap();
        let checkout = dir.path().join("checkout");
        crate::vcs::init_scratch_repo(&checkout).unwrap();
        run_git_commit(&checkout);
        let svc = test_service(dir.path());
        let task = svc
            .create_task(
                "My Feature",
                false,
                crate::task_repo::TaskRepoSource::Path(checkout.clone()),
                None,
                crate::task_repo::TaskRepoOptions::default(),
            )
            .unwrap();
        assert_eq!(task.name, "My Feature");
        assert!(task.id.starts_with('t'));
        let expected_repo = dir
            .path()
            .join("tasks")
            .join(&task.id)
            .join("workspace")
            .join("checkout");
        assert_eq!(task.repo_path, expected_repo);
        assert!(expected_repo.join(".git").is_file());
        assert_eq!(
            crate::vcs::detect_vcs_root(Some(&expected_repo)).as_deref(),
            Some(expected_repo.as_path())
        );
    }

    #[test]
    fn create_task_with_deferred_container_skips_switch() {
        let dir = tempdir().unwrap();
        let svc = test_service(dir.path());
        let task = svc
            .create_task(
                "Isolated",
                true,
                crate::task_repo::TaskRepoSource::Scratch,
                None,
                crate::task_repo::TaskRepoOptions {
                    create_worktree: true,
                    container_isolation: true,
                    defer_container_create: true,
                },
            )
            .unwrap();
        assert!(task.container_isolation);
        let state = svc.load_state().unwrap();
        // Switch must wait until Distrobox setup finishes (TUI progress UI).
        assert_eq!(state.context_mode, ContextMode::Default);
        assert!(state.current_task_id.is_none());
        assert!(state.tasks.contains_key(&task.id));
    }

    #[test]
    fn create_task_with_path_without_worktree_uses_main_repo() {
        let dir = tempdir().unwrap();
        let checkout = dir.path().join("checkout");
        crate::vcs::init_scratch_repo(&checkout).unwrap();
        run_git_commit(&checkout);
        let svc = test_service(dir.path());
        let task = svc
            .create_task(
                "Direct",
                false,
                crate::task_repo::TaskRepoSource::Path(checkout.clone()),
                None,
                crate::task_repo::TaskRepoOptions {
                    create_worktree: false,
                    container_isolation: false,
                defer_container_create: false,
                },
            )
            .unwrap();
        assert_eq!(task.repo_path, checkout);
        assert_eq!(task.source_repo_path.as_deref(), Some(checkout.as_path()));
    }

    fn run_git_commit(repo: &std::path::Path) {
        let path = repo.to_str().unwrap();
        std::process::Command::new("git")
            .args(["-C", path, "config", "user.email", "tsk@test"])
            .status()
            .unwrap();
        std::process::Command::new("git")
            .args(["-C", path, "config", "user.name", "tsk"])
            .status()
            .unwrap();
        std::process::Command::new("git")
            .args(["-C", path, "commit", "--allow-empty", "-m", "init"])
            .status()
            .unwrap();
    }
}
