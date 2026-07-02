use std::collections::HashSet;
use std::path::Path;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use lae_core::{
    load_repos, paths_match, repo_display_path, save_repos, unique_repo_id, ContextMode, DaemonClient,
    RegisteredRepo, Result,
};
use lae_core::Task;

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
}

#[derive(Debug, Clone)]
pub struct RepoChoice {
    pub repo_id: Option<String>,
    pub label: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepoFormField {
    Name,
    Path,
    Url,
}

pub enum Screen {
    Main,
    NewTaskPickRepo {
        choices: Vec<RepoChoice>,
        list_state: ratatui::widgets::ListState,
    },
    NewTaskName {
        name: String,
        repo_id: Option<String>,
        repo_label: String,
    },
    RepoForm {
        name: String,
        path: String,
        url: String,
        focus: RepoFormField,
        editing_id: Option<String>,
    },
    ConfirmDeleteRepo {
        repo_id: String,
        repo_name: String,
    },
    ConfirmArchive {
        task_id: String,
        task_name: String,
    },
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
    default_taskspace_active: bool,
}

impl App {
    pub fn new(client: DaemonClient) -> Result<Self> {
        let mut app = Self {
            client,
            panel: Panel::Tasks,
            repos: load_repos()?,
            entries: Vec::new(),
            list_state: ratatui::widgets::ListState::default(),
            repo_list_state: ratatui::widgets::ListState::default(),
            screen: Screen::Main,
            status: None,
            should_quit: false,
            default_taskspace_active: true,
        };
        app.reload()?;
        Ok(app)
    }

