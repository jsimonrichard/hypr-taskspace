use std::path::PathBuf;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use lae_core::{
    collect_task_repo_paths, detect_vcs_root, ensure_daemon, load_repos, paths_match,
    repo_display_path, unregister_repo, ContextMode, DaemonClient, RegisteredRepo, Result, Task,
    TaskRepoSource,
};
use crate::grep_dir_picker::{GrepDirPicker, PickerAction};
use crate::modal::{arrow_nav_delta, ModalButtonAction, ModalButtonBar};
use crate::new_task_form::{cycle_form_focus, initial_form_focus, NewTaskFormFocus};
use crate::ui;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Panel {
    Tasks,
    Repos,
}

#[derive(Debug, Clone)]
pub enum ListEntry {
    Header { label: String },
    Task(TaskRow),
}

#[derive(Debug, Clone)]
pub struct TaskRow {
    pub id: String,
    pub name: String,
    pub current: bool,
    pub is_default: bool,
    pub is_archived: bool,
}

#[derive(Debug, Clone)]
pub struct RepoChoice {
    pub repo: Option<PathBuf>,
    pub label: String,
}

pub enum Screen {
    Main,
    RepoPicker {
        picker: GrepDirPicker,
    },
    NewTaskPickRepo {
        choices: Vec<RepoChoice>,
        list_state: ratatui::widgets::ListState,
        buttons: ModalButtonBar,
        actions_focused: bool,
    },
    NewTaskName {
        name: String,
        repo: TaskRepoSource,
        repo_label: String,
        create_worktree: bool,
        buttons: ModalButtonBar,
        focus: NewTaskFormFocus,
    },
    ConfirmDeleteRepo {
        repo_path: PathBuf,
        repo_name: String,
        buttons: ModalButtonBar,
    },
    ConfirmArchive {
        task_id: String,
        task_name: String,
        window_count: usize,
        container_exists: bool,
        data_dir: String,
        buttons: ModalButtonBar,
    },
    ConfirmDelete {
        task_id: String,
        task_name: String,
        window_count: usize,
        container_exists: bool,
        data_dir: String,
        is_archived: bool,
        buttons: ModalButtonBar,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonStatus {
    Unknown,
    Running,
    Stopped,
}

pub struct App {
    pub client: DaemonClient,
    pub panel: Panel,
    pub repos: Vec<RegisteredRepo>,
    pub entries: Vec<ListEntry>,
    pub list_state: ratatui::widgets::ListState,
    pub repo_list_state: ratatui::widgets::ListState,
    pub screen: Screen,
    pub status: Option<(bool, String)>,
    pub should_quit: bool,
    pub daemon_status: DaemonStatus,
    pub(crate) daemon_recheck_requested: bool,
    default_taskspace_active: bool,
}

impl App {
    pub fn new(client: DaemonClient) -> Result<Self> {
        let mut app = Self {
            client,
            panel: Panel::Tasks,
            repos: Vec::new(),
            entries: Vec::new(),
            list_state: ratatui::widgets::ListState::default(),
            repo_list_state: ratatui::widgets::ListState::default(),
            screen: Screen::Main,
            status: None,
            should_quit: false,
            daemon_status: DaemonStatus::Unknown,
            daemon_recheck_requested: false,
            default_taskspace_active: true,
        };
        app.reload()?;
        Ok(app)
    }

    pub fn show_daemon_warning(&self) -> bool {
        self.daemon_status == DaemonStatus::Stopped
    }

    pub fn set_daemon_status(&mut self, running: bool) {
        self.daemon_status = if running {
            DaemonStatus::Running
        } else {
            DaemonStatus::Stopped
        };
    }

    fn require_daemon(&self) -> Result<()> {
        match self.daemon_status {
            DaemonStatus::Stopped => Err(lae_core::LaeError::Other(
                "lae daemon is not running — run `lae daemon start`".into(),
            )),
            DaemonStatus::Running => Ok(()),
            DaemonStatus::Unknown => ensure_daemon(),
        }
    }

    fn refresh_repos(&mut self, task_paths: impl IntoIterator<Item = PathBuf>) -> Result<()> {
        self.repos = load_repos(task_paths)?;
        Ok(())
    }

    pub fn reload(&mut self) -> Result<()> {
        self.daemon_recheck_requested = true;
        let svc = self.client.direct();
        let state = svc.load_state()?;
        self.default_taskspace_active = state.context_mode == ContextMode::Default;
        let current_task = state.current_task_id.as_deref();

        let active_tasks = svc.list_active_tasks()?;
        let archived_tasks = svc.list_archived_tasks()?;
        let task_paths = collect_task_repo_paths(
            active_tasks
                .iter()
                .chain(archived_tasks.iter())
                .map(|t| {
                    t.source_repo_path
                        .as_deref()
                        .unwrap_or(t.repo_path.as_path())
                }),
        );
        self.refresh_repos(task_paths)?;

        let prev_task_id = self
            .selected_task()
            .map(|t| t.id.clone())
            .filter(|id| !id.is_empty());
        let prev_repo_sel = self.repo_list_state.selected();

        let mut matched = std::collections::HashSet::new();

        self.entries.clear();
        self.entries.push(ListEntry::Header {
            label: "host".into(),
        });
        self.entries.push(ListEntry::Task(TaskRow {
            id: String::new(),
            name: "default taskspace".into(),
            current: self.default_taskspace_active,
            is_default: true,
            is_archived: false,
        }));

        for repo in &self.repos {
            let mut repo_tasks: Vec<&Task> = active_tasks
                .iter()
                .filter(|t| {
                    let key = t
                        .source_repo_path
                        .as_deref()
                        .unwrap_or(t.repo_path.as_path());
                    paths_match(key, &repo_display_path(repo))
                })
                .collect();
            repo_tasks.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

            self.entries.push(ListEntry::Header {
                label: repo.name.clone(),
            });
            for task in repo_tasks {
                matched.insert(task.id.clone());
                self.entries.push(ListEntry::Task(TaskRow {
                    id: task.id.clone(),
                    name: task.name.clone(),
                    current: current_task == Some(task.id.as_str()),
                    is_default: false,
                    is_archived: false,
                }));
            }
        }

        let mut scratch: Vec<&Task> = active_tasks
            .iter()
            .filter(|t| !matched.contains(&t.id))
            .collect();
        if !scratch.is_empty() {
            scratch.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
            self.entries.push(ListEntry::Header {
                label: "scratch".into(),
            });
            for task in scratch {
                self.entries.push(ListEntry::Task(TaskRow {
                        id: task.id.clone(),
                        name: task.name.clone(),
                        current: current_task == Some(task.id.as_str()),
                        is_default: false,
                    is_archived: false,
                }));
            }
        }

        if !archived_tasks.is_empty() {
            let mut archived = archived_tasks;
            archived.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
            self.entries.push(ListEntry::Header {
                label: "archived".into(),
            });
            for task in archived {
                self.entries.push(ListEntry::Task(TaskRow {
                    id: task.id.clone(),
                    name: task.name.clone(),
                    current: false,
                    is_default: false,
                    is_archived: true,
                }));
            }
        }

        let new_sel = prev_task_id
            .and_then(|id| {
                self.entries.iter().position(|entry| {
                    matches!(entry, ListEntry::Task(t) if t.id == id)
                })
            })
            .or_else(|| {
                self.entries.iter().position(|entry| {
                    matches!(entry, ListEntry::Task(t) if t.current)
                })
            })
            .or_else(|| self.first_selectable());

        self.list_state.select(new_sel);
        self.ensure_selection_on_task();

        let repo_sel = prev_repo_sel
            .filter(|i| *i < self.repos.len())
            .or_else(|| default_repo_selection(&self.repos).or_else(|| (!self.repos.is_empty()).then_some(0)));
        self.repo_list_state.select(repo_sel);

        Ok(())
    }

    pub fn handle_event(&mut self, event: Event) -> Result<()> {
        if let Screen::RepoPicker { .. } = &self.screen {
            let key = match event {
                Event::Key(key) if key.kind == crossterm::event::KeyEventKind::Press => key,
                _ => return Ok(()),
            };
            let action = if let Screen::RepoPicker { picker } = &mut self.screen {
                picker.handle_key(key)?
            } else {
                return Ok(());
            };
            match action {
                PickerAction::Continue => {}
                PickerAction::Cancel => self.screen = Screen::Main,
                PickerAction::Register => {
                    let cwd = if let Screen::RepoPicker { picker } = &self.screen {
                        picker.cwd().to_path_buf()
                    } else {
                        return Ok(());
                    };
                    self.finish_repo_picker(&cwd)?;
                }
            }
            return Ok(());
        }

        if let Event::Key(key) = event {
            if key.kind == crossterm::event::KeyEventKind::Press {
                self.handle_key(key)?;
            }
        }
        Ok(())
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Result<()> {
        match &mut self.screen {
            Screen::Main => self.handle_main_key(key),
            Screen::NewTaskPickRepo { .. } => self.handle_new_task_pick_repo_key(key),
            Screen::NewTaskName { .. } => self.handle_new_task_name_key(key),
            Screen::ConfirmDeleteRepo { .. } => self.handle_confirm_delete_repo_key(key),
            Screen::ConfirmArchive { .. } => self.handle_confirm_archive_key(key),
            Screen::ConfirmDelete { .. } => self.handle_confirm_delete_key(key),
            Screen::RepoPicker { .. } => Ok(()),
        }
    }

    fn handle_main_key(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Tab | KeyCode::Char('l') | KeyCode::Right => {
                self.panel = Panel::Repos;
                return Ok(());
            }
            KeyCode::BackTab | KeyCode::Char('h') | KeyCode::Left => {
                self.panel = Panel::Tasks;
                return Ok(());
            }
            _ => {}
        }
        match self.panel {
            Panel::Tasks => self.handle_tasks_panel_key(key)?,
            Panel::Repos => self.handle_repos_panel_key(key)?,
        }
        Ok(())
    }

    fn handle_tasks_panel_key(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => self.should_quit = true,
            KeyCode::Char('j') | KeyCode::Down => self.select_next(),
            KeyCode::Char('k') | KeyCode::Up => self.select_prev(),
            KeyCode::Char('n') => self.begin_new_task()?,
            KeyCode::Char('r') => {
                self.reload()?;
                self.status = Some((true, "Refreshed".into()));
            }
            KeyCode::Char('d') => self.begin_archive()?,
            KeyCode::Char('D') => self.begin_delete()?,
            KeyCode::Enter => self.switch_selected_task()?,
            _ => {}
        }
        Ok(())
    }

    fn handle_repos_panel_key(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => self.should_quit = true,
            KeyCode::Char('j') | KeyCode::Down => self.select_repo_next(),
            KeyCode::Char('k') | KeyCode::Up => self.select_repo_prev(),
            KeyCode::Char('n') => self.begin_repo_picker()?,
            KeyCode::Char('d') => self.begin_delete_repo(),
            KeyCode::Char('r') => {
                self.reload()?;
                self.status = Some((true, "Refreshed".into()));
            }
            _ => {}
        }
        Ok(())
    }

