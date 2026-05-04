use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

use crate::session::ClaudeCodeStatus;
use super::{App, Mode};

// Alternating-row background for unfocused sidebar
const ALT_BG: Color = Color::Rgb(28, 30, 36);
const SEL_BG: Color = Color::Rgb(42, 46, 54);
// The sidebar's own background when focused vs unfocused
const FOCUSED_BG: Color = Color::Rgb(18, 20, 26);
const UNFOCUSED_BG: Color = Color::Rgb(26, 28, 34);

pub fn render(frame: &mut Frame, app: &mut App) {
    let area = frame.area();

    let focused = app.is_focused;
    let sidebar_bg = if focused { FOCUSED_BG } else { UNFOCUSED_BG };
    let border_color = if focused { Color::Cyan } else { Color::Rgb(60, 60, 70) };

    let title = if app.sticky { " ccmux [S] " } else { " ccmux " };
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .style(Style::default().bg(sidebar_bg));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let footer_h = match app.mode {
        Mode::Normal | Mode::ActionHints => 2,
        Mode::Compose { .. } => 3,
        _ => 1,
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(footer_h)])
        .split(inner);

    render_list(frame, app, chunks[0], sidebar_bg);
    render_footer(frame, app, chunks[1]);

    if matches!(app.mode, Mode::Help) {
        render_help_overlay(frame, inner);
    }

    if app.error.is_some() || app.message.is_some() {
        render_message_bar(frame, app, area);
    }
}

fn pane_display_name(pane: &crate::session::DetectedPane) -> String {
    pane.current_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| pane.current_path.to_string_lossy().into_owned())
}

fn shorten_path(path: &std::path::Path) -> String {
    if let Ok(home) = std::env::var("HOME") {
        if let Ok(rel) = path.strip_prefix(std::path::Path::new(&home)) {
            return format!("~/{}", rel.display());
        }
    }
    path.display().to_string()
}

fn truncate(s: &str, max: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        s.to_string()
    } else {
        chars[..max.saturating_sub(1)].iter().collect::<String>() + "…"
    }
}

/// Extract meaningful content lines from above the Claude Code input field.
/// Returns lines in reading order (oldest first), up to `max_lines`.
fn extract_preview_lines(content: &str, max_line_len: usize, max_lines: usize) -> Vec<String> {
    let lines: Vec<&str> = content.lines().collect();

    // Find the ─ separator that sits directly above the ❯ prompt.
    // That boundary is where Claude's input box starts; we want content ABOVE it.
    let boundary = lines.iter().enumerate().rev().find_map(|(i, line)| {
        if line.contains('❯') && i > 0 && lines[i - 1].contains('─') {
            Some(i - 1) // index of the ─ line
        } else {
            None
        }
    });

    // If no input field found, skip lines that look like Claude's status bar
    let end = boundary.unwrap_or_else(|| {
        lines.iter().rposition(|l| {
            let t = l.trim();
            t.starts_with('❯')
                || t.contains("accept edits")
                || t.contains("Esc to cancel")
                || t.contains("Tab to amend")
                || t.contains("ctrl+e to")
        })
        .map(|i| i.saturating_sub(1))
        .unwrap_or(lines.len())
    });

    let meaningful: Vec<String> = lines[..end]
        .iter()
        .rev()
        .map(|l| l.trim())
        .filter(|l| {
            l.len() > 2
                && !l.chars().all(|c| {
                    matches!(c, '─' | '═' | '━' | '╮' | '╭' | '╯' | '╰' | '│' | ' ' | '▸' | '·' | '*')
                })
        })
        .take(max_lines)
        .map(|l| truncate(l, max_line_len))
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();

    meaningful
}

