use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use lae_core::{detect_vcs_root, normalize_repo_path, register_repo, RegisteredRepo, Result};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;

#[derive(Debug, Clone)]
struct PickerEntry {
    label: String,
    path: PathBuf,
    is_dir: bool,
}

pub struct GrepDirPicker {
    cwd: PathBuf,
    filter: String,
    entries: Vec<PickerEntry>,
    list_state: ListState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerAction {
    Continue,
    Cancel,
    Register,
}

impl GrepDirPicker {
    pub fn new(start: Option<PathBuf>) -> Result<Self> {
        let cwd = start
            .or_else(|| detect_vcs_root(None))
            .or_else(|| std::env::current_dir().ok())
            .ok_or_else(|| lae_core::LaeError::Other("Could not determine start directory".into()))?;
        let mut picker = Self {
            cwd: normalize_repo_path(&cwd),
            filter: String::new(),
            entries: Vec::new(),
            list_state: ListState::default(),
        };
        picker.refresh()?;
        Ok(picker)
    }

    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Result<PickerAction> {
        if key.code == KeyCode::Esc {
            return Ok(PickerAction::Cancel);
        }
        if key.modifiers.intersects(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return Ok(PickerAction::Cancel);
        }

        match key.code {
            KeyCode::Enter if register_modifiers(&key.modifiers) => {
                return Ok(PickerAction::Register);
            }
            KeyCode::Char('y') if key.modifiers.intersects(KeyModifiers::CONTROL) => {
                return Ok(PickerAction::Register);
            }
            KeyCode::Char('j') | KeyCode::Char('m')
                if key.modifiers.intersects(KeyModifiers::CONTROL) =>
            {
                return Ok(PickerAction::Register);
            }
            KeyCode::Down => self.select_next(),
            KeyCode::Up => self.select_prev(),
            KeyCode::Char('j') if self.filter.is_empty() => self.select_next(),
            KeyCode::Char('k') if self.filter.is_empty() => self.select_prev(),
            KeyCode::Enter => self.enter_selected()?,
            KeyCode::Backspace => {
                if self.filter.pop().is_some() {
                    self.refresh()?;
                } else {
                    self.go_up()?;
                }
            }
            KeyCode::Char(ch) if !key.modifiers.intersects(KeyModifiers::CONTROL) => {
                self.filter.push(ch);
                self.refresh()?;
            }
            _ => {}
        }
        Ok(PickerAction::Continue)
    }

    pub fn register_at_cwd(existing: &[RegisteredRepo], cwd: &Path) -> Result<RegisteredRepo> {
        let root = detect_vcs_root(Some(cwd)).ok_or_else(|| {
            lae_core::LaeError::Other(format!(
                "No git or jj repo at {} — open a checkout folder first",
                cwd.display()
            ))
        })?;
        register_repo(&root, existing)
    }

    fn enter_selected(&mut self) -> Result<()> {
        let Some(entry) = self.selected_entry().cloned() else {
            return Ok(());
        };
        if !entry.is_dir {
            return Ok(());
        }
        self.cwd = normalize_repo_path(&entry.path);
        self.filter.clear();
        self.refresh()?;
        Ok(())
    }

    fn go_up(&mut self) -> Result<()> {
        let Some(parent) = self.cwd.parent() else {
            return Ok(());
        };
        self.cwd = parent.to_path_buf();
        self.filter.clear();
        self.refresh()?;
        Ok(())
    }

    fn refresh(&mut self) -> Result<()> {
        self.entries.clear();
        let filtering = !self.filter.is_empty();
        if !filtering {
            if let Some(parent) = self.cwd.parent() {
                self.entries.push(PickerEntry {
                    label: "../".into(),
                    path: parent.to_path_buf(),
                    is_dir: true,
                });
            }
        }

        let needle = self.filter.to_lowercase();
        let mut dirs = Vec::new();
        for entry in std::fs::read_dir(&self.cwd).map_err(|source| lae_core::LaeError::Read {
            path: self.cwd.clone(),
            source,
        })? {
            let entry = entry.map_err(|source| lae_core::LaeError::Read {
                path: self.cwd.clone(),
                source,
            })?;
            let path = entry.path();
            let file_type = entry.file_type().map_err(|source| lae_core::LaeError::Read {
                path: path.clone(),
                source,
            })?;
            if !file_type.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if filtering && !name.to_lowercase().contains(&needle) {
                continue;
            }
            dirs.push(PickerEntry {
                label: format!("{name}/"),
                path,
                is_dir: true,
            });
        }

        if filtering {
            dirs.sort_by(|a, b| {
                let name_a = entry_name(&a.label);
                let name_b = entry_name(&b.label);
                match (
                    match_score(name_a, &needle),
                    match_score(name_b, &needle),
                ) {
                    (Some(sa), Some(sb)) => sa.cmp(&sb).then_with(|| a.label.cmp(&b.label)),
                    _ => a.label.cmp(&b.label),
                }
            });
        } else {
            dirs.sort_by(|a, b| a.label.to_lowercase().cmp(&b.label.to_lowercase()));
        }
        self.entries.extend(dirs);

        if self.entries.is_empty() {
            self.list_state.select(None);
        } else {
            self.list_state.select(Some(0));
        }
        Ok(())
    }

