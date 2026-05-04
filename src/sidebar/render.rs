use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

use crate::session::ClaudeCodeStatus;
use crate::config::{WINDOW_COLORS, AVAILABLE_MODELS, AVAILABLE_EFFORTS};
use super::{App, Mode};
use super::mode::WorktreeStep;

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
        Mode::Normal | Mode::ActionHints => 3,
        Mode::Compose { .. } => 3,
        Mode::Rename { .. } => 2,
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

    match &app.mode {
        Mode::Rename { .. } => render_rename_overlay(frame, app, inner),
        Mode::NewWindow { .. } => render_new_window_overlay(frame, app, inner),
        Mode::ActionMenu { .. } => render_action_menu_overlay(frame, app, inner),
        Mode::WorktreeFlow(_) => render_worktree_overlay(frame, app, inner),
        _ => {}
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
                        key("Enter"), hint(" focus"), sep(),
                        key("1-9"), hint(" jump"),
                    ]),
                    Line::from(vec![
                        key("i"), hint(" send"), sep(),
                        key("l"), hint(" actions"), sep(),
                        key("w"), hint(" worktree"), sep(),
                        key("n"), hint(" new"), sep(),
                        key("r"), hint(" rename"), sep(),
                        key("K"), hint(" kill"),
                    ]),
                    Line::from(vec![
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
        Mode::Rename { text } => {
            let cursor = Span::styled("█", Style::default().fg(Color::Cyan));
            frame.render_widget(
                Paragraph::new(vec![
                    Line::from(vec![
                        Span::styled("Rename: ", Style::default().fg(Color::DarkGray)),
                        Span::styled(text.as_str(), Style::default().fg(Color::White)),
                        cursor,
                    ]),
                    Line::from(vec![
                        key("Enter"), hint(" confirm  "), key("Esc"), hint(" cancel"),
                    ]),
                ]),
                area,
            );
        }
        Mode::NewWindow { .. } => {
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    key("Tab"), hint(" next field  "), key("Enter"), hint(" create  "), key("Esc"), hint(" cancel"),
                ])),
                area,
            );
        }
        Mode::ActionMenu { .. } => {
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    key("j/k"), hint(" navigate  "), key("Enter"), hint(" select  "), key("Esc"), hint(" cancel"),
                ])),
                area,
            );
        }
        Mode::WorktreeFlow(WorktreeStep::Fetching) => {
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    hint("  Fetching branches…  "),
                    key("Esc"), hint(" cancel"),
                ])),
                area,
            );
        }
        Mode::WorktreeFlow(_) => {}
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

fn overlay_rect(area: Rect, content_lines: usize) -> Rect {
    let h = (content_lines as u16 + 2).min(area.height);
    Rect {
        x: area.x,
        y: area.y + area.height.saturating_sub(h),
        width: area.width,
        height: h,
    }
}

fn render_rename_overlay(frame: &mut Frame, app: &App, area: Rect) {
    let text = match &app.mode {
        Mode::Rename { text } => text.as_str(),
        _ => return,
    };

    let cursor = Span::styled("█", Style::default().fg(Color::Cyan));
    let lines: Vec<Line> = vec![
        Line::from(vec![
            Span::styled("  Rename: ", Style::default().fg(Color::Cyan)),
            Span::styled(text, Style::default().fg(Color::White)),
            cursor,
        ]),
        Line::from(vec![
            Span::raw("  "),
            key("Enter"), hint(" confirm  "), key("Esc"), hint(" cancel"),
        ]),
    ];

    let overlay = overlay_rect(area, lines.len());
    frame.render_widget(Clear, overlay);
    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::default()
                .title(" Rename window ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan)))
            .style(Style::default().bg(Color::Rgb(18, 20, 26))),
        overlay,
    );
}

fn render_new_window_overlay(frame: &mut Frame, app: &App, area: Rect) {
    let (name, color_idx, launch_claude, field) = match &app.mode {
        Mode::NewWindow { name, color_idx, launch_claude, field } =>
            (name.as_str(), *color_idx, *launch_claude, *field),
        _ => return,
    };

    let color_name = WINDOW_COLORS.get(color_idx).map(|c| c.0).unwrap_or("none");
    let check = if launch_claude { "[x]" } else { "[ ]" };

    let field_style = |f: u8| -> (Style, Style) {
        if field == f {
            (
                Style::default().fg(Color::Cyan),
                Style::default().fg(Color::White),
            )
        } else {
            (
                Style::default().fg(Color::DarkGray),
                Style::default().fg(Color::Rgb(100, 100, 110)),
            )
        }
    };

    let cursor = Span::styled("█", Style::default().fg(Color::Cyan));

    let (label0, val0) = field_style(0);
    let (label1, val1) = field_style(1);
    let (label2, val2) = field_style(2);

    let name_span: Vec<Span> = if field == 0 {
        vec![
            Span::styled("  Name:    ", label0),
            Span::styled(name, val0),
            cursor,
        ]
    } else {
        vec![
            Span::styled("  Name:    ", label0),
            Span::styled(name, val0),
        ]
    };

    let lines: Vec<Line> = vec![
        Line::from(name_span),
        Line::from(vec![
            Span::styled("  Color:   ", label1),
            Span::styled("◀ ", val1),
            Span::styled(color_name, val1),
            Span::styled(" ▶", val1),
        ]),
        Line::from(vec![
            Span::styled("  Claude:  ", label2),
            Span::styled(format!("{} Launch claude", check), val2),
        ]),
        Line::from(vec![
            Span::raw("  "),
            key("Tab"), hint(" next field  "), key("Enter"), hint(" create  "), key("Esc"), hint(" cancel"),
        ]),
    ];

    let overlay = overlay_rect(area, lines.len());
    frame.render_widget(Clear, overlay);
    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::default()
                .title(" New window ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan)))
            .style(Style::default().bg(Color::Rgb(18, 20, 26))),
        overlay,
    );
}