fn render_list(frame: &mut Frame, app: &mut App, area: Rect, sidebar_bg: Color) {
    let total_items = App::flat_panes_ref(&app.groups).len();
    if total_items == 0 {
        frame.render_widget(
            Paragraph::new("  No Claude sessions detected")
                .style(Style::default().fg(Color::DarkGray).bg(sidebar_bg)),
            area,
        );
        return;
    }

    let area_h = area.height as usize;
    let area_w = area.width as usize;
    let sel = app.selected;

    // Auto-scroll: keep selected item in view (rough 4-line estimate per item).
    let per_screen = (area_h / 4).max(1);
    if sel < app.scroll_offset {
        app.scroll_offset = sel;
    } else if sel >= app.scroll_offset + per_screen {
        app.scroll_offset = sel + 1 - per_screen;
    }
    let scroll = app.scroll_offset;

    let own_window = app.own_window_id.clone().unwrap_or_default();
    let inner_w = area_w.saturating_sub(2); // after 2-char prefix

    let mut lines: Vec<Line> = Vec::new();
    let mut click_rows: Vec<(u16, usize)> = Vec::new();
    let mut flat_idx = 0usize;
    let mut rows_used = 0usize;

    if scroll > 0 {
        lines.push(Line::from(Span::styled(
            format!("  ↑ {} above", scroll),
            Style::default().fg(Color::DarkGray).bg(sidebar_bg),
        )));
        rows_used += 1;
    }

    'outer: for group in &app.groups {
        if group.panes.len() > 1 || group.server.is_some() {
            let win_idx = group.panes.first().map(|p| p.window_index.as_str()).unwrap_or("?");
            let server_label = group.server.as_deref()
                .map(|s| format!(" [{}]", s))
                .unwrap_or_default();
            if flat_idx >= scroll {
                lines.push(Line::from(Span::styled(
                    format!("  win {}{}", win_idx, server_label),
                    Style::default().fg(Color::DarkGray).bg(sidebar_bg),
                )));
                rows_used += 1;
            }
        }

        for pane in &group.panes {
            let pane_idx = flat_idx;
            flat_idx += 1;

            if pane_idx < scroll { continue; }

            let is_sel = pane_idx == sel;

            // Gather preview lines now (before mutable borrow of app)
            let preview_lines: Vec<String> = if is_sel {
                app.pane_content_cache.get(&pane.pane_id)
                    .map(|c| extract_preview_lines(c, inner_w.saturating_sub(1), 6))
                    .unwrap_or_default()
            } else {
                Vec::new()
            };

            let item_h = 3 + if is_sel { preview_lines.len().max(1) } else { 0 };
            // +1 for the separator line between items
            let total_item_h = item_h + 1;

            if rows_used + total_item_h > area_h { break 'outer; }

            let is_cur = pane.window_id == own_window;
            let sc = status_color(&pane.status);
            let icon = pane.status.icon();

            let branch = pane.git_branch().unwrap_or_else(|| "?".to_string());
            let name = pane_display_name(pane);
            let num_str = format!("%{}", pane.display_num);
            let name_max = inner_w.saturating_sub(4 + num_str.len());
            let name_short = truncate(&name, name_max);
            let branch_short = truncate(&branch, inner_w.saturating_sub(1));
            let path_max = if is_sel { inner_w.saturating_sub(12) } else { inner_w.saturating_sub(1) };
            let path_short = truncate(&shorten_path(&pane.current_path), path_max);

            click_rows.push((area.y + lines.len() as u16, pane_idx));

            let row_bg: Color = if is_sel { SEL_BG }
                else if pane_idx % 2 == 0 { ALT_BG }
                else { sidebar_bg };

            let sp = |fg: Color| Style::default().fg(fg).bg(row_bg);
            let base = Style::default().bg(row_bg);

            // ── 2-char prefix ─────────────────────────────────────────────────
            let win_span = if is_cur {
                Span::styled("▶", sp(Color::Cyan))
            } else {
                Span::styled(" ", base)
            };
            let sel_span = if is_sel {
                Span::styled("▌", sp(sc))
            } else if pane.status == ClaudeCodeStatus::WaitingInput {
                Span::styled("▌", sp(Color::Yellow))
            } else {
                Span::styled(" ", base)
            };

            let name_fg = if !is_sel && pane.status == ClaudeCodeStatus::WaitingInput {
                Color::Yellow
            } else {
                Color::White
            };
            let name_mod = if is_sel || is_cur { Modifier::BOLD } else { Modifier::empty() };

            // Right-align %N — use chars().count() because icons like ● are 3 bytes but 1 column.
            let left_len = 2 + 1 + icon.chars().count() + 1 + name_short.chars().count();
            let pad = area_w.saturating_sub(left_len + num_str.len());

            // ── Line 1: [W][S] icon name ··· %N ──────────────────────────────
            // Trailing fill ensures row_bg covers the full width (ratatui clips at area.width).
            lines.push(Line::from(vec![
                win_span, sel_span,
                Span::styled(format!(" {} ", icon), sp(sc)),
                Span::styled(
                    name_short.clone(),
                    Style::default().fg(name_fg).add_modifier(name_mod).bg(row_bg),
                ),
                Span::styled(" ".repeat(pad), base),
                Span::styled(num_str, sp(Color::Rgb(70, 70, 70))),
                Span::styled(" ".repeat(area_w), base),
            ]).style(base));

            // Helper: pipe span on continuation lines (same position as sel_span on line 1).
            // For selected items the pipe extends the full block height.
            // For waiting-input items it also extends to signal attention.
            let cont_pipe: Option<Span> = if is_sel {
                Some(Span::styled("▌", sp(sc)))
            } else if pane.status == ClaudeCodeStatus::WaitingInput {
                Some(Span::styled("▌", sp(Color::Yellow)))
            } else {
                None
            };

            // Trailing fill — pads the line to the full area width so the row_bg
            // covers every cell. Ratatui clips at area.width so over-filling is safe.
            let fill = || Span::styled(" ".repeat(area_w), base);

            // ── Line 2: branch ────────────────────────────────────────────────
            if let Some(ref pipe) = cont_pipe {
                lines.push(Line::from(vec![
                    Span::styled(" ", base),
                    pipe.clone(),
                    Span::styled(
                        format!(" {}", branch_short),
                        sp(if is_sel { Color::Cyan } else { Color::Rgb(80, 90, 110) }),
                    ),
                    fill(),
                ]).style(base));
            } else {
                lines.push(Line::from(vec![
                    Span::styled(format!("   {}", branch_short), sp(Color::Rgb(80, 90, 110))),
                    fill(),
                ]).style(base));
            }

            // ── Line 3: path + status (selected) or path alone ───────────────
            if is_sel {
                let status_label = match &pane.status {
                    ClaudeCodeStatus::Working => "● Working",
                    ClaudeCodeStatus::WaitingInput => "⚠ Waiting",
                    ClaudeCodeStatus::Idle => "○ Idle",
                    ClaudeCodeStatus::Unknown => "○ Unknown",
                };
                lines.push(Line::from(vec![
                    Span::styled(" ", base),
                    Span::styled("▌", sp(sc)),
                    Span::styled(format!(" {}  ", path_short), sp(Color::DarkGray)),
                    Span::styled(status_label, sp(sc)),
                    fill(),
                ]).style(base));
            } else if pane.status == ClaudeCodeStatus::WaitingInput {
                lines.push(Line::from(vec![
                    Span::styled(" ", base),
                    Span::styled("▌", sp(Color::Yellow)),
                    Span::styled(format!(" {}", path_short), sp(Color::Rgb(55, 58, 68))),
                    fill(),
                ]).style(base));
            } else {
                lines.push(Line::from(vec![
                    Span::styled(format!("   {}", path_short), sp(Color::Rgb(55, 58, 68))),
                    fill(),
                ]).style(base));
            }

            // ── Lines 4+: content preview (selected only) ────────────────────
            if is_sel {
                if preview_lines.is_empty() {
                    lines.push(Line::from(vec![
                        Span::styled(" ", base),
                        Span::styled("▌", sp(sc)),
                        Span::styled(" —", sp(Color::Rgb(70, 72, 85))),
                        fill(),
                    ]).style(base));
                } else {
                    for pl in &preview_lines {
                        lines.push(Line::from(vec![
                            Span::styled(" ", base),
                            Span::styled("▌", sp(sc)),
                            Span::styled(format!(" {}", pl), sp(Color::Rgb(140, 145, 165))),
                            fill(),
                        ]).style(base));
                    }
                }
            }

            // ── Separator between items ───────────────────────────────────────
            lines.push(Line::from(Span::styled(
                " ".repeat(area_w),
                Style::default().bg(sidebar_bg),
            )));

            rows_used += total_item_h;
        }
    }

    if flat_idx < total_items {
        let remaining = total_items - flat_idx;
        lines.push(Line::from(Span::styled(
            format!("  ↓ {} below", remaining),
            Style::default().fg(Color::DarkGray).bg(sidebar_bg),
        )));
    }

    app.pane_click_rows = click_rows;
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(sidebar_bg)),
        area,
    );
}

