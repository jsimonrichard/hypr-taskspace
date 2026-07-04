use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::{App, ListEntry, Panel, Screen, TaskRow};
use crate::grep_dir_picker;
use crate::modal::{draw_button_bar, ModalButtonBar};

pub fn draw(frame: &mut Frame, app: &mut App) {
    match &mut app.screen {
        Screen::RepoPicker { picker } => {
            grep_dir_picker::draw(frame, picker);
            return;
        }
        _ => {}
    }

    let area = frame.area();
    frame.render_widget(Clear, area);

    let header_rows = if app.show_daemon_warning() { 2 } else { 1 };
    let chunks = Layout::vertical([
        Constraint::Length(header_rows),
        Constraint::Min(3),
        Constraint::Length(2),
        Constraint::Length(1),
    ])
    .split(area);

    if app.show_daemon_warning() {
        let header = Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).split(chunks[0]);
        draw_tabs(frame, header[0], app);
        draw_daemon_warning(frame, header[1]);
    } else {
        draw_tabs(frame, chunks[0], app);
    }
    match app.panel {
        Panel::Tasks => draw_task_list(frame, chunks[1], app),
        Panel::Repos => draw_repo_list(frame, chunks[1], app),
    }
    draw_help(frame, chunks[2], app);
    draw_status(frame, chunks[3], app);

    match &app.screen {
        Screen::NewTaskPickRepo { .. } => draw_new_task_pick_repo(frame, area, app),
        Screen::NewTaskName { .. } => draw_new_task_name(frame, area, app),
        Screen::ConfirmDeleteRepo { .. } => draw_confirm_delete_repo(frame, area, app),
        Screen::ConfirmArchive { .. } => draw_confirm_archive(frame, area, app),
        Screen::ConfirmDelete { .. } => draw_confirm_delete(frame, area, app),
        Screen::Main | Screen::RepoPicker { .. } => {}
    }
}

