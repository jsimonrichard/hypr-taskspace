use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::{
    App, ListEntry, Panel, RepoFormField, Screen, TaskRow,
};

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    frame.render_widget(Clear, area);

    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(2),
        Constraint::Length(1),
    ])
    .split(area);

    draw_tabs(frame, chunks[0], app);
    match app.panel {
        Panel::Tasks => draw_task_list(frame, chunks[1], app),
        Panel::Repos => draw_repo_list(frame, chunks[1], app),
    }
    draw_help(frame, chunks[2], app);
    draw_status(frame, chunks[3], app);

    match &app.screen {
        Screen::NewTaskPickRepo { .. } => draw_new_task_pick_repo(frame, area, app),
        Screen::NewTaskName { .. } => draw_new_task_name(frame, area, app),
        Screen::RepoForm { .. } => draw_repo_form(frame, area, app),
        Screen::ConfirmDeleteRepo { .. } => draw_confirm_delete_repo(frame, area, app),
        Screen::ConfirmArchive { .. } => draw_confirm_archive(frame, area, app),
        Screen::Main => {}
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
            Paragraph::new("No repos configured — press n to add one")
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
    let icon = if task.is_default { "󰣇" } else { "󱓝" };
    let marker = if task.current { " ●" } else { "" };
    let detail = if task.is_default {
        String::new()
    } else {
        format!("  {}", task.id)
    };
    Line::from(vec![
        Span::raw("    "),
        Span::raw(format!("{icon} ")),
        Span::styled(task.name.clone(), Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(format!("{detail}{marker}")),
    ])
}

fn draw_help(frame: &mut Frame, area: Rect, app: &App) {
    let text = match &app.screen {
        Screen::Main if app.panel == Panel::Tasks => {
            "↑/↓ move  Enter switch  n new  d archive  r refresh  h/l Tab panels  q quit"
        }
        Screen::Main if app.panel == Panel::Repos => {
            "↑/↓ move  n new  e edit  d delete  r refresh  h/l Tab panels  q quit"
        }
        Screen::NewTaskPickRepo { .. } => "↑/↓ select repo  Enter continue  Esc cancel",
        Screen::NewTaskName { .. } => "Type task name  Enter create  Esc cancel",
        Screen::RepoForm { .. } => "Tab field  Enter save  Esc cancel",
        Screen::ConfirmDeleteRepo { .. } => "y confirm delete  n/Esc cancel",
        Screen::ConfirmArchive { .. } => "y confirm archive  n/Esc cancel",
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

fn draw_new_task_pick_repo(frame: &mut Frame, area: Rect, app: &mut App) {
    let Screen::NewTaskPickRepo { choices, list_state } = &mut app.screen else {
        return;
    };

    let popup = centered_rect(70, 60, area);
    frame.render_widget(Clear, popup);

    let block = Block::default()
        .title(" Select repo for new task ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Green));

    let items: Vec<ListItem> = choices
        .iter()
        .map(|choice| ListItem::new(choice.label.clone()))
        .collect();

    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▸ ");

    frame.render_stateful_widget(list, popup, list_state);
}

fn draw_new_task_name(frame: &mut Frame, area: Rect, app: &App) {
    let Screen::NewTaskName {
        name,
        repo_label,
        ..
    } = &app.screen
    else {
        return;
    };

    let popup = centered_rect(70, 24, area);
    frame.render_widget(Clear, popup);

    let block = Block::default()
        .title(" New task ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Green));

    let body = format!("Repo: {repo_label}\n\nName: {name}_");
    let prompt = Paragraph::new(body).block(block).wrap(Wrap { trim: true });
    frame.render_widget(prompt, popup);
}

fn draw_repo_form(frame: &mut Frame, area: Rect, app: &App) {
    let Screen::RepoForm {
        name,
        path,
        url,
        focus,
        editing_id,
    } = &app.screen
    else {
        return;
    };

    let popup = centered_rect(70, 32, area);
    frame.render_widget(Clear, popup);

    let title = if editing_id.is_some() {
        " Edit repo "
    } else {
        " Add repo "
    };
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Green));

    let field = |label: &str, value: &str, active: bool| {
        let marker = if active { "▸ " } else { "  " };
        format!("{marker}{label}: {value}")
    };

    let body = format!(
        "{}\n{}\n{}",
        field("Name", name, *focus == RepoFormField::Name),
        field("Path", path, *focus == RepoFormField::Path),
        field("Url ", url, *focus == RepoFormField::Url),
    );
    let prompt = Paragraph::new(body).block(block).wrap(Wrap { trim: true });
    frame.render_widget(prompt, popup);
}

fn draw_confirm_delete_repo(frame: &mut Frame, area: Rect, app: &App) {
    let Screen::ConfirmDeleteRepo { repo_name, .. } = &app.screen else {
        return;
    };

    let popup = centered_rect(60, 20, area);
    frame.render_widget(Clear, popup);

    let block = Block::default()
        .title(" Remove repo ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));

    let body = format!(
        "Remove \"{repo_name}\" from configuration?\n\nExisting tasks are not deleted."
    );
    let prompt = Paragraph::new(body).block(block).wrap(Wrap { trim: true });
    frame.render_widget(prompt, popup);
}

fn draw_confirm_archive(frame: &mut Frame, area: Rect, app: &App) {
    let Screen::ConfirmArchive { task_name, .. } = &app.screen else {
        return;
    };

    let popup = centered_rect(60, 20, area);
    frame.render_widget(Clear, popup);

    let block = Block::default()
        .title(" Archive task ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));

    let body = format!("Archive \"{task_name}\"?\n\nThis removes the task from the active list.");
    let prompt = Paragraph::new(body).block(block).wrap(Wrap { trim: true });
    frame.render_widget(prompt, popup);
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
