mod app;
mod modal;
mod ui;

use std::io::{self, stdout, Stdout};
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyEventKind};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::{execute, terminal::ClearType};
use lae_core::{DaemonClient, Result};
use ratatui::prelude::*;

use app::App;

struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> io::Result<(Self, Terminal<CrosstermBackend<Stdout>>)> {
        enable_raw_mode()?;
        execute!(stdout(), EnterAlternateScreen, crossterm::cursor::Hide)?;
        let backend = CrosstermBackend::new(stdout());
        let terminal = Terminal::new(backend)?;
        Ok((Self, terminal))
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(stdout(), LeaveAlternateScreen, crossterm::cursor::Show);
    }
}

/// Run the interactive task manager TUI in the current terminal.
pub fn run() -> Result<()> {
    let (_guard, mut terminal) =
        TerminalGuard::enter().map_err(|e| lae_core::LaeError::Other(e.to_string()))?;

    let client = DaemonClient::with_defaults()?;
    let mut app = App::new(client)?;

    let tick = Duration::from_millis(250);
    let mut last_tick = Instant::now();

    loop {
        terminal
            .draw(|frame| app.draw(frame))
            .map_err(|e| lae_core::LaeError::Other(e.to_string()))?;

        let timeout = tick.saturating_sub(last_tick.elapsed());
        if event::poll(timeout).map_err(|e| lae_core::LaeError::Other(e.to_string()))? {
            match event::read().map_err(|e| lae_core::LaeError::Other(e.to_string()))? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    if let Err(err) = app.handle_key(key) {
                        app.status = Some((false, err.to_string()));
                    }
                }
                Event::Resize(_, _) => {
                    let _ = execute!(stdout(), crossterm::terminal::Clear(ClearType::All));
                }
                _ => {}
            }
        }

        if last_tick.elapsed() >= tick {
            last_tick = Instant::now();
        }

        if app.should_quit {
            break;
        }
    }

    Ok(())
}