fn draw_tabs(frame: &mut Frame, area: Rect, app: &App) {
    let tasks_style = if app.panel == Panel::Tasks {
        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let repos_style = if app.panel == Panel::Repos {
        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let line = Line::from(vec![
        Span::styled(" Tasks ", tasks_style),
        Span::raw("  "),
        Span::styled(" Repos ", repos_style),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

fn draw_daemon_warning(frame: &mut Frame, area: Rect) {
    let line = Line::from(Span::styled(
        " ⚠  Daemon not running — run `lae daemon start`, then press r to refresh",
        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
    ));
    frame.render_widget(Paragraph::new(line), area);
}

fn panel_block() -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
}

fn draw_task_list(frame: &mut Frame, area: Rect, app: &mut App) {
    let block = panel_block();

    if app.entries.is_empty() {
        frame.render_widget(
            Paragraph::new("No tasks — press n to create one")
                .block(block)
                .wrap(Wrap { trim: true }),
            area,
        );
        return;
    }

    let items: Vec<ListItem> = app
        .entries
        .iter()
        .map(|entry| match entry {
            ListEntry::Header { label } => ListItem::new(header_line(label)).style(
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            ),
            ListEntry::Task(task) => ListItem::new(task_line(task)),
        })
        .collect();

    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▸ ");

    frame.render_stateful_widget(list, area, &mut app.list_state);
}

fn draw_repo_list(frame: &mut Frame, area: Rect, app: &mut App) {
    let block = panel_block();

    if app.repos.is_empty() {
        frame.render_widget(
            Paragraph::new("No repos — press n to browse and register a checkout")
                .block(block)
                .wrap(Wrap { trim: true }),
            area,
        );
        return;
    }

    let items: Vec<ListItem> = app
        .repos
        .iter()
        .map(|repo| {
            let url = repo
                .url
                .as_deref()
                .map(|u| format!("  {u}"))
                .unwrap_or_default();
            ListItem::new(Line::from(vec![
                Span::raw("󰉋 "),
                Span::styled(
                    repo.name.clone(),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::raw(format!("  {}{url}", repo.path.display())),
            ]))
        })
        .collect();

    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▸ ");

    frame.render_stateful_widget(list, area, &mut app.repo_list_state);
}

fn header_line(label: &str) -> Line<'static> {
    Line::from(vec![
        Span::raw("  "),
        Span::styled(
            label.to_uppercase(),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        ),
    ])
}

fn task_line(task: &TaskRow) -> Line<'static> {
    let icon = if task.is_default {
        "󰣇"
    } else if task.is_archived {
        "󰁰"
    } else {
        "󱓝"
    };
    let marker = if task.current { " ●" } else { "" };
    let name_style = if task.is_archived {
        Style::default().fg(Color::DarkGray)
    } else {
        Style::default().add_modifier(Modifier::BOLD)
    };
    Line::from(vec![
        Span::raw("    "),
        Span::raw(format!("{icon} ")),
        Span::styled(task.name.clone(), name_style),
        Span::raw(marker),
    ])
}

fn draw_help(frame: &mut Frame, area: Rect, app: &App) {
    let text = match &app.screen {
        Screen::Main if app.panel == Panel::Tasks => {
            "↑/↓ move  Enter switch  n new  d archive  D delete  r refresh  h/l Tab panels  q quit"
        }
        Screen::Main if app.panel == Panel::Repos => {
            "↑/↓ move  n browse/add  d remove  r refresh  h/l Tab panels  q quit"
        }
        Screen::Main => "",
        _ => "",
    };
    let help = Paragraph::new(text).style(Style::default().fg(Color::DarkGray));
    frame.render_widget(help, area);
}

fn draw_status(frame: &mut Frame, area: Rect, app: &App) {
    let line = if let Some((ok, ref msg)) = app.status {
        let style = if ok {
            Style::default().fg(Color::Green)
        } else {
            Style::default().fg(Color::Red)
        };
        Line::from(Span::styled(msg.clone(), style))
    } else {
        Line::from("")
    };
    frame.render_widget(Paragraph::new(line), area);
}

fn draw_modal_dialog(
    frame: &mut Frame,
    _area: Rect,
    popup: Rect,
    title: &str,
    border_color: Color,
    body: &str,
    buttons: &ModalButtonBar,
    buttons_active: bool,
) {
    frame.render_widget(Clear, popup);

    let block = Block::default()
        .title(format!(" {title} "))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let chunks = Layout::vertical([
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .split(inner);

    frame.render_widget(
        Paragraph::new(body).wrap(Wrap { trim: true }),
        chunks[0],
    );
    draw_button_bar(frame, chunks[1], buttons, buttons_active);
}

fn draw_new_task_pick_repo(frame: &mut Frame, area: Rect, app: &mut App) {
    let Screen::NewTaskPickRepo {
        choices,
        list_state,
        buttons,
        actions_focused,
    } = &mut app.screen
    else {
        return;
    };

    let popup = centered_rect(70, 60, area);
    frame.render_widget(Clear, popup);

    let block = Block::default()
        .title(" Select repo for new task ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Green));

    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let chunks = Layout::vertical([
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .split(inner);

    let items: Vec<ListItem> = choices
        .iter()
        .map(|choice| ListItem::new(choice.label.clone()))
        .collect();

    let list = List::new(items)
        .highlight_style(
            Style::default()
                .bg(if *actions_focused {
                    Color::Reset
                } else {
                    Color::DarkGray
                })
                .add_modifier(if *actions_focused {
                    Modifier::empty()
                } else {
                    Modifier::BOLD
                }),
        )
        .highlight_symbol(if *actions_focused { "  " } else { "▸ " });

    frame.render_stateful_widget(list, chunks[0], list_state);
    draw_button_bar(frame, chunks[1], buttons, *actions_focused);
}

fn draw_new_task_name(frame: &mut Frame, area: Rect, app: &App) {
    let Screen::NewTaskName {
        name,
        repo_label,
        buttons,
        actions_focused,
        ..
    } = &app.screen
    else {
        return;
    };

    let popup = centered_rect(70, 26, area);
    let body = format!("Repo: {repo_label}\n\nName: {name}_");
    draw_modal_dialog(
        frame,
        area,
        popup,
        "New task",
        Color::Green,
        &body,
        buttons,
        *actions_focused,
    );
}

fn draw_confirm_delete_repo(frame: &mut Frame, area: Rect, app: &App) {
    let Screen::ConfirmDeleteRepo { repo_name, buttons, .. } = &app.screen else {
        return;
    };

    let popup = centered_rect(70, 28, area);
    let body = format!(
        "Remove \"{repo_name}\" from lae?\n\nDeletes `.lae/repo.toml` in the checkout.\nExisting tasks are not deleted."
    );
    draw_modal_dialog(
        frame,
        area,
        popup,
        "Remove repo",
        Color::Yellow,
        &body,
        buttons,
        true,
    );
}

fn draw_confirm_archive(frame: &mut Frame, area: Rect, app: &App) {
    let Screen::ConfirmArchive {
        task_name,
        window_count,
        container_exists,
        data_dir,
        buttons,
        ..
    } = &app.screen
    else {
        return;
    };

    let windows_line = if *window_count == 1 {
        "Close 1 open window.".into()
    } else {
        format!("Close {window_count} open windows.")
    };
    let container_line = if *container_exists {
        "Stop the Distrobox container (files kept).".to_string()
    } else {
        String::new()
    };
    let mut body = format!(
        "Archive \"{task_name}\"?\n\n{windows_line}\nTask files stay at {data_dir}."
    );
    if !container_line.is_empty() {
        body.push('\n');
        body.push_str(&container_line);
    }

    let popup = centered_rect(70, 32, area);
    draw_modal_dialog(
        frame,
        area,
        popup,
        "Archive task",
        Color::Yellow,
        &body,
        buttons,
        true,
    );
}

fn draw_confirm_delete(frame: &mut Frame, area: Rect, app: &App) {
    let Screen::ConfirmDelete {
        task_name,
        window_count,
        container_exists,
        data_dir,
        is_archived,
        buttons,
        ..
    } = &app.screen
    else {
        return;
    };

    let windows_line = if *window_count == 1 {
        "Close 1 open window.".into()
    } else {
        format!("Close {window_count} open windows.")
    };
    let container_line = if *container_exists {
        "Remove the Distrobox container.".to_string()
    } else {
        String::new()
    };
    let archive_note = if *is_archived {
        String::new()
    } else {
        "This skips archive and deletes immediately.\n".to_string()
    };
    let mut body = format!(
        "{archive_note}Permanently delete \"{task_name}\"?\n\n{windows_line}\nDelete task data at {data_dir}."
    );
    if !container_line.is_empty() {
        body.push('\n');
        body.push_str(&container_line);
    }

    let popup = centered_rect(70, 36, area);
    draw_modal_dialog(
        frame,
        area,
        popup,
        "Delete task",
        Color::Red,
        &body,
        buttons,
        true,
    );
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let popup_layout = Layout::vertical([
        Constraint::Percentage((100 - percent_y) / 2),
        Constraint::Percentage(percent_y),
        Constraint::Percentage((100 - percent_y) / 2),
    ])
    .split(area);

    Layout::horizontal([
        Constraint::Percentage((100 - percent_x) / 2),
        Constraint::Percentage(percent_x),
        Constraint::Percentage((100 - percent_x) / 2),
    ])
    .split(popup_layout[1])[1]
}