    fn begin_repo_picker(&mut self) -> Result<()> {
        self.status = None;
        let picker = GrepDirPicker::new(None)?;
        self.screen = Screen::RepoPicker { picker };
        Ok(())
    }

    fn finish_repo_picker(&mut self, cwd: &std::path::Path) -> Result<()> {
        match GrepDirPicker::register_at_cwd(&self.repos, cwd) {
            Ok(repo) => {
                self.reload()?;
                self.status = Some((true, format!("Registered {}", repo.name)));
                self.screen = Screen::Main;
                self.panel = Panel::Repos;
            }
            Err(err) => {
                self.status = Some((false, err.to_string()));
                self.screen = Screen::Main;
                self.panel = Panel::Repos;
            }
        }
        Ok(())
    }

    fn begin_new_task(&mut self) -> Result<()> {
        self.require_daemon()?;
        self.status = None;
        let mut choices = vec![RepoChoice {
            repo: None,
            label: "No repo (scratch workspace)".into(),
        }];
        for repo in &self.repos {
            choices.push(RepoChoice {
                repo: Some(repo_display_path(repo)),
                label: format!("{}  {}", repo.name, repo.path.display()),
            });
        }
        let mut list_state = ratatui::widgets::ListState::default();
        list_state.select(Some(default_new_task_choice(&choices).unwrap_or(0)));
        self.screen = Screen::NewTaskPickRepo {
            choices,
            list_state,
            buttons: ModalButtonBar::cancel_continue(),
            actions_focused: false,
        };
        Ok(())
    }