fn render_action_menu_overlay(frame: &mut Frame, app: &App, area: Rect) {
    let (items, cursor) = match &app.mode {
        Mode::ActionMenu { items, cursor } => (items, *cursor),
        _ => return,
    };

    let mut lines: Vec<Line> = Vec::new();
    let fill_width = area.width as usize;

    for (i, item) in items.iter().enumerate() {
        let label = item.label();
        if i == cursor {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  {}  {}", "▶", label),
                    Style::default().fg(Color::Cyan).bg(SEL_BG),
                ),
                Span::styled(" ".repeat(fill_width), Style::default().bg(SEL_BG)),
            ]));
        } else {
            lines.push(Line::from(Span::styled(
                format!("    {}", label),
                Style::default().fg(Color::DarkGray),
            )));
        }
    }
    lines.push(Line::from(vec![
        Span::raw("  "),
        key("j/k"), hint(" navigate  "), key("Enter"), hint(" select  "), key("Esc"), hint(" cancel"),
    ]));

    let overlay = overlay_rect(area, lines.len());
    frame.render_widget(Clear, overlay);
    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::default()
                .title(" Actions ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan)))
            .style(Style::default().bg(Color::Rgb(18, 20, 26))),
        overlay,
    );
}

fn render_worktree_overlay(frame: &mut Frame, app: &App, area: Rect) {
    match &app.mode {
        Mode::WorktreeFlow(step) => match step {
            WorktreeStep::Fetching => {
                // Handled in render_footer; no overlay needed.
            }
            WorktreeStep::BranchSelect {
                branches, filter, cursor, entering_new, new_branch_text, ..
            } => {
                render_branch_select_overlay(frame, area, branches, filter, *cursor, *entering_new, new_branch_text);
            }
            WorktreeStep::FolderName { folder, .. } => {
                render_folder_name_overlay(frame, area, folder);
            }
            WorktreeStep::Options { opts, .. } => {
                render_options_overlay(frame, area, opts);
            }
            WorktreeStep::Executing { status } => {
                // Show a simple status message in the footer area (bottom 1 row).
                let h = 1u16;
                let footer_area = Rect {
                    x: area.x,
                    y: area.y + area.height.saturating_sub(h),
                    width: area.width,
                    height: h,
                };
                frame.render_widget(
                    Paragraph::new(Line::from(vec![
                        Span::styled(format!("  {}", status), Style::default().fg(Color::DarkGray)),
                    ])),
                    footer_area,
                );
            }
        },
        _ => {}
    }
}

fn render_branch_select_overlay(
    frame: &mut Frame,
    area: Rect,
    branches: &[crate::git::BranchEntry],
    filter: &str,
    cursor: usize,
    entering_new: bool,
    new_branch_text: &str,
) {
    // Use the full inner area for this overlay.
    let overlay = area;
    let cursor_char = Span::styled("█", Style::default().fg(Color::Cyan));

    let mut lines: Vec<Line> = Vec::new();

    if entering_new {
        lines.push(Line::from(vec![
            Span::styled("  New branch: ", Style::default().fg(Color::Cyan)),
            Span::styled(new_branch_text, Style::default().fg(Color::White)),
            cursor_char,
        ]));
        lines.push(Line::from(vec![
            Span::raw("  "),
            key("Enter"), hint(" create  "), key("Tab"), hint("/"), key("F"), hint(" existing  "), key("Esc"), hint(" cancel"),
        ]));
    } else {
        // Filter line
        lines.push(Line::from(vec![
            Span::styled("  Filter: ", Style::default().fg(Color::Cyan)),
            Span::styled(filter, Style::default().fg(Color::White)),
            cursor_char,
        ]));

        // Filtered branch list
        let filtered: Vec<&crate::git::BranchEntry> = branches.iter()
            .filter(|b| filter.is_empty() || b.name.contains(filter))
            .collect();

        // How many rows we can show (overlay height - 2 borders - filter line - hint line)
        let max_rows = (overlay.height as usize).saturating_sub(4);

        // Scroll to keep cursor visible
        let start = if cursor >= max_rows { cursor + 1 - max_rows } else { 0 };
        let visible = filtered.iter().skip(start).take(max_rows);

        for (i, branch) in visible.enumerate() {
            let abs_idx = start + i;
            let is_sel = abs_idx == cursor;
            let suffix = if branch.worktree_path.is_some() {
                " (worktree)"
            } else {
                ""
            };
            let label = format!("  {}{}", branch.name, suffix);

            if is_sel {
                let fill_width = overlay.width as usize;
                lines.push(Line::from(vec![
                    Span::styled(label, Style::default().fg(Color::Cyan).bg(SEL_BG)),
                    Span::styled(" ".repeat(fill_width), Style::default().bg(SEL_BG)),
                ]));
            } else if branch.worktree_path.is_some() {
                // Show worktree suffix dimmed
                let base = format!("  {}", branch.name);
                lines.push(Line::from(vec![
                    Span::styled(base, Style::default().fg(Color::DarkGray)),
                    Span::styled(" (worktree)", Style::default().fg(Color::Rgb(60, 60, 70))),
                ]));
            } else {
                lines.push(Line::from(Span::styled(label, Style::default().fg(Color::DarkGray))));
            }
        }

        lines.push(Line::from(vec![
            Span::raw("  "),
            key("j/k"), hint(" nav  "), key("Enter"), hint(" select  "), key("F"), hint(" new branch  "), key("Esc"), hint(" cancel"),
        ]));
    }

    frame.render_widget(Clear, overlay);
    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::default()
                .title(" New worktree — select branch ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan)))
            .style(Style::default().bg(Color::Rgb(18, 20, 26))),
        overlay,
    );
}

