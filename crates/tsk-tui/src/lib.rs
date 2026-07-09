mod app;
mod daemon_check;
mod grep_dir_picker;
mod modal;
mod new_task_form;
mod ui;

use std::io::{self, stdout, ErrorKind, Stdout};
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::{execute, terminal::ClearType};
use tsk_core::{DaemonClient, Result};
use ratatui::prelude::*;

use app::App;
use daemon_check::AsyncDaemonChecker;

struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> io::Result<(Self, Terminal<CrosstermBackend<Stdout>>)> {
        enable_raw_mode()?;
        // DISAMBIGUATE_ESCAPE_CODES helps Ctrl+Enter report with a CONTROL modifier.
        // Do not enable REPORT_ALL_KEYS_AS_ESCAPE_CODES — it breaks Shift (keys arrive as
        // physical codes + SHIFT modifier instead of shifted characters).
        let kb_flags = KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES;
        execute!(
            stdout(),
            EnterAlternateScreen,
            crossterm::cursor::Hide,
            PushKeyboardEnhancementFlags(kb_flags),
        )?;
        let backend = CrosstermBackend::new(stdout());
        let terminal = Terminal::new(backend)?;
        Ok((Self, terminal))
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(
            stdout(),
            LeaveAlternateScreen,
            crossterm::cursor::Show,
            PopKeyboardEnhancementFlags,
        );
    }
}

/// Run the interactive task manager TUI in the current terminal.
pub fn run() -> Result<()> {
    let (_guard, mut terminal) =
        TerminalGuard::enter().map_err(|e| tsk_core::TskError::Other(e.to_string()))?;

    let client = DaemonClient::with_defaults()?;
    let mut app = App::new(client)?;
    let mut daemon_checker = AsyncDaemonChecker::new();
    daemon_checker.spawn_check();

    let tick = Duration::from_millis(250);
    let mut last_tick = Instant::now();
    let daemon_recheck_interval = Duration::from_secs(5);
    let mut last_daemon_check = Instant::now();

    loop {
        if let Some(running) = daemon_checker.poll() {
            app.set_daemon_status(running);
        }

        // Drain Distrobox setup lines every frame so progress/errors show promptly.
        app.poll_container_setup();

        loop {
            match terminal.draw(|frame| app.draw(frame)) {
                Ok(_) => break,
                Err(err) if err.kind() == ErrorKind::Interrupted => continue,
                Err(err) => return Err(tsk_core::TskError::Other(err.to_string())),
            }
        }

        let timeout = tick.saturating_sub(last_tick.elapsed());
        if poll_event(timeout)? {
            let event = read_event()?;
            if matches!(event, Event::Resize(_, _)) {
                let _ = execute!(stdout(), crossterm::terminal::Clear(ClearType::All));
            }
            if let Err(err) = app.handle_event(event) {
                app.status = Some((false, err.to_string()));
            }
            if app.daemon_recheck_requested {
                        app.daemon_recheck_requested = false;
                daemon_checker.spawn_check();
            }
        }

        if last_tick.elapsed() >= tick {
            last_tick = Instant::now();
        }

        if last_daemon_check.elapsed() >= daemon_recheck_interval {
            last_daemon_check = Instant::now();
            daemon_checker.spawn_check();
        }

        if app.should_quit {
            break;
        }
    }

    Ok(())
}

fn poll_event(timeout: Duration) -> Result<bool> {
    loop {
        match event::poll(timeout) {
            Ok(value) => return Ok(value),
            Err(err) if err.kind() == ErrorKind::Interrupted => continue,
            Err(err) => return Err(tsk_core::TskError::Other(err.to_string())),
        }
    }
}

fn read_event() -> Result<Event> {
    loop {
        match event::read() {
            Ok(event) => return Ok(event),
            Err(err) if err.kind() == ErrorKind::Interrupted => continue,
            Err(err) => return Err(tsk_core::TskError::Other(err.to_string())),
        }
    }
}
