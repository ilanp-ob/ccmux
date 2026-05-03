use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

use crate::session::ClaudeCodeStatus;
use super::{App, Mode};

pub fn render(frame: &mut Frame, app: &mut App) {
    let area = frame.area();

    // Main block
    let block = Block::default()
        .title(" ccmux ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Split inner: list area + footer
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(inner);

    render_list(frame, app, chunks[0]);
    render_footer(frame, app, chunks[1]);

    // Overlay error/message
    if app.error.is_some() || app.message.is_some() {
        render_message_bar(frame, app, area);
    }
}

fn render_list(frame: &mut Frame, app: &App, area: Rect) {
    let flat = App::flat_panes_ref(&app.groups);
    if flat.is_empty() {
        let p = Paragraph::new("  No Claude sessions detected")
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(p, area);
        return;
    }

    let mut lines: Vec<Line> = Vec::new();
    let mut flat_idx = 0usize;

    for group in &app.groups {
        // Window group header
        let server_label = group.server.as_deref()
            .map(|s| format!("  [{}]", s))
            .unwrap_or_default();
        lines.push(Line::from(vec![
            Span::styled(
                format!("  ▸ {}{}",  group.window_name, server_label),
                Style::default().fg(Color::Blue).add_modifier(Modifier::BOLD),
            ),
        ]));

        for pane in &group.panes {
            let is_selected = flat_idx == app.selected;
            flat_idx += 1;

            let status_color = status_color(&pane.status);
            let icon = pane.status.icon();
            let branch = pane.git_branch().unwrap_or_else(|| "?".to_string());
            let name = &pane.window_name;

            if is_selected {
                // Expanded row
                let path_str = pane.current_path.to_string_lossy();
                let status_label = match &pane.status {
                    ClaudeCodeStatus::Working => "Working…",
                    ClaudeCodeStatus::WaitingInput => "⚠ Waiting for input",
                    ClaudeCodeStatus::Idle => "Idle",
                    ClaudeCodeStatus::Unknown => "Unknown",
                };

                lines.push(Line::from(vec![
                    Span::styled("  ┌ ", Style::default().fg(status_color)),
                    Span::styled(
                        format!("{} {} ", icon, name),
                        Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(branch.clone(), Style::default().fg(Color::Cyan)),
                    Span::styled(
                        format!("  %{}", pane.display_num),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]));
                lines.push(Line::from(vec![
                    Span::styled("  │ ", Style::default().fg(status_color)),
                    Span::styled(path_str.to_string(), Style::default().fg(Color::DarkGray)),
                ]));
                lines.push(Line::from(vec![
                    Span::styled("  │ ", Style::default().fg(status_color)),
                    Span::styled(status_label, Style::default().fg(status_color)),
                ]));
                lines.push(Line::from(vec![
                    Span::styled("  │ ", Style::default().fg(status_color)),
                    Span::styled(
                        "[Enter] focus  [K] kill  [r] rename  [w] worktree",
                        Style::default().fg(Color::DarkGray),
                    ),
                ]));
                lines.push(Line::from(vec![
                    Span::styled("  └────", Style::default().fg(status_color)),
                ]));
            } else {
                // Compact row
                let alert = pane.status == ClaudeCodeStatus::WaitingInput;
                let row_style = if alert {
                    Style::default().fg(Color::Yellow)
                } else {
                    Style::default()
                };
                lines.push(Line::from(vec![
                    Span::raw("    "),
                    Span::styled(format!("{} ", icon), Style::default().fg(status_color)),
                    Span::styled(format!("{:<12}", name), row_style),
                    Span::styled(format!("  {:<10}", branch), Style::default().fg(Color::Cyan)),
                    Span::styled(
                        format!("  %{}", pane.display_num),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]));
            }
        }

        lines.push(Line::raw(""));
    }

    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
}

fn render_footer(frame: &mut Frame, app: &App, area: Rect) {
    let text = match app.mode {
        Mode::Normal | Mode::ActionHints => "j/k nav  Enter focus  K kill  r rename  w worktree  ? help  q quit",
        Mode::Help => "Press ? or Esc to close help",
        Mode::Confirm(_) => "Confirm? [y]es / [n]o",
    };
    let p = Paragraph::new(text)
        .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(p, area);
}

fn render_message_bar(frame: &mut Frame, app: &App, area: Rect) {
    let msg = app.error.as_deref().or(app.message.as_deref()).unwrap_or("");
    let color = if app.error.is_some() { Color::Red } else { Color::Green };
    let bar_area = Rect { y: area.bottom().saturating_sub(2), height: 1, ..area };
    frame.render_widget(Clear, bar_area);
    frame.render_widget(
        Paragraph::new(format!(" {}", msg)).style(Style::default().fg(color)),
        bar_area,
    );
}

fn status_color(status: &ClaudeCodeStatus) -> Color {
    match status {
        ClaudeCodeStatus::Working => Color::Green,
        ClaudeCodeStatus::WaitingInput => Color::Yellow,
        ClaudeCodeStatus::Idle => Color::Cyan,
        ClaudeCodeStatus::Unknown => Color::DarkGray,
    }
}