    fn handle_new_task_pick_repo_key(&mut self, key: KeyEvent) -> Result<()> {
        if matches!(
            key.code,
            KeyCode::Char('j') | KeyCode::Char('k') | KeyCode::Up | KeyCode::Down
        ) {
            if let Screen::NewTaskPickRepo {
                actions_focused,
                ..
            } = &mut self.screen
            {
                *actions_focused = false;
            }
        }

        if let Some(delta) = arrow_nav_delta(key) {
            if let Screen::NewTaskPickRepo {
                buttons,
                actions_focused,
                ..
            } = &mut self.screen
            {
                if *actions_focused {
                    buttons.navigate(delta);
                } else {
                    *actions_focused = true;
                    buttons.enter_bar();
                }
            }
            return Ok(());
        }

        if let Screen::NewTaskPickRepo {
            actions_focused,
            ..
        } = &mut self.screen
        {
            if *actions_focused {
                return self.handle_pick_repo_buttons(key);
            }
        }

        let Screen::NewTaskPickRepo { choices, list_state, .. } = &mut self.screen else {
            return Ok(());
        };
        match key.code {
            KeyCode::Esc => self.screen = Screen::Main,
            KeyCode::Char('j') | KeyCode::Down => {
                if choices.is_empty() {
                    return Ok(());
                }
                let i = match list_state.selected() {
                    Some(i) => (i + 1).min(choices.len() - 1),
                    None => 0,
                };
                list_state.select(Some(i));
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if choices.is_empty() {
                    return Ok(());
                }
                let i = match list_state.selected() {
                    Some(i) => i.saturating_sub(1),
                    None => 0,
                };
                list_state.select(Some(i));
            }
            KeyCode::Enter => self.advance_new_task_from_repo_pick()?,
            _ => {}
        }
        Ok(())
    }