fn key(k: &'static str) -> Span<'static> {
    Span::styled(k, Style::default().fg(Color::White).add_modifier(Modifier::BOLD))
}

fn hint(h: &'static str) -> Span<'static> {
    Span::styled(h, Style::default().fg(Color::DarkGray))
}

fn sep() -> Span<'static> {
    Span::raw("  ")
}

fn render_footer(frame: &mut Frame, app: &App, area: Rect) {
    match &app.mode {
        Mode::Normal | Mode::ActionHints => {
            frame.render_widget(
                Paragraph::new(vec![
                    Line::from(vec![
                        key("j/k"), hint(" nav"), sep(),
                        key("1-9"), hint(" jump (×2 focus)"), sep(),
                        key("K"), hint(" kill"),
                    ]),
                    Line::from(vec![
                        key("i"), hint(" send"), sep(),
                        key("s"), hint(" sticky"), sep(),
                        key("?"), hint(" help"), sep(),
                        key("q"), hint(" quit"),
                    ]),
                ]),
                area,
            );
        }
        Mode::Help => {
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    hint("Press "), key("?"), hint(" or "), key("Esc"), hint(" to close"),
                ])),
                area,
            );
        }
        Mode::Confirm(_) => {
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    hint("Confirm? "), key("y"), hint(" yes  "), key("n"), hint(" no"),
                ])),
                area,
            );
        }
        Mode::Compose { text } => {
            let cursor = Span::styled("█", Style::default().fg(Color::Cyan));
            frame.render_widget(
                Paragraph::new(vec![
                    Line::from(vec![
                        Span::styled("› ", Style::default().fg(Color::Cyan)),
                        Span::styled(text.as_str(), Style::default().fg(Color::White)),
                        cursor,
                    ]),
                    Line::from(vec![
                        key("Enter"), hint(" send  "), key("Esc"), hint(" cancel"),
                    ]),
                    Line::from(Span::raw("")),
                ]),
                area,
            );
        }
        _ => {}
    }
}