    fn selected_entry(&self) -> Option<&PickerEntry> {
        self.list_state
            .selected()
            .and_then(|i| self.entries.get(i))
    }

    fn select_next(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        let i = match self.list_state.selected() {
            Some(i) => (i + 1).min(self.entries.len() - 1),
            None => 0,
        };
        self.list_state.select(Some(i));
    }

    fn select_prev(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        let i = match self.list_state.selected() {
            Some(i) => i.saturating_sub(1),
            None => 0,
        };
        self.list_state.select(Some(i));
    }
}

pub fn draw(frame: &mut Frame, picker: &mut GrepDirPicker) {
    let area = frame.area();
    frame.render_widget(Clear, area);

    let vcs_hint = vcs_label(picker.cwd());
    let block = Block::default()
        .title(format!(" Register repo — {} ({vcs_hint}) ", picker.cwd().display()))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Green));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .split(inner);

    let filter_line = if picker.filter.is_empty() {
        Line::from(vec![
            Span::styled("Filter: ", Style::default().fg(Color::DarkGray)),
            Span::styled("_", Style::default().fg(Color::Cyan)),
        ])
    } else {
        Line::from(vec![
            Span::styled("Filter: ", Style::default().fg(Color::DarkGray)),
            Span::styled(picker.filter.clone(), Style::default().fg(Color::Cyan)),
            Span::styled("_", Style::default().fg(Color::Cyan)),
        ])
    };
    frame.render_widget(Paragraph::new(filter_line), chunks[0]);

    if picker.entries.is_empty() {
        frame.render_widget(
            Paragraph::new("No matching folders")
                .wrap(Wrap { trim: true })
                .style(Style::default().fg(Color::DarkGray)),
            chunks[1],
        );
    } else {
        let items: Vec<ListItem> = picker
            .entries
            .iter()
            .map(|entry| {
                ListItem::new(if entry.label == "../" {
                    Line::from(Span::styled(
                        entry.label.clone(),
                        Style::default().fg(Color::DarkGray),
                    ))
                } else {
                    Line::from(entry.label.clone())
                })
            })
            .collect();
        let list = List::new(items)
            .highlight_style(
                Style::default()
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("▸ ");
        frame.render_stateful_widget(list, chunks[1], &mut picker.list_state);
    }

    let help = Paragraph::new("↑/↓ move · Enter open folder · Backspace delete/up · Esc cancel")
        .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(help, chunks[3]);

    let register_line = if vcs_hint == "no repo" {
        Line::from(Span::styled(
            "Open a git or jj checkout, then register",
            Style::default().fg(Color::Yellow),
        ))
    } else {
        Line::from(vec![
            Span::styled("▸ Register: ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
            Span::styled(
                "Ctrl+Enter",
                Style::default().fg(Color::Black).bg(Color::Green).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" or ", Style::default().fg(Color::Green)),
            Span::styled(
                "Ctrl+Y",
                Style::default().fg(Color::Black).bg(Color::Green).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  ({vcs_hint} repo at this path)"),
                Style::default().fg(Color::DarkGray),
            ),
        ])
    };
    frame.render_widget(Paragraph::new(register_line), chunks[2]);
}

fn register_modifiers(modifiers: &KeyModifiers) -> bool {
    modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
}

fn vcs_label(path: &Path) -> &'static str {
    match detect_vcs_root(Some(path)) {
        Some(root) if root.join(".jj").is_dir() => "jj",
        Some(_) => "git",
        None => "no repo",
    }
}

fn entry_name(label: &str) -> &str {
    label.strip_suffix('/').unwrap_or(label)
}

/// Lower is a better match.
fn match_score(name: &str, needle: &str) -> Option<u32> {
    let name = name.to_lowercase();
    if name == needle {
        return Some(0);
    }
    if name.starts_with(needle) {
        return Some(100 + name.len() as u32);
    }
    let pos = name.find(needle)?;
    Some(1000 + pos as u32 + name.len() as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn match_score_prefers_exact_then_prefix() {
        assert!(match_score("local", "local").unwrap() < match_score("local-agentic-env", "local").unwrap());
        assert!(
            match_score("local-agentic-env", "local").unwrap()
                < match_score("my-local-app", "local").unwrap()
        );
    }

    #[test]
    fn register_key_bindings() {
        use crossterm::event::KeyModifiers;
        assert!(register_modifiers(&KeyModifiers::CONTROL));
        assert!(register_modifiers(&KeyModifiers::ALT));
        assert!(!register_modifiers(&KeyModifiers::empty()));
    }
}