fn render_folder_name_overlay(frame: &mut Frame, area: Rect, folder: &str) {
    let cursor = Span::styled("█", Style::default().fg(Color::Cyan));
    let lines: Vec<Line> = vec![
        Line::from(vec![
            Span::styled("  Folder: ", Style::default().fg(Color::Cyan)),
            Span::styled(folder, Style::default().fg(Color::White)),
            cursor,
        ]),
        Line::from(Span::styled(
            "  (created alongside the main repo)",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(vec![
            Span::raw("  "),
            key("Enter"), hint(" confirm  "), key("Esc"), hint(" cancel"),
        ]),
    ];

    let overlay = overlay_rect(area, lines.len());
    frame.render_widget(Clear, overlay);
    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::default()
                .title(" New worktree — folder name ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan)))
            .style(Style::default().bg(Color::Rgb(18, 20, 26))),
        overlay,
    );
}

fn render_options_overlay(frame: &mut Frame, area: Rect, opts: &crate::sidebar::mode::WorktreeOpts) {
    let field_style = |f: u8| -> (Style, Style) {
        if opts.field == f {
            (
                Style::default().fg(Color::Cyan),
                Style::default().fg(Color::White),
            )
        } else {
            (
                Style::default().fg(Color::DarkGray),
                Style::default().fg(Color::Rgb(100, 100, 110)),
            )
        }
    };

    let model_name = AVAILABLE_MODELS.get(opts.model_idx).copied().unwrap_or("?");
    let effort_name = AVAILABLE_EFFORTS.get(opts.effort_idx).copied().unwrap_or("?");
    let color_name = WINDOW_COLORS.get(opts.color_idx).map(|c| c.0).unwrap_or("none");
    let claude_check = if opts.launch_claude { "[x]" } else { "[ ]" };
    let vscode_check = if opts.open_vscode { "[x]" } else { "[ ]" };

    let (lm, vm) = field_style(0);
    let (le, ve) = field_style(1);
    let (lc, vc) = field_style(2);
    let (lcol, vcol) = field_style(3);
    let (lv, vv) = field_style(4);

    let lines: Vec<Line> = vec![
        Line::from(vec![
            Span::styled("  Model:   ", lm),
            Span::styled("◀ ", vm),
            Span::styled(model_name, vm),
            Span::styled(" ▶", vm),
        ]),
        Line::from(vec![
            Span::styled("  Effort:  ", le),
            Span::styled("◀ ", ve),
            Span::styled(effort_name, ve),
            Span::styled(" ▶", ve),
        ]),
        Line::from(vec![
            Span::styled("  Claude:  ", lc),
            Span::styled(format!("{} Launch claude", claude_check), vc),
        ]),
        Line::from(vec![
            Span::styled("  Color:   ", lcol),
            Span::styled("◀ ", vcol),
            Span::styled(color_name, vcol),
            Span::styled(" ▶", vcol),
        ]),
        Line::from(vec![
            Span::styled("  VSCode:  ", lv),
            Span::styled(format!("{} Open VSCode", vscode_check), vv),
        ]),
        Line::from(vec![
            Span::raw("  "),
            key("Tab"), hint(" next  "), key("◀▶"), hint(" cycle  "), key("Space"), hint(" toggle  "), key("Enter"), hint(" create"),
        ]),
    ];

    let overlay = overlay_rect(area, lines.len());
    frame.render_widget(Clear, overlay);
    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::default()
                .title(" New worktree — options ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan)))
            .style(Style::default().bg(Color::Rgb(18, 20, 26))),
        overlay,
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