fn render_help_overlay(frame: &mut Frame, area: Rect) {
    fn row<'a>(k: &'a str, desc: &'a str) -> Line<'a> {
        Line::from(vec![
            Span::styled(format!("  {:>6}  ", k), Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::styled(desc, Style::default().fg(Color::White)),
        ])
    }
    fn section(title: &str) -> Line {
        Line::from(Span::styled(
            format!("  {}", title),
            Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD),
        ))
    }

    let lines: Vec<Line> = vec![
        section("Navigation"),
        row("j / ↓",  "Select next session"),
        row("k / ↑",  "Select previous session"),
        row("1–9",    "Jump to session by number (×2 to focus)"),
        row("Enter",  "Preview window (×2 to focus pane)"),
        Line::raw(""),
        section("Actions"),
        row("i",      "Send message to Claude session"),
        row("l",      "Action menu (PR ops, delete worktree)"),
        row("n",      "New tmux window"),
        row("w",      "New worktree (fetch → branch → options)"),
        row("r",      "Rename current window"),
        row("K",      "Kill current window (confirm)"),
        Line::raw(""),
        section("Sidebar"),
        row("s",      "Toggle sticky — auto-open sidebar when"),
        row("",       "  switching to Claude windows"),
        row("q / Esc","Close sidebar"),
        row("?",      "This help screen"),
        Line::raw(""),
        Line::from(Span::styled(
            "  Press ? or Esc to close",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    let h = (lines.len() as u16 + 2).min(area.height);
    let overlay = Rect {
        x: area.x,
        y: area.y + area.height.saturating_sub(h),
        width: area.width,
        height: h,
    };

    frame.render_widget(Clear, overlay);
    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::default()
                .title(" Help ")
                .borders(Borders::TOP)
                .border_style(Style::default().fg(Color::Cyan)))
            .style(Style::default().bg(Color::Rgb(18, 20, 26))),
        overlay,
    );
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