    fn handle_pick_repo_buttons(&mut self, key: KeyEvent) -> Result<()> {
        if matches!(
            key.code,
            KeyCode::Char('j') | KeyCode::Char('k') | KeyCode::Up | KeyCode::Down
        ) {
            if let Screen::NewTaskPickRepo {
                actions_focused,
                ..
            } = &mut self.screen
            {
                *actions_focused = false;
            }
            return Ok(());
        }

        let action = match &mut self.screen {
            Screen::NewTaskPickRepo { buttons, .. } => buttons.handle_key(key),
            _ => return Ok(()),
        };
        match action {
            Some(ModalButtonAction::Cancel) => self.screen = Screen::Main,
            Some(ModalButtonAction::Confirm) => self.advance_new_task_from_repo_pick()?,
            None => {}
        }
        Ok(())
    }

    fn advance_new_task_from_repo_pick(&mut self) -> Result<()> {
        let Screen::NewTaskPickRepo { choices, list_state, .. } = &self.screen else {
            return Ok(());
        };
        let Some(sel) = list_state.selected() else {
            return Ok(());
        };
        let Some(choice) = choices.get(sel).cloned() else {
            return Ok(());
        };
        let create_worktree = choice.repo.is_some();
        let (repo, repo_label_text) = match choice.repo {
            None => (
                TaskRepoSource::Scratch,
                "Scratch workspace".into(),
            ),
            Some(path) => (
                TaskRepoSource::Path(path),
                choice.label,
            ),
        };
        let focus = initial_form_focus(&repo);
        self.screen = Screen::NewTaskName {
            name: String::new(),
            repo,
            repo_label: repo_label_text,
            create_worktree,
            buttons: ModalButtonBar::cancel_create(),
            focus,
        };
        Ok(())
    }