    pub fn reload(&mut self) -> Result<()> {
        self.repos = load_repos()?;
        let state = self.client.load_state()?;
        self.default_taskspace_active = state.context_mode == ContextMode::Default;
        let current_task = state.current_task_id.as_deref();

        let prev_task_id = self
            .selected_task()
            .map(|t| t.id.clone())
            .filter(|id| !id.is_empty());
        let prev_repo_sel = self.repo_list_state.selected();

        let active_tasks = self.client.list_active_tasks()?;
        let mut matched = HashSet::new();

        self.entries.clear();
        self.entries.push(ListEntry::Header {
            label: "host".into(),
        });
        self.entries.push(ListEntry::Task(TaskRow {
            id: String::new(),
            name: "default taskspace".into(),
            current: self.default_taskspace_active,
            is_default: true,
        }));

        for repo in &self.repos {
            let mut repo_tasks: Vec<&Task> = active_tasks
                .iter()
                .filter(|t| paths_match(&t.repo_path, &repo_display_path(repo)))
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
            .or_else(|| (!self.repos.is_empty()).then_some(0));
        self.repo_list_state.select(repo_sel);

        Ok(())
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Result<()> {
        match &mut self.screen {
            Screen::Main => self.handle_main_key(key),
            Screen::NewTaskPickRepo { .. } => self.handle_new_task_pick_repo_key(key),
            Screen::NewTaskName { .. } => self.handle_new_task_name_key(key),
            Screen::RepoForm { .. } => self.handle_repo_form_key(key),
            Screen::ConfirmDeleteRepo { .. } => self.handle_confirm_delete_repo_key(key),
            Screen::ConfirmArchive { .. } => self.handle_confirm_archive_key(key),
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
            KeyCode::Char('n') => self.begin_new_repo(),
            KeyCode::Char('e') => self.begin_edit_repo(),
            KeyCode::Char('d') => self.begin_delete_repo(),
            KeyCode::Char('r') => {
                self.reload()?;
                self.status = Some((true, "Refreshed".into()));
            }
            _ => {}
        }
        Ok(())
    }

    fn begin_new_task(&mut self) -> Result<()> {
        self.status = None;
        let mut choices = vec![RepoChoice {
            repo_id: None,
            label: "No repo (scratch workspace)".into(),
        }];
        for repo in &self.repos {
            choices.push(RepoChoice {
                repo_id: Some(repo.id.clone()),
                label: format!("{}  {}", repo.name, repo.path.display()),
            });
        }
        let mut list_state = ratatui::widgets::ListState::default();
        list_state.select(Some(0));
        self.screen = Screen::NewTaskPickRepo {
            choices,
            list_state,
        };
        Ok(())
    }

    fn handle_new_task_pick_repo_key(&mut self, key: KeyEvent) -> Result<()> {
        let Screen::NewTaskPickRepo { choices, list_state } = &mut self.screen else {
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
            KeyCode::Enter => {
                let Some(sel) = list_state.selected() else {
                    return Ok(());
                };
                let Some(choice) = choices.get(sel).cloned() else {
                    return Ok(());
                };
                self.screen = Screen::NewTaskName {
                    name: String::new(),
                    repo_id: choice.repo_id,
                    repo_label: choice.label,
                };
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_new_task_name_key(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc => self.screen = Screen::Main,
            KeyCode::Enter => {
                let (name, repo_id) = match &self.screen {
                    Screen::NewTaskName { name, repo_id, .. } => {
                        (name.trim().to_string(), repo_id.clone())
                    }
                    _ => return Ok(()),
                };
                if name.is_empty() {
                    self.status = Some((false, "Task name cannot be empty".into()));
                    self.screen = Screen::Main;
                    return Ok(());
                }
                match self
                    .client
                    .create_task(&name, true, repo_id.as_deref())
                {
                    Ok(_task) => {
                        self.should_quit = true;
                    }
                    Err(err) => {
                        self.status = Some((false, err.to_string()));
                        self.screen = Screen::Main;
                    }
                }
            }
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
        }
        Ok(())
    }

    fn begin_new_repo(&mut self) {
        self.status = None;
        self.screen = Screen::RepoForm {
            name: String::new(),
            path: String::new(),
            url: String::new(),
            focus: RepoFormField::Path,
            editing_id: None,
        };
    }

    fn begin_edit_repo(&mut self) {
        let Some(repo) = self.selected_repo().cloned() else {
            self.status = Some((false, "Select a repo to edit".into()));
            return;
        };
        self.status = None;
        self.screen = Screen::RepoForm {
            name: repo.name,
            path: repo.path.to_string_lossy().into_owned(),
            url: repo.url.unwrap_or_default(),
            focus: RepoFormField::Name,
            editing_id: Some(repo.id),
        };
    }

    fn begin_delete_repo(&mut self) {
        let Some(repo) = self.selected_repo().cloned() else {
            self.status = Some((false, "Select a repo to delete".into()));
            return;
        };
        self.status = None;
        self.screen = Screen::ConfirmDeleteRepo {
            repo_id: repo.id,
            repo_name: repo.name,
        };
    }

    fn handle_repo_form_key(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc => self.screen = Screen::Main,
            KeyCode::Tab => self.cycle_repo_form_focus(1),
            KeyCode::BackTab => self.cycle_repo_form_focus(-1),
            KeyCode::Enter => self.save_repo_form()?,
            KeyCode::Backspace => self.repo_form_pop(),
            KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.repo_form_push(ch);
            }
            _ => {}
        }
        Ok(())
    }

    fn cycle_repo_form_focus(&mut self, delta: i8) {
        let Screen::RepoForm { focus, .. } = &mut self.screen else {
            return;
        };
        *focus = match (*focus, delta) {
            (RepoFormField::Name, 1) | (RepoFormField::Path, -1) => RepoFormField::Path,
            (RepoFormField::Path, 1) | (RepoFormField::Url, -1) => RepoFormField::Url,
            (RepoFormField::Url, 1) | (RepoFormField::Name, -1) => RepoFormField::Name,
            (f, _) => f,
        };
    }

    fn repo_form_push(&mut self, ch: char) {
        let Screen::RepoForm {
            name,
            path,
            url,
            focus,
            ..
        } = &mut self.screen
        else {
            return;
        };
        match focus {
            RepoFormField::Name => name.push(ch),
            RepoFormField::Path => path.push(ch),
            RepoFormField::Url => url.push(ch),
        }
    }

    fn repo_form_pop(&mut self) {
        let Screen::RepoForm {
            name,
            path,
            url,
            focus,
            ..
        } = &mut self.screen
        else {
            return;
        };
        match focus {
            RepoFormField::Name => {
                name.pop();
            }
            RepoFormField::Path => {
                path.pop();
            }
            RepoFormField::Url => {
                url.pop();
            }
        }
    }

    fn save_repo_form(&mut self) -> Result<()> {
        let Screen::RepoForm {
            name,
            path,
            url,
            editing_id,
            ..
        } = &self.screen
        else {
            return Ok(());
        };

        let path = path.trim();
        if path.is_empty() {
            self.status = Some((false, "Repo path is required".into()));
            return Ok(());
        }

        let display_name = if name.trim().is_empty() {
            Path::new(path)
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "repo".into())
        } else {
            name.trim().to_string()
        };

        let id = editing_id
            .clone()
            .unwrap_or_else(|| unique_repo_id(&self.repos, &display_name));

        let repo = RegisteredRepo {
            id: id.clone(),
            name: display_name,
            path: Path::new(path).into(),
            url: if url.trim().is_empty() {
                None
            } else {
                Some(url.trim().to_string())
            },
        };

        if let Some(i) = self.repos.iter().position(|r| r.id == id) {
            self.repos[i] = repo;
        } else {
            self.repos.push(repo);
        }
        self.repos.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        save_repos(&self.repos)?;
        self.reload()?;
        self.status = Some((true, "Saved repo".into()));
        self.screen = Screen::Main;
        self.panel = Panel::Repos;
        Ok(())
    }

    fn handle_confirm_delete_repo_key(&mut self, key: KeyEvent) -> Result<()> {
        let (repo_id, repo_name) = match &self.screen {
            Screen::ConfirmDeleteRepo {
                repo_id,
                repo_name,
            } => (repo_id.clone(), repo_name.clone()),
            _ => return Ok(()),
        };

        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                self.repos.retain(|r| r.id != repo_id);
                save_repos(&self.repos)?;
                self.reload()?;
                self.status = Some((true, format!("Removed {repo_name}")));
                self.screen = Screen::Main;
            }
            KeyCode::Char('n') | KeyCode::Esc => {
                self.screen = Screen::Main;
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_confirm_archive_key(&mut self, key: KeyEvent) -> Result<()> {
        let (task_id, task_name) = match &self.screen {
            Screen::ConfirmArchive {
                task_id,
                task_name,
            } => (task_id.clone(), task_name.clone()),
            _ => return Ok(()),
        };

        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
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
            KeyCode::Char('n') | KeyCode::Esc => {
                self.screen = Screen::Main;
            }
            _ => {}
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
        let Some(task) = self.selected_task().cloned() else {
            return Ok(());
        };
        if task.is_default {
            self.client.context_default()?;
        } else {
            self.client.switch_task(&task.id)?;
        }
        self.should_quit = true;
        Ok(())
    }

    fn begin_archive(&mut self) -> Result<()> {
        let Some(task) = self.selected_task().cloned() else {
            return Ok(());
        };
        if task.is_default {
            self.status = Some((false, "Cannot archive the default taskspace".into()));
            return Ok(());
        }
        self.status = None;
        self.screen = Screen::ConfirmArchive {
            task_id: task.id,
            task_name: task.name,
        };
        Ok(())
    }

    pub fn draw(&mut self, frame: &mut ratatui::Frame) {
        ui::draw(frame, self);
    }
}
