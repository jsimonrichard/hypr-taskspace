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
    },
    CancelContinue {
        focused: usize,
    },
    CancelCreate {
        focused: usize,
    },
    CancelSave {
        focused: usize,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModalButtonAction {
    Cancel,
    Confirm,
}

impl ModalButtonBar {
    pub fn cancel_confirm(confirm_label: &'static str) -> Self {
        Self::CancelConfirm {
            confirm_label,
            focused: 0,
        }
    }

    pub fn cancel_continue() -> Self {
        Self::CancelContinue { focused: 0 }
    }

    pub fn cancel_create() -> Self {
        Self::CancelCreate { focused: 0 }
    }

    pub fn cancel_save() -> Self {
        Self::CancelSave { focused: 0 }
    }

    pub fn buttons(&self) -> Vec<ModalButton> {
        match self {
            Self::CancelConfirm { confirm_label, .. } => vec![
                ModalButton {
                    label: "Cancel",
                    shortcut: 'n',
                },
                ModalButton {
                    label: confirm_label,
                    shortcut: 'y',
                },
            ],
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
            Self::CancelCreate { .. } => vec![
                ModalButton {
                    label: "Cancel",
                    shortcut: 'n',
                },
                ModalButton {
                    label: "Create",
                    shortcut: 'y',
                },
            ],
            Self::CancelSave { .. } => vec![
                ModalButton {
                    label: "Cancel",
                    shortcut: 'n',
                },
                ModalButton {
                    label: "Save",
                    shortcut: 'y',
                },
            ],
        }
    }

    pub fn focused(&self) -> usize {
        match self {
            Self::CancelConfirm { focused, .. }
            | Self::CancelContinue { focused, .. }
            | Self::CancelCreate { focused, .. }
            | Self::CancelSave { focused, .. } => *focused,
        }
    }

    pub fn set_focus(&mut self, index: usize) {
        let count = self.buttons().len();
        let index = if count == 0 { 0 } else { index % count };
        match self {
            Self::CancelConfirm { focused, .. }
            | Self::CancelContinue { focused, .. }
            | Self::CancelCreate { focused, .. }
            | Self::CancelSave { focused, .. } => *focused = index,
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
        if self.focused() == 0 {
            ModalButtonAction::Cancel
        } else {
            ModalButtonAction::Confirm
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
                if lower == 'n' {
                    self.set_focus(0);
                    Some(ModalButtonAction::Cancel)
                } else if lower == 'y' {
                    self.set_focus(1);
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