    fn move_new_task_form_focus(&mut self, delta: i32) {
        let Screen::NewTaskName {
            repo,
            focus,
            buttons,
            ..
        } = &mut self.screen
        else {
            return;
        };
        *focus = cycle_form_focus(*focus, repo, delta);
        if *focus == NewTaskFormFocus::Buttons {
            buttons.enter_bar();
        }
    }

    fn handle_new_task_name_key(&mut self, key: KeyEvent) -> Result<()> {
        let Screen::NewTaskName { focus, .. } = &self.screen else {
            return Ok(());
        };
        let current_focus = *focus;

        if key.code == KeyCode::Esc {
            self.screen = Screen::Main;
            return Ok(());
        }

        if matches!(
            key.code,
            KeyCode::Tab | KeyCode::BackTab | KeyCode::Up | KeyCode::Down
        ) {
            let delta = match key.code {
                KeyCode::Tab | KeyCode::Down => 1,
                KeyCode::BackTab | KeyCode::Up => -1,
                _ => unreachable!(),
            };
            self.move_new_task_form_focus(delta);
            return Ok(());
        }

        match current_focus {
            NewTaskFormFocus::Worktree => match key.code {
                KeyCode::Char(' ') => {
                    if let Screen::NewTaskName { create_worktree, .. } = &mut self.screen {
                        *create_worktree = !*create_worktree;
                    }
                }
                KeyCode::Enter => self.move_new_task_form_focus(1),
                _ => {}
            },
            NewTaskFormFocus::Name => match key.code {
                KeyCode::Enter => self.submit_new_task_name()?,
                KeyCode::Backspace => {
                    if let Screen::NewTaskName { name, .. } = &mut self.screen {
                        name.pop();
                    }
                }
                KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    if let Screen::NewTaskName { name, .. } = &mut self.screen {
                        name.push(ch);
                    }
                }
                _ => {}
            },
            NewTaskFormFocus::Buttons => {
                if let Some(delta) = arrow_nav_delta(key) {
                    if let Screen::NewTaskName { buttons, .. } = &mut self.screen {
                        buttons.navigate(delta);
                    }
                    return Ok(());
                }
                if let Screen::NewTaskName { buttons, .. } = &mut self.screen {
                    match buttons.handle_key(key) {
                        Some(ModalButtonAction::Cancel) => self.screen = Screen::Main,
                        Some(ModalButtonAction::Confirm) => self.submit_new_task_name()?,
                        None => {}
                    }
                }
            }
        }
        Ok(())
    }

    fn submit_new_task_name(&mut self) -> Result<()> {
        let (name, repo, create_worktree) = match &self.screen {
            Screen::NewTaskName {
                name,
                repo,
                create_worktree,
                ..
            } => (name.trim().to_string(), repo.clone(), *create_worktree),
            _ => return Ok(()),
        };
        if name.is_empty() {
            self.status = Some((false, "Task name cannot be empty".into()));
            self.screen = Screen::Main;
            return Ok(());
        }
        let repo_options = lae_core::TaskRepoOptions { create_worktree };
        match self.client.create_task(&name, true, repo, repo_options) {
            Ok(_task) => {
                self.should_quit = true;
            }
            Err(err) => {
                self.status = Some((false, err.to_string()));
                self.screen = Screen::Main;
            }
        }
        Ok(())
    }

    fn begin_delete_repo(&mut self) {
        let Some(repo) = self.selected_repo().cloned() else {
            self.status = Some((false, "Select a repo to remove".into()));
            return;
        };
        self.status = None;
        self.screen = Screen::ConfirmDeleteRepo {
            repo_path: repo_display_path(&repo),
            repo_name: repo.name,
            buttons: ModalButtonBar::cancel_confirm("Remove"),
        };
    }

    fn handle_confirm_delete_repo_key(&mut self, key: KeyEvent) -> Result<()> {
        let (repo_path, repo_name, action) = match &mut self.screen {
            Screen::ConfirmDeleteRepo {
                repo_path,
                repo_name,
                buttons,
            } => (
                repo_path.clone(),
                repo_name.clone(),
                buttons.handle_key(key),
            ),
            _ => return Ok(()),
        };

        match action {
            Some(ModalButtonAction::Cancel) => self.screen = Screen::Main,
            Some(ModalButtonAction::Confirm) => {
                unregister_repo(&repo_path)?;
                self.reload()?;
                self.status = Some((true, format!("Removed {repo_name}")));
                self.screen = Screen::Main;
            }
            None => {}
        }
        Ok(())
    }

    fn handle_confirm_archive_key(&mut self, key: KeyEvent) -> Result<()> {
        let (task_id, task_name, action) = match &mut self.screen {
            Screen::ConfirmArchive {
                task_id,
                task_name,
                buttons,
                ..
            } => (
                task_id.clone(),
                task_name.clone(),
                buttons.handle_key(key),
            ),
            _ => return Ok(()),
        };

        match action {
            Some(ModalButtonAction::Cancel) => self.screen = Screen::Main,
            Some(ModalButtonAction::Confirm) => {
                match self.client.archive_task(&task_id) {
                    Ok(()) => {
                        self.reload()?;
                        self.status = Some((true, format!("Archived {task_name}")));
                    }
                    Err(err) => {
                        self.status = Some((false, err.to_string()));
                    }
                }
                self.screen = Screen::Main;
            }
            None => {}
        }
        Ok(())
    }

    fn handle_confirm_delete_key(&mut self, key: KeyEvent) -> Result<()> {
        let (task_id, task_name, action) = match &mut self.screen {
            Screen::ConfirmDelete {
                task_id,
                task_name,
                buttons,
                ..
            } => (
                task_id.clone(),
                task_name.clone(),
                buttons.handle_key(key),
            ),
            _ => return Ok(()),
        };

        match action {
            Some(ModalButtonAction::Cancel) => self.screen = Screen::Main,
            Some(ModalButtonAction::Confirm) => {
                match self.client.delete_task(&task_id) {
                    Ok(()) => {
                        self.reload()?;
                        self.status = Some((true, format!("Deleted {task_name}")));
                    }
                    Err(err) => {
                        self.status = Some((false, err.to_string()));
                    }
                }
                self.screen = Screen::Main;
            }
            None => {}
        }
        Ok(())
    }

    fn select_repo_next(&mut self) {
        if self.repos.is_empty() {
            return;
        }
        let i = match self.repo_list_state.selected() {
            Some(i) => (i + 1).min(self.repos.len() - 1),
            None => 0,
        };
        self.repo_list_state.select(Some(i));
    }

    fn select_repo_prev(&mut self) {
        if self.repos.is_empty() {
            return;
        }
        let i = match self.repo_list_state.selected() {
            Some(i) => i.saturating_sub(1),
            None => 0,
        };
        self.repo_list_state.select(Some(i));
    }

    fn selected_repo(&self) -> Option<&RegisteredRepo> {
        self.repo_list_state
            .selected()
            .and_then(|i| self.repos.get(i))
    }

    fn first_selectable(&self) -> Option<usize> {
        self.entries
            .iter()
            .position(|entry| matches!(entry, ListEntry::Task(_)))
    }

    fn next_selectable(&self, from: usize) -> Option<usize> {
        ((from + 1)..self.entries.len()).find(|&i| matches!(self.entries[i], ListEntry::Task(_)))
    }

    fn prev_selectable(&self, from: usize) -> Option<usize> {
        (0..from)
            .rev()
            .find(|&i| matches!(self.entries[i], ListEntry::Task(_)))
    }

    fn ensure_selection_on_task(&mut self) {
        let Some(sel) = self.list_state.selected() else {
            if let Some(i) = self.first_selectable() {
                self.list_state.select(Some(i));
            }
            return;
        };
        if matches!(self.entries.get(sel), Some(ListEntry::Task(_))) {
            return;
        }
        if let Some(i) = self.next_selectable(sel).or_else(|| self.prev_selectable(sel)) {
            self.list_state.select(Some(i));
        } else if let Some(i) = self.first_selectable() {
            self.list_state.select(Some(i));
        } else {
            self.list_state.select(None);
        }
    }

    fn select_next(&mut self) {
        let Some(from) = self.list_state.selected() else {
            if let Some(i) = self.first_selectable() {
                self.list_state.select(Some(i));
            }
            return;
        };
        if let Some(i) = self.next_selectable(from) {
            self.list_state.select(Some(i));
        }
    }

    fn select_prev(&mut self) {
        let Some(from) = self.list_state.selected() else {
            if let Some(i) = self.first_selectable() {
                self.list_state.select(Some(i));
            }
            return;
        };
        if let Some(i) = self.prev_selectable(from) {
            self.list_state.select(Some(i));
        }
    }

    fn selected_task(&self) -> Option<&TaskRow> {
        self.list_state
            .selected()
            .and_then(|i| self.entries.get(i))
            .and_then(|entry| match entry {
                ListEntry::Task(task) => Some(task),
                ListEntry::Header { .. } => None,
            })
    }

    fn switch_selected_task(&mut self) -> Result<()> {
        self.require_daemon()?;
        let Some(task) = self.selected_task().cloned() else {
            return Ok(());
        };
        if task.is_archived {
            self.status = Some((false, "Archived tasks cannot be switched to — delete or recreate".into()));
            return Ok(());
        }
        if task.is_default {
            self.client.context_default()?;
        } else {
            self.client.switch_task(&task.id)?;
        }
        self.should_quit = true;
        Ok(())
    }

    fn begin_archive(&mut self) -> Result<()> {
        self.require_daemon()?;
        let Some(task) = self.selected_task().cloned() else {
            return Ok(());
        };
        if task.is_default {
            self.status = Some((false, "Cannot archive the default taskspace".into()));
            return Ok(());
        }
        if task.is_archived {
            self.status = Some((false, "Task is already archived".into()));
            return Ok(());
        }
        let preview = self.client.preview_task_teardown(&task.id)?;
        self.status = None;
        self.screen = Screen::ConfirmArchive {
            task_id: task.id,
            task_name: task.name,
            window_count: preview.window_count,
            container_exists: preview.container_exists,
            data_dir: preview.data_dir.display().to_string(),
            buttons: ModalButtonBar::confirm_first("Archive"),
        };
        Ok(())
    }

    fn begin_delete(&mut self) -> Result<()> {
        self.require_daemon()?;
        let Some(task) = self.selected_task().cloned() else {
            return Ok(());
        };
        if task.is_default {
            self.status = Some((false, "Cannot delete the default taskspace".into()));
            return Ok(());
        }
        let preview = self.client.preview_task_teardown(&task.id)?;
        self.status = None;
        self.screen = Screen::ConfirmDelete {
            task_id: task.id,
            task_name: task.name,
            window_count: preview.window_count,
            container_exists: preview.container_exists,
            data_dir: preview.data_dir.display().to_string(),
            is_archived: task.is_archived,
            buttons: ModalButtonBar::cancel_confirm("Delete"),
        };
        Ok(())
    }

    pub fn draw(&mut self, frame: &mut ratatui::Frame) {
        ui::draw(frame, self);
    }
}

fn default_repo_selection(repos: &[RegisteredRepo]) -> Option<usize> {
    let cwd = std::env::current_dir().ok()?;
    let root = detect_vcs_root(Some(&cwd))?;
    repos
        .iter()
        .position(|repo| paths_match(&repo.path, &root))
}

fn default_new_task_choice(choices: &[RepoChoice]) -> Option<usize> {
    let cwd = std::env::current_dir().ok()?;
    let root = detect_vcs_root(Some(&cwd))?;
    choices.iter().position(|choice| {
        choice
            .repo
            .as_ref()
            .is_some_and(|path| paths_match(path, &root))
    })
}
