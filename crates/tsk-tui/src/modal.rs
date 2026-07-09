//! In-dialog action buttons — rendered inside modals.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModalButton {
    pub label: &'static str,
    pub shortcut: char,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModalButtonBar {
    CancelConfirm {
        confirm_label: &'static str,
        focused: usize,
        /// When true, the confirm action is the first (default-focused) button.
        confirm_first: bool,
    },
    CancelContinue {
        focused: usize,
    },
    CancelCreate {
        focused: usize,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModalButtonAction {
    Cancel,
    Confirm,
}

impl ModalButtonBar {
    /// Confirm-first layout — for low-friction happy paths (e.g. archive).
    pub fn confirm_first(confirm_label: &'static str) -> Self {
        Self::CancelConfirm {
            confirm_label,
            focused: 0,
            confirm_first: true,
        }
    }

    pub fn cancel_continue() -> Self {
        Self::CancelContinue { focused: 0 }
    }

    pub fn cancel_create() -> Self {
        Self::CancelCreate { focused: 0 }
    }

    pub fn buttons(&self) -> Vec<ModalButton> {
        match self {
            Self::CancelConfirm {
                confirm_label,
                confirm_first,
                ..
            } => {
                let cancel = ModalButton {
                    label: "Cancel",
                    shortcut: 'n',
                };
                let confirm = ModalButton {
                    label: confirm_label,
                    shortcut: 'y',
                };
                if *confirm_first {
                    vec![confirm, cancel]
                } else {
                    vec![cancel, confirm]
                }
            }
            Self::CancelContinue { .. } => vec![
                ModalButton {
                    label: "Cancel",
                    shortcut: 'n',
                },
                ModalButton {
                    label: "Continue",
                    shortcut: 'y',
                },
            ],
            // Create-first — matches confirm_first happy-path layout.
            Self::CancelCreate { .. } => vec![
                ModalButton {
                    label: "Create",
                    shortcut: 'y',
                },
                ModalButton {
                    label: "Cancel",
                    shortcut: 'n',
                },
            ],
        }
    }

    pub fn focused(&self) -> usize {
        match self {
            Self::CancelConfirm { focused, .. }
            | Self::CancelContinue { focused, .. }
            | Self::CancelCreate { focused, .. } => *focused,
        }
    }

    pub fn set_focus(&mut self, index: usize) {
        let count = self.buttons().len();
        let index = if count == 0 { 0 } else { index % count };
        match self {
            Self::CancelConfirm { focused, .. }
            | Self::CancelContinue { focused, .. }
            | Self::CancelCreate { focused, .. } => *focused = index,
        }
    }

    pub fn cycle_focus(&mut self, delta: i32) {
        let count = self.buttons().len() as i32;
        if count <= 0 {
            return;
        }
        let next = (self.focused() as i32 + delta).rem_euclid(count);
        self.set_focus(next as usize);
    }

    /// Enter the button bar — always land on the first button.
    pub fn enter_bar(&mut self) {
        self.set_focus(0);
    }

    /// Navigate while the button bar is already active.
    pub fn navigate(&mut self, delta: i32) {
        self.cycle_focus(delta);
    }

    pub fn activate_focused(&self) -> ModalButtonAction {
        self.action_at(self.focused())
    }

    fn action_at(&self, index: usize) -> ModalButtonAction {
        let (cancel_idx, _confirm_idx) = self.action_indices();
        if index == cancel_idx {
            ModalButtonAction::Cancel
        } else {
            ModalButtonAction::Confirm
        }
    }

    fn action_indices(&self) -> (usize, usize) {
        match self {
            Self::CancelConfirm {
                confirm_first: true,
                ..
            }
            | Self::CancelCreate { .. } => (1, 0),
            Self::CancelConfirm { .. } | Self::CancelContinue { .. } => (0, 1),
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Option<ModalButtonAction> {
        match key.code {
            KeyCode::Esc => Some(ModalButtonAction::Cancel),
            KeyCode::Left | KeyCode::Char('h') => {
                self.navigate(-1);
                None
            }
            KeyCode::Right | KeyCode::Char('l') => {
                self.navigate(1);
                None
            }
            KeyCode::Enter => Some(self.activate_focused()),
            KeyCode::Char(ch) => {
                let lower = ch.to_ascii_lowercase();
                let (cancel_idx, confirm_idx) = self.action_indices();
                if lower == 'n' {
                    self.set_focus(cancel_idx);
                    Some(ModalButtonAction::Cancel)
                } else if lower == 'y' {
                    self.set_focus(confirm_idx);
                    Some(ModalButtonAction::Confirm)
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}

/// ←/→ from a text field or list — enters the button bar (first button).
pub fn arrow_nav_delta(key: KeyEvent) -> Option<i32> {
    match key.code {
        KeyCode::Left => Some(-1),
        KeyCode::Right => Some(1),
        _ => None,
    }
}

pub fn draw_button_bar(frame: &mut Frame, area: Rect, bar: &ModalButtonBar, buttons_active: bool) {
    if area.height == 0 || area.width == 0 {
        return;
    }

    let buttons = bar.buttons();
    let focused = bar.focused();
    let mut spans = Vec::new();

    for (index, button) in buttons.iter().enumerate() {
        if index > 0 {
            spans.push(Span::raw("   "));
        }

        let is_focused = buttons_active && index == focused;
        let border = if is_focused { "▐" } else { " " };
        let style = if is_focused {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        };

        let text = format!("{border} {} ({}) {border}", button.shortcut, button.label);
        spans.push(Span::styled(text, style));
    }

    let hint = if buttons_active {
        "  Esc · Enter  ←/→ · h/l"
    } else {
        "  Esc · Enter  ←/→ buttons"
    };
    spans.push(Span::styled(hint, Style::default().fg(Color::DarkGray)));

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}
