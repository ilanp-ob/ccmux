use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

use crate::session::ClaudeCodeStatus;
use crate::config::{WINDOW_COLORS, AVAILABLE_MODELS, AVAILABLE_EFFORTS};
use crate::history::relative_time;
use super::{App, Mode};
use super::mode::WorktreeStep;

// Window group header background (slightly lighter than sidebar bg)
const HDR_BG: Color = Color::Rgb(22, 24, 31);
// Session row background (consistent within a group)
const ROW_BG: Color = Color::Rgb(28, 30, 38);
const SEL_BG: Color = Color::Rgb(42, 46, 54);
// The sidebar's own background when focused vs unfocused
const FOCUSED_BG: Color = Color::Rgb(18, 20, 26);
const UNFOCUSED_BG: Color = Color::Rgb(26, 28, 34);

// Cycling left-accent-bar colors — one per window group
const GROUP_ACCENTS: &[Color] = &[
    Color::Rgb(60,  80, 160),  // blue
    Color::Rgb(50, 140,  70),  // green
    Color::Rgb(160,  60,  60), // red
    Color::Rgb(120,  50, 160), // purple
    Color::Rgb(160, 120,  30), // gold
    Color::Rgb(30,  140, 160), // teal
];

/// Build spans for a text field showing a block cursor at `cursor` (char index).
/// When `active` is false, returns a plain unstyled span.
fn text_with_cursor(text: &str, cursor: usize, style: Style, active: bool) -> Vec<Span<'static>> {
    if !active {
        return vec![Span::styled(text.to_string(), style)];
    }
    let chars: Vec<char> = text.chars().collect();
    let block = Span::styled("█", Style::default().fg(Color::Cyan));
    if cursor >= chars.len() {
        vec![Span::styled(text.to_string(), style), block]
    } else {
        let before: String = chars[..cursor].iter().collect();
        let after: String = chars[cursor + 1..].iter().collect();
        vec![Span::styled(before, style), block, Span::styled(after, style)]
    }
}

/// Convert an xterm-256 colour index to a ratatui RGB colour.
fn xterm256_to_rgb(n: u8) -> Color {
    match n {
        0..=15 => {
            const BASIC: [(u8, u8, u8); 16] = [
                (0,0,0),(128,0,0),(0,128,0),(128,128,0),
                (0,0,128),(128,0,128),(0,128,128),(192,192,192),
                (128,128,128),(255,0,0),(0,255,0),(255,255,0),
                (0,0,255),(255,0,255),(0,255,255),(255,255,255),
            ];
            let (r, g, b) = BASIC[n as usize];
            Color::Rgb(r, g, b)
        }
        16..=231 => {
            let idx = n - 16;
            let bi = idx % 6;
            let gi = (idx / 6) % 6;
            let ri = idx / 36;
            let v = |x: u8| if x == 0 { 0u8 } else { 55u8.saturating_add(x.saturating_mul(40)) };
            Color::Rgb(v(ri), v(gi), v(bi))
        }
        232..=255 => {
            let l = 8u8.saturating_add((n - 232).saturating_mul(10));
            Color::Rgb(l, l, l)
        }
    }
}

/// Parse a tmux colour string ("colour75", "#61AFEF") into a ratatui Color.
fn tmux_colour_to_ratatui(colour: &str) -> Option<Color> {
    let s = colour.trim();
    if let Some(rest) = s.strip_prefix("colour") {
        rest.parse::<u8>().ok().map(xterm256_to_rgb)
    } else if let Some(hex) = s.strip_prefix('#') {
        if hex.len() == 6 {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            Some(Color::Rgb(r, g, b))
        } else {
            None
        }
    } else {
        None
    }
}

/// Render a ── Title ── separator line with INFO_BG background.
fn titled_sep(title: &str, w: usize) -> Line<'static> {
    const SEP_CLR: Color = Color::Rgb(45, 48, 58);
    const TITLE_CLR: Color = Color::Rgb(85, 90, 110);
    let label = format!(" {} ", title);
    let label_w = label.chars().count();
    let dashes = w.saturating_sub(label_w);
    let left = dashes / 3;
    let right = dashes - left;
    Line::from(vec![
        Span::styled("─".repeat(left), Style::default().fg(SEP_CLR).bg(INFO_BG)),
        Span::styled(label, Style::default().fg(TITLE_CLR).bg(INFO_BG)),
        Span::styled("─".repeat(right), Style::default().fg(SEP_CLR).bg(INFO_BG)),
    ])
}

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
        Mode::EditWindow { .. } => 1,
        _ => 1,
    };
    let metrics_h: u16 = match app.mode {
        Mode::Normal | Mode::ActionHints => 2,
        _ => 0,
    };
    // top-title-sep + usage + mid-title-sep + mempalace + bottom-sep
    let info_h: u16 = if app.global_info.has_data() { 5 } else { 0 };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(info_h),
            Constraint::Length(footer_h),
            Constraint::Length(metrics_h),
        ])
        .split(inner);

    render_list(frame, app, chunks[0], sidebar_bg);
    if info_h > 0 {
        render_global_info(frame, app, chunks[1]);
    }
    render_footer(frame, app, chunks[2]);
    if metrics_h > 0 {
        render_own_metrics(frame, app, chunks[3]);
    }

    if matches!(app.mode, Mode::Help) {
        render_help_overlay(frame, inner);
    }

    match &app.mode {
        Mode::EditWindow { .. } => render_edit_window_overlay(frame, app, inner),
        Mode::NewWindow { .. } => render_new_window_overlay(frame, app, inner),
        Mode::ActionMenu { .. } => render_action_menu_overlay(frame, app, inner),
        Mode::WorktreeFlow(_) => render_worktree_overlay(frame, app, inner),
        Mode::FolderPick(_) => render_folder_pick_overlay(frame, app, inner),
        Mode::History(_) => render_history(frame, app, inner),
        _ => {}
    }

    if app.error.is_some() || app.message.is_some() {
        render_message_bar(frame, app, area);
    }
}

fn pane_display_name(pane: &crate::session::DetectedPane) -> String {
    if !pane.window_name.is_empty() {
        return pane.window_name.clone();
    }
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

fn is_shell_cmd(cmd: &str) -> bool {
    let name = cmd.rsplit('/').next().unwrap_or(cmd);
    matches!(name, "zsh" | "bash" | "fish" | "sh" | "dash" | "csh" | "tcsh" | "nu")
}

fn render_list(frame: &mut Frame, app: &mut App, area: Rect, sidebar_bg: Color) {
    let total_items = App::flat_panes_ref(&app.groups).len();
    if total_items == 0 && app.jobs.is_empty() {
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
    let inner_w = area_w.saturating_sub(2);

    // Auto-scroll: keep selected item in view.
    let per_screen = (area_h / 5).max(1);
    if sel < app.scroll_offset {
        app.scroll_offset = sel;
    } else if sel >= app.scroll_offset + per_screen {
        app.scroll_offset = sel + 1 - per_screen;
    }
    let scroll = app.scroll_offset;
    let own_window = app.own_window_id.clone().unwrap_or_default();

    // ── Phase 1: collect visible entries at minimum heights ───────────────────
    // We pre-collect owned data so we can compute spare space before rendering,
    // then expand non-selected items into the spare rows if content is available.

    struct RenderItem {
        pane_id: String,
        pane_idx: usize,
        window_id: String,
        status: ClaudeCodeStatus,
        /// Pre-computed icon string — animated spinner frame for Thinking, static otherwise.
        icon: &'static str,
        is_sel: bool,
        is_cur: bool,
        name: String,
        num_str: String,
        branch: String,
        path_short: String,
        preview: Vec<String>, // populated for selected; may get 1 line added for others
        /// Extra non-Claude panes in this window; rendered below the last Claude pane.
        /// Each entry is (path_display, optional_command). Command is None for idle shells.
        extra_panes: Vec<(String, Option<String>)>,
        /// Left-accent-bar color assigned to this pane's window group.
        accent: Color,
    }
    enum Entry { Header { label: String, accent: Color, is_current: bool }, Item(RenderItem) }

    let mut entries: Vec<Entry> = Vec::new();
    let mut rows_used: usize = if scroll > 0 { 1 } else { 0 };
    let mut flat_idx = 0usize;
    let mut group_idx = 0usize;

    'collect: for group in &app.groups {
        let accent = group.color_name.as_deref()
            .and_then(tmux_colour_to_ratatui)
            .unwrap_or(GROUP_ACCENTS[group_idx % GROUP_ACCENTS.len()]);
        group_idx += 1;
        let mut hdr_pushed = false;
        let is_current_group = group.panes.first()
            .map(|p| p.window_id == own_window).unwrap_or(false);

        let last_pane_idx_in_group = group.panes.len().saturating_sub(1);

        for (pane_pos, pane) in group.panes.iter().enumerate() {
            let pane_idx = flat_idx;
            flat_idx += 1;
            if pane_idx < scroll { continue; }

            if !hdr_pushed {
                if rows_used >= area_h { break 'collect; }
                let win_idx = group.panes.first().map(|p| p.window_index.as_str()).unwrap_or("?");
                let srv = group.server.as_deref()
                    .map(|s| format!(" [{}]", s)).unwrap_or_default();
                let prefix = format!("  win {} ", win_idx);
                let name_budget = inner_w.saturating_sub(prefix.len() + srv.len());
                let win_name = truncate(&group.window_name, name_budget);
                entries.push(Entry::Header {
                    label: format!("{}{}{}", prefix, win_name, srv),
                    accent,
                    is_current: is_current_group,
                });
                rows_used += 1;
                hdr_pushed = true;
            }

            let is_sel = pane_idx == sel;
            let is_cur = pane.window_id == own_window;

            let preview: Vec<String> = if is_sel {
                app.pane_content_cache.get(&pane.pane_id)
                    .map(|c| extract_preview_lines(c, inner_w.saturating_sub(1), 6))
                    .unwrap_or_default()
            } else {
                Vec::new()
            };

            // Compute extra pane display data before min_h (row count depends on content).
            let extra_panes: Vec<(String, Option<String>)> = if pane_pos == last_pane_idx_in_group {
                group.extra_panes.iter().map(|ep| {
                    let path_display = shorten_path(&ep.path);
                    let cmd = if is_shell_cmd(&ep.command) { None } else { Some(ep.command.clone()) };
                    (path_display, cmd)
                }).collect()
            } else {
                Vec::new()
            };
            // +1 for the thin HDR_BG divider row that precedes the extra panes block.
            let extra_rows: usize = if extra_panes.is_empty() { 0 } else {
                1 + extra_panes.iter().map(|(_, cmd)| if cmd.is_some() { 2 } else { 1 }).sum::<usize>()
            };

            // min height: 3 content rows + extra pane sub-rows + 1 separator + preview for selected
            let min_h = 3 + extra_rows + 1 + if is_sel { preview.len().max(1) } else { 0 };
            if rows_used + min_h > area_h { break 'collect; }
            rows_used += min_h;

            let branch = pane.git_branch().unwrap_or_else(|| "?".to_string());
            let name = pane_display_name(pane);
            let num_str = format!("%{}", pane.display_num);
            let name_short = truncate(&name, inner_w.saturating_sub(4 + num_str.len()));
            let branch_short = truncate(&branch, inner_w.saturating_sub(1));
            let path_max = if is_sel { inner_w.saturating_sub(12) } else { inner_w.saturating_sub(1) };
            let path_short = truncate(&shorten_path(&pane.current_path), path_max);

            let icon = if pane.status == ClaudeCodeStatus::Thinking {
                crate::session::THINKING_FRAMES[app.thinking_frame % crate::session::THINKING_FRAMES.len()]
            } else {
                pane.status.icon()
            };

            entries.push(Entry::Item(RenderItem {
                pane_id: pane.pane_id.clone(),
                pane_idx,
                window_id: pane.window_id.clone(),
                status: pane.status.clone(),
                icon,
                is_sel, is_cur,
                name: name_short,
                num_str,
                branch: branch_short,
                path_short,
                preview,
                extra_panes,
                accent,
            }));
        }
    }

    // Scroll correction: if the selected item wasn't rendered (items above it consumed
    // all available rows), snap scroll_offset to sel so it appears on the next frame.
    let sel_in_view = entries.iter().any(|e| matches!(e, Entry::Item(ri) if ri.pane_idx == sel));
    if !sel_in_view {
        app.scroll_offset = sel;
    }

    // ── Phase 2: expand non-selected items into spare rows ────────────────────
    let mut spare = area_h.saturating_sub(rows_used);
    if spare > 0 {
        for entry in &mut entries {
            if spare == 0 { break; }
            if let Entry::Item(ref mut item) = entry {
                if !item.is_sel && item.preview.is_empty() {
                    let p = app.pane_content_cache.get(&item.pane_id)
                        .and_then(|c| {
                            let v = extract_preview_lines(c, inner_w.saturating_sub(1), 1);
                            if v.is_empty() { None } else { Some(v) }
                        });
                    if let Some(lines) = p {
                        item.preview = lines;
                        spare -= 1;
                    }
                }
            }
        }
    }

    // ── Phase 3: render ───────────────────────────────────────────────────────
    let mut lines: Vec<Line> = Vec::new();
    let mut click_rows: Vec<(u16, usize)> = Vec::new();

    if scroll > 0 {
        lines.push(Line::from(Span::styled(
            format!("  ↑ {} above", scroll),
            Style::default().fg(Color::DarkGray).bg(sidebar_bg),
        )));
    }

    for entry in &entries {
        match entry {
            Entry::Header { label, accent, is_current } => {
                let label_rest = if label.starts_with(' ') { &label[1..] } else { label.as_str() };
                let hdr_bg = if *is_current {
                    // Slightly brighter row background for the active window header.
                    Color::Rgb(35, 40, 55)
                } else {
                    HDR_BG
                };
                let bar = if *is_current {
                    // Filled block + bold accent — much more visible than a thin ▶.
                    Span::styled("▌▶", Style::default().fg(*accent).bg(hdr_bg).add_modifier(Modifier::BOLD))
                } else {
                    Span::styled("▎ ", Style::default().fg(*accent).bg(hdr_bg))
                };
                lines.push(Line::from(vec![
                    bar,
                    Span::styled(label_rest.to_string(),
                        Style::default().fg(*accent).bg(hdr_bg)
                            .add_modifier(if *is_current { Modifier::BOLD } else { Modifier::empty() })),
                    Span::styled(" ".repeat(area_w), Style::default().bg(hdr_bg)),
                ]));
            }
            Entry::Item(item) => {
                let sc = status_color(&item.status);
                let icon = item.icon;
                let is_alerted = app.alerted_windows.contains(&item.window_id);
                const ALERT_COLOR: Color = Color::Rgb(60, 180, 180);
                let needs_attention = is_alerted || item.status == ClaudeCodeStatus::WaitingInput;
                // Only show selection styling when the sidebar itself has focus.
                let is_sel = item.is_sel && app.is_focused;
                let blink = app.blink_phase && needs_attention && !is_sel;

                // ON phase: vivid bright background — obvious contrast with normal dark rows.
                // OFF phase: subtle tint so the row still reads as "needs attention".
                let row_bg: Color = if is_sel {
                    SEL_BG
                } else if item.status == ClaudeCodeStatus::WaitingInput {
                    if blink { Color::Rgb(130, 115, 10) } else { Color::Rgb(30, 28, 10) }
                } else if is_alerted {
                    if blink { Color::Rgb(10, 80, 80) } else { Color::Rgb(10, 35, 35) }
                } else {
                    ROW_BG
                };

                let sp = |fg: Color| Style::default().fg(fg).bg(row_bg);
                let base = Style::default().bg(row_bg);
                let fill = || Span::styled(" ".repeat(area_w), base);

                let win_span = Span::styled("▎", Style::default().fg(item.accent).bg(row_bg));
                let alert_fg = if blink { Color::Rgb(180, 240, 240) } else { ALERT_COLOR };
                let wait_fg  = if blink { Color::Rgb(255, 245, 150) } else { Color::Yellow };
                let sel_span = if is_sel {
                    Span::styled("▌", sp(sc))
                } else if item.status == ClaudeCodeStatus::WaitingInput {
                    Span::styled("▌", sp(wait_fg))
                } else if is_alerted {
                    Span::styled("▌", sp(alert_fg))
                } else {
                    Span::styled(" ", base)
                };
                let name_fg = if !is_sel && item.status == ClaudeCodeStatus::WaitingInput {
                    wait_fg
                } else if !is_sel && is_alerted {
                    alert_fg
                } else {
                    Color::White
                };
                // Native terminal blink on the name when attention needed and not selected.
                let name_mod = if needs_attention && !is_sel {
                    Modifier::BOLD | Modifier::SLOW_BLINK
                } else if is_sel || item.is_cur {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                };
                let left_len = 2 + 1 + icon.chars().count() + 1 + item.name.chars().count();
                let pad = area_w.saturating_sub(left_len + item.num_str.len());

                click_rows.push((area.y + lines.len() as u16, item.pane_idx));

                // ── Line 1: [W][S] icon name ··· %N ──────────────────────────
                lines.push(Line::from(vec![
                    win_span, sel_span,
                    Span::styled(format!(" {} ", icon), sp(sc)),
                    Span::styled(item.name.clone(),
                        Style::default().fg(name_fg).add_modifier(name_mod).bg(row_bg)),
                    Span::styled(" ".repeat(pad), base),
                    Span::styled(item.num_str.clone(), sp(item.accent)),
                    fill(),
                ]).style(base));

                let cont_pipe: Option<Span> = if is_sel {
                    Some(Span::styled("▌", sp(sc)))
                } else if item.status == ClaudeCodeStatus::WaitingInput {
                    Some(Span::styled("▌", sp(wait_fg)))
                } else if is_alerted {
                    Some(Span::styled("▌", sp(alert_fg)))
                } else {
                    None
                };

                // ── Line 2: branch ────────────────────────────────────────────
                let bar2 = Span::styled("▎", Style::default().fg(item.accent).bg(row_bg));
                if let Some(ref pipe) = cont_pipe {
                    lines.push(Line::from(vec![
                        bar2, pipe.clone(),
                        Span::styled(format!(" {}", item.branch),
                            sp(if is_sel { Color::Cyan } else { Color::Rgb(80, 90, 110) })),
                        fill(),
                    ]).style(base));
                } else {
                    lines.push(Line::from(vec![
                        bar2,
                        Span::styled(format!("  {}", item.branch), sp(Color::Rgb(80, 90, 110))),
                        fill(),
                    ]).style(base));
                }

                // ── Line 3: path + status ─────────────────────────────────────
                let bar3 = || Span::styled("▎", Style::default().fg(item.accent).bg(row_bg));
                if is_sel {
                    let status_label = match &item.status {
                        ClaudeCodeStatus::Working => "● Working".to_string(),
                        ClaudeCodeStatus::Thinking => format!("{} Thinking", item.icon),
                        ClaudeCodeStatus::WaitingInput => "⚠ Waiting".to_string(),
                        ClaudeCodeStatus::Idle => "○ Idle".to_string(),
                        ClaudeCodeStatus::Unknown => "○ Unknown".to_string(),
                    };
                    lines.push(Line::from(vec![
                        bar3(), Span::styled("▌", sp(sc)),
                        Span::styled(format!(" {}  ", item.path_short), sp(Color::DarkGray)),
                        Span::styled(status_label, sp(sc)),
                        fill(),
                    ]).style(base));
                } else if item.status == ClaudeCodeStatus::WaitingInput {
                    lines.push(Line::from(vec![
                        bar3(), Span::styled("▌", sp(wait_fg)),
                        Span::styled(format!(" {} ", item.path_short), sp(Color::Rgb(80, 78, 40))),
                        Span::styled("⚠ Waiting", sp(wait_fg)),
                        fill(),
                    ]).style(base));
                } else if is_alerted {
                    lines.push(Line::from(vec![
                        bar3(), Span::styled("▌", sp(alert_fg)),
                        Span::styled(format!(" {} ", item.path_short), sp(Color::Rgb(55, 58, 68))),
                        Span::styled("● Done", sp(alert_fg)),
                        fill(),
                    ]).style(base));
                } else {
                    lines.push(Line::from(vec![
                        bar3(),
                        Span::styled(format!("  {}", item.path_short), sp(Color::Rgb(55, 58, 68))),
                        fill(),
                    ]).style(base));
                }

                // ── Lines 4+: content preview ─────────────────────────────────
                let bar4 = || Span::styled("▎", Style::default().fg(item.accent).bg(row_bg));
                if item.preview.is_empty() {
                    if is_sel {
                        lines.push(Line::from(vec![
                            bar4(), Span::styled("▌", sp(sc)),
                            Span::styled(" —", sp(Color::Rgb(70, 72, 85))),
                            fill(),
                        ]).style(base));
                    }
                } else if is_sel {
                    for pl in &item.preview {
                        lines.push(Line::from(vec![
                            bar4(), Span::styled("▌", sp(sc)),
                            Span::styled(format!(" {}", pl), sp(Color::Rgb(140, 145, 165))),
                            fill(),
                        ]).style(base));
                    }
                } else {
                    // Expanded non-selected: 1 dim preview line, no pipe
                    for pl in &item.preview {
                        lines.push(Line::from(vec![
                            bar4(),
                            Span::styled(format!("  {}", pl), sp(Color::Rgb(75, 80, 100))),
                            fill(),
                        ]).style(base));
                    }
                }

                // ── Extra (non-Claude) panes ──────────────────────────────────
                let extra_bar = || Span::styled("▎", Style::default().fg(item.accent).bg(HDR_BG));
                if !item.extra_panes.is_empty() {
                    lines.push(Line::from(vec![
                        extra_bar(),
                        Span::styled(" ".repeat(area_w), Style::default().bg(HDR_BG)),
                    ]));
                }
                for (path_display, cmd) in &item.extra_panes {
                    // Line 1: path (always)
                    let path_label = truncate(
                        &format!("  · {}", path_display),
                        inner_w.saturating_sub(1),
                    );
                    lines.push(Line::from(vec![
                        Span::styled("▎", Style::default().fg(item.accent).bg(ROW_BG)),
                        Span::styled(path_label, Style::default().fg(Color::Rgb(60, 65, 82)).bg(ROW_BG)),
                        Span::styled(" ".repeat(area_w), Style::default().bg(ROW_BG)),
                    ]));
                    // Line 2: command (only when something real is running, not a bare shell)
                    if let Some(command) = cmd {
                        let cmd_label = truncate(
                            &format!("    {}", command),
                            inner_w.saturating_sub(1),
                        );
                        lines.push(Line::from(vec![
                            Span::styled("▎", Style::default().fg(item.accent).bg(ROW_BG)),
                            Span::styled(cmd_label, Style::default().fg(Color::Rgb(50, 55, 72)).bg(ROW_BG)),
                            Span::styled(" ".repeat(area_w), Style::default().bg(ROW_BG)),
                        ]));
                    }
                }

                // ── Separator ─────────────────────────────────────────────────
                lines.push(Line::from(Span::styled(
                    " ".repeat(area_w),
                    Style::default().bg(sidebar_bg),
                )));
            }
        }
    }

    if flat_idx < total_items {
        lines.push(Line::from(Span::styled(
            format!("  ↓ {} below", total_items - flat_idx),
            Style::default().fg(Color::DarkGray).bg(sidebar_bg),
        )));
    }

    // ── Agents section ────────────────────────────────────────────────────────
    if !app.jobs.is_empty() {
        let pane_count = total_items;

        // How many rows are left after panes?
        let rows_so_far = lines.len();
        let remaining = area_h.saturating_sub(rows_so_far);

        // Each agent: 3 content rows + 1 separator = 4 rows.
        // Reserve 1 for the header + 1 for a potential "↑ N above" indicator.
        const AGENT_ROWS: usize = 4;
        let max_vis = remaining.saturating_sub(2) / AGENT_ROWS;

        // Compute jobs_scroll: keep the selected agent within the visible window.
        let jobs_sel = if sel >= pane_count && sel < pane_count + app.jobs.len() {
            sel - pane_count
        } else {
            usize::MAX
        };
        let prev_js = app.jobs_scroll_offset;
        let jobs_scroll = if max_vis == 0 {
            0
        } else if jobs_sel == usize::MAX {
            prev_js.min(app.jobs.len().saturating_sub(max_vis))
        } else if jobs_sel < prev_js {
            jobs_sel
        } else if jobs_sel >= prev_js + max_vis {
            jobs_sel + 1 - max_vis
        } else {
            prev_js
        };
        app.jobs_scroll_offset = jobs_scroll;

        if max_vis > 0 || remaining >= 1 {
            let header = format!("agents ({})", app.jobs.len());
            lines.push(titled_sep(&header, area_w));

            if jobs_scroll > 0 {
                lines.push(Line::from(Span::styled(
                    format!("  ↑ {} above", jobs_scroll),
                    Style::default().fg(Color::DarkGray).bg(sidebar_bg),
                )));
            }

            let mut shown = 0;
            for (job_idx, job) in app.jobs.iter().enumerate().skip(jobs_scroll) {
                if shown >= max_vis { break; }

                let global_idx = pane_count + job_idx;
                let is_sel = global_idx == sel && app.is_focused;

                // Accent and row background based on status
                const BLOCKED_ACCENT: Color = Color::Rgb(200, 130, 30);
                const WORKING_ACCENT: Color = Color::Rgb(50,  160,  70);
                const IDLE_ACCENT:    Color = Color::Rgb(80,   85, 105);

                let accent = match job.status {
                    crate::jobs::JobStatus::Blocked  => BLOCKED_ACCENT,
                    crate::jobs::JobStatus::Working  => WORKING_ACCENT,
                    _                                => IDLE_ACCENT,
                };
                let status_fg = accent;

                let row_bg: Color = if is_sel {
                    SEL_BG
                } else if job.status == crate::jobs::JobStatus::Blocked {
                    if app.blink_phase { Color::Rgb(50, 30, 5) } else { Color::Rgb(32, 22, 8) }
                } else {
                    ROW_BG
                };

                let sp   = |fg: Color| Style::default().fg(fg).bg(row_bg);
                let base = Style::default().bg(row_bg);
                let fill = || Span::styled(" ".repeat(area_w), base);

                // ── Line 1: status icon + name + display num ──────────────────
                let icon = job.status.icon();
                let num_str = format!("⧉{}", job.display_num);
                let name_budget = inner_w.saturating_sub(2 + 2 + num_str.len() + 1);
                let name_short = truncate(&job.name, name_budget);
                let padding = inner_w
                    .saturating_sub(2 + name_short.chars().count() + 2 + num_str.len());
                lines.push(Line::from(vec![
                    Span::styled(if is_sel { "▌" } else { " " }, sp(status_fg)),
                    Span::styled(format!(" {} ", icon), sp(status_fg)),
                    Span::styled(name_short.clone(),
                        Style::default().fg(Color::Rgb(200, 205, 220)).bg(row_bg)
                            .add_modifier(if is_sel { Modifier::BOLD } else { Modifier::empty() })),
                    Span::styled(" ".repeat(padding), base),
                    Span::styled(num_str, sp(Color::Rgb(80, 85, 105))),
                    fill(),
                ]).style(base));

                // ── Line 2: needs (blocked) or detail (working) ───────────────
                let line2_text = if job.status == crate::jobs::JobStatus::Blocked {
                    job.needs.as_deref()
                        .map(|n| format!("  needs: {}", n))
                        .unwrap_or_else(|| format!("  {}", job.detail))
                } else {
                    format!("  {}", job.detail)
                };
                let line2_fg = if job.status == crate::jobs::JobStatus::Blocked {
                    Color::Rgb(220, 175, 100)
                } else {
                    Color::Rgb(120, 125, 145)
                };
                lines.push(Line::from(vec![
                    Span::styled(
                        truncate(&line2_text, inner_w.saturating_sub(1)),
                        sp(line2_fg)),
                    fill(),
                ]).style(base));

                // ── Line 3: cwd + age ─────────────────────────────────────────
                let age = format_age(job.age_secs());
                let cwd_short = {
                    let home = std::env::var("HOME").unwrap_or_default();
                    let s = job.cwd.to_string_lossy();
                    if s.starts_with(&home) {
                        format!("~{}", &s[home.len()..])
                    } else {
                        s.to_string()
                    }
                };
                let meta = format!("  {} · {}", cwd_short, age);
                lines.push(Line::from(vec![
                    Span::styled(truncate(&meta, inner_w.saturating_sub(1)),
                        sp(Color::Rgb(65, 68, 85))),
                    fill(),
                ]).style(base));

                // ── Separator ─────────────────────────────────────────────────
                lines.push(Line::from(Span::styled(
                    " ".repeat(area_w),
                    Style::default().bg(sidebar_bg),
                )));

                shown += 1;
            }

            let clipped = app.jobs.len().saturating_sub(jobs_scroll + shown);
            if clipped > 0 {
                lines.push(Line::from(Span::styled(
                    format!("  ↓ {} more", clipped),
                    Style::default().fg(Color::DarkGray).bg(sidebar_bg),
                )));
            }
        }
    }

    app.pane_click_rows = click_rows;
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(sidebar_bg)),
        area,
    );
}

fn format_age(secs: i64) -> String {
    if secs < 60        { "just now".to_string() }
    else if secs < 3600 { format!("{}m ago", secs / 60) }
    else if secs < 86400 { format!("{}h ago", secs / 3600) }
    else                { format!("{}d ago", secs / 86400) }
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

const INFO_BG: Color = Color::Rgb(16, 18, 24);

fn usage_color(pct: f32) -> Color {
    if pct >= 80.0 { Color::Rgb(255, 100, 80) }
    else if pct >= 50.0 { Color::Rgb(255, 220, 80) }
    else { Color::Rgb(150, 220, 150) }
}

fn fmt_time_left(target_epoch: i64, now: i64) -> String {
    let rem = (target_epoch - now).max(0);
    let d = rem / 86400;
    let h = (rem % 86400) / 3600;
    let m = (rem % 3600) / 60;
    if d > 0 { format!("{}d{}h", d, h) } else if h > 0 { format!("{}h{}m", h, m) } else { format!("{}m", m) }
}

fn fmt_time_ago(source_epoch: i64, now: i64) -> String {
    let diff = (now - source_epoch).max(0);
    match diff {
        d if d >= 86400 => format!("{}d ago", d / 86400),
        d if d >= 3600  => format!("{}h ago", d / 3600),
        d if d >= 60    => format!("{}m ago", d / 60),
        _               => "just now".into(),
    }
}

fn render_global_info(frame: &mut Frame, app: &App, area: Rect) {
    let gi = &app.global_info;
    let info_style = Style::default().bg(INFO_BG);
    let dim = |fg: Color| Style::default().fg(fg).bg(INFO_BG);

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let w = area.width as usize;

    // Row 0: usage percentages with live reset countdowns
    let mut usage_spans = vec![Span::styled(" ", info_style)];
    if let Some(u5) = gi.usage_5h {
        let clr = usage_color(u5);
        usage_spans.push(Span::styled(format!("5h:{:.0}%", u5), dim(clr)));
        if let Some(at) = gi.reset_5h_at {
            usage_spans.push(Span::styled(
                format!("({})", fmt_time_left(at, now)),
                dim(Color::Rgb(80, 85, 100)),
            ));
        }
    }
    if let Some(u7) = gi.usage_7d {
        if usage_spans.len() > 1 { usage_spans.push(Span::raw("  ")); }
        let clr = usage_color(u7);
        usage_spans.push(Span::styled(format!("7d:{:.0}%", u7), dim(clr)));
        if let Some(at) = gi.reset_7d_at {
            usage_spans.push(Span::styled(
                format!("({})", fmt_time_left(at, now)),
                dim(Color::Rgb(80, 85, 100)),
            ));
        }
    }
    if let Some(at) = gi.usage_updated_at {
        usage_spans.push(Span::raw("  "));
        usage_spans.push(Span::styled(
            fmt_time_ago(at, now),
            dim(Color::Rgb(70, 75, 95)),
        ));
    }

    // Row 2: MemPalace stats
    let mp_clr = Color::Rgb(165, 135, 210);
    let mut mp_parts: Vec<String> = Vec::new();
    if let Some(d) = &gi.mp_drawers {
        let mut s = d.clone();
        if let Some(sz) = &gi.mp_size { s.push_str(&format!("({})", sz)); }
        mp_parts.push(s);
    }
    if let (Some(w), Some(r)) = (gi.mp_wings, gi.mp_rooms) {
        mp_parts.push(format!("{}W/{}R", w, r));
    }
    if let Some(at) = gi.mp_last_at {
        mp_parts.push(fmt_time_ago(at, now));
    }
    let mp_str = mp_parts.join(" · ");

    let mp_line = Line::from(vec![
        Span::styled(" 🏛 ", dim(mp_clr)),
        Span::styled(mp_str, dim(mp_clr)),
    ]);

    frame.render_widget(
        Paragraph::new(vec![
            titled_sep("Claude usage", w),
            Line::from(usage_spans),
            titled_sep("MemPalace", w),
            mp_line,
            titled_sep("Shortcuts", w),
        ]).style(info_style),
        area,
    );
}

fn render_footer(frame: &mut Frame, app: &App, area: Rect) {
    match &app.mode {
        Mode::Normal | Mode::ActionHints => {
            let is_job = app.selected_job().is_some();
            frame.render_widget(
                Paragraph::new(vec![
                    Line::from(vec![
                        key("j/k"), hint(" nav"), sep(),
                        if is_job { key("Enter") } else { key("Enter") },
                        hint(if is_job { " open" } else { " focus" }), sep(),
                        key("1-9"), hint(" jump"),
                    ]),
                    Line::from(vec![
                        key("i"), hint(if is_job { " reply" } else { " send" }), sep(),
                        if is_job { key("r") } else { key("l") },
                        hint(if is_job { " resume" } else { " actions" }), sep(),
                        key("w"), hint(" wt"), sep(),
                        key("c"), hint(" new"), sep(),
                        key("e"), hint(" edit"), sep(),
                        key("K"), hint(" kill"),
                    ]),
                    Line::from(vec![
                        key("s"), hint(" sticky"), sep(),
                        key("o"), hint(" houston"), sep(),
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
        Mode::Compose { text, cursor } => {
            let text_style = Style::default().fg(Color::White);
            let mut spans = vec![Span::styled("› ", Style::default().fg(Color::Cyan))];
            spans.extend(text_with_cursor(text.as_str(), *cursor, text_style, true));
            frame.render_widget(
                Paragraph::new(vec![
                    Line::from(spans),
                    Line::from(vec![
                        key("Enter"), hint(" send  "), key("Esc"), hint(" cancel"),
                    ]),
                    Line::from(Span::raw("")),
                ]),
                area,
            );
        }
        Mode::EditWindow { .. } => {
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    key("Tab"), hint(" next field  "), key("Enter"), hint(" apply  "), key("Esc"), hint(" cancel"),
                ])),
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
        Mode::FolderPick(_) => {}
        Mode::History(_) => {}
    }
}

fn render_own_metrics(frame: &mut Frame, app: &App, area: Rect) {
    use super::hostmem::fmt_mem;
    let w = area.width as usize;
    let avg = app.own_cpu_avg();
    let max_cpu = app.own_cpu_max();
    let val_color = |v: f32| {
        if v >= 10.0 { Color::Rgb(255, 100, 80) }
        else if v >= 3.0 { Color::Rgb(255, 210, 80) }
        else { Color::Rgb(100, 105, 125) }
    };
    let dim = Color::Rgb(55, 58, 72);
    let info_style = Style::default().bg(INFO_BG);
    let ds = |fg: Color| Style::default().fg(fg).bg(INFO_BG);

    // Host-app memory thresholds (MB).
    let rss_color = |mb: f32| {
        if mb >= 8192.0 { Color::Rgb(255, 100, 80) }
        else if mb >= 4096.0 { Color::Rgb(255, 210, 80) }
        else { Color::Rgb(100, 105, 125) }
    };
    let swap_color = |mb: f32| {
        if mb >= 4096.0 { Color::Rgb(255, 100, 80) }
        else if mb >= 1024.0 { Color::Rgb(255, 210, 80) }
        else { Color::Rgb(100, 105, 125) }
    };

    // --- Separator line: "ccmux" + the host app name and its RSS, right-aligned. ---
    // The sidebar is narrow (~48 cols), so we split the host metrics across the
    // panel's two lines: app name + resident memory here, system swap on the data
    // line. Each is labelled and fits without dropping ccmux's own metrics.
    let title_clr = Color::Rgb(85, 90, 110);
    let sep_line = match &app.host_app {
        // Reserve: leading space + name + space + rss + trailing space, plus room
        // for the "ccmux" label itself (+8).
        Some(host)
            if w > host.name.chars().count() + fmt_mem(app.host_app_rss_mb).chars().count() + 11 =>
        {
            let rss_str = fmt_mem(app.host_app_rss_mb);
            // Appended segment width must equal what we subtract from the separator.
            let seg_w = host.name.chars().count() + rss_str.chars().count() + 3;
            let mut spans = titled_sep("ccmux", w - seg_w).spans;
            spans.push(Span::styled(format!(" {} ", host.name), ds(title_clr)));
            spans.push(Span::styled(format!("{} ", rss_str), ds(rss_color(app.host_app_rss_mb))));
            Line::from(spans)
        }
        _ => titled_sep("ccmux", w),
    };

    // --- Data line: ccmux cpu/mem left, host system-swap right (behind a divider). ---
    let divider = Color::Rgb(50, 53, 65);
    let left: Vec<Span> = vec![
        Span::styled("  cpu ", ds(dim)),
        Span::styled(format!("avg {:.1}%", avg), ds(val_color(avg))),
        Span::styled("  max ", ds(dim)),
        Span::styled(format!("{:.1}%", max_cpu), ds(val_color(max_cpu))),
        Span::styled("  │  ", ds(divider)),
        Span::styled(format!("{:.0} MB", app.own_rss_mb), ds(dim)),
    ];

    let right: Vec<Span> = if app.host_app.is_some() {
        vec![
            Span::styled("│ ", ds(divider)),
            Span::styled("sw ", ds(dim)),
            Span::styled(fmt_mem(app.system_swap_mb), ds(swap_color(app.system_swap_mb))),
        ]
    } else {
        Vec::new()
    };

    let left_w: usize = left.iter().map(|s| s.content.chars().count()).sum();
    let right_w: usize = right.iter().map(|s| s.content.chars().count()).sum();
    let mut data_spans = left;
    if !right.is_empty() && left_w + right_w + 2 <= w {
        let pad = w - left_w - right_w;
        data_spans.push(Span::styled(" ".repeat(pad), info_style));
        data_spans.extend(right);
    }

    frame.render_widget(
        Paragraph::new(vec![sep_line, Line::from(data_spans)]).style(info_style),
        area,
    );
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
        row("c",      "New session — pick folder, launch Claude"),
        row("w",      "New worktree (fetch → branch → options)"),
        row("o",      "New worktree on ~/dev/houston"),
        row("h",      "Browse Claude history (preview / resume)"),
        row("e",      "Edit window — name and color"),
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

fn render_edit_window_overlay(frame: &mut Frame, app: &App, area: Rect) {
    use crate::config::WINDOW_COLORS;

    let (name, color_idx, field, name_cursor) = match &app.mode {
        Mode::EditWindow { name, color_idx, field, name_cursor, .. } =>
            (name.as_str(), *color_idx, *field, *name_cursor),
        _ => return,
    };

    let color_name = WINDOW_COLORS.get(color_idx).map(|c| c.0).unwrap_or("none");

    let field_style = |f: u8| -> (Style, Style) {
        if field == f {
            (Style::default().fg(Color::Cyan), Style::default().fg(Color::White))
        } else {
            (Style::default().fg(Color::DarkGray), Style::default().fg(Color::Rgb(100, 100, 110)))
        }
    };

    let (label0, val0) = field_style(0);
    let (label1, val1) = field_style(1);

    let mut name_spans: Vec<Span> = vec![Span::styled("  Name:   ", label0)];
    name_spans.extend(text_with_cursor(name, name_cursor, val0, field == 0));

    let lines: Vec<Line> = vec![
        Line::from(name_spans),
        Line::from(vec![
            Span::styled("  Color:  ", label1),
            Span::styled("◀ ", val1),
            Span::styled(color_name, val1),
            Span::styled(" ▶", val1),
        ]),
        Line::from(vec![
            Span::raw("  "),
            key("Tab"), hint(" next field  "),
            key("^B"), hint(" 🤖 toggle  "),
            key("Enter"), hint(" apply  "), key("Esc"), hint(" cancel"),
        ]),
    ];

    let overlay = overlay_rect(area, lines.len());
    frame.render_widget(Clear, overlay);
    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::default()
                .title(" Edit window ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan)))
            .style(Style::default().bg(Color::Rgb(18, 20, 26))),
        overlay,
    );
}

fn render_new_window_overlay(frame: &mut Frame, app: &App, area: Rect) {
    let (name, color_idx, launch_claude, field, name_cursor) = match &app.mode {
        Mode::NewWindow { name, color_idx, launch_claude, field, name_cursor } =>
            (name.as_str(), *color_idx, *launch_claude, *field, *name_cursor),
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

    let (label0, val0) = field_style(0);
    let (label1, val1) = field_style(1);
    let (label2, val2) = field_style(2);

    let mut name_span: Vec<Span> = vec![Span::styled("  Name:    ", label0)];
    name_span.extend(text_with_cursor(name, name_cursor, val0, field == 0));

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
                branches, filter, filter_cursor, cursor, entering_new, new_branch_text, new_branch_cursor, ..
            } => {
                render_branch_select_overlay(frame, area, branches, filter, *filter_cursor, *cursor, *entering_new, new_branch_text, *new_branch_cursor);
            }
            WorktreeStep::FolderName { folder, cursor, .. } => {
                render_folder_name_overlay(frame, area, folder, *cursor);
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
    filter_cursor: usize,
    cursor: usize,
    entering_new: bool,
    new_branch_text: &str,
    new_branch_cursor: usize,
) {
    let overlay = area;
    let white = Style::default().fg(Color::White);
    let mut lines: Vec<Line> = Vec::new();

    if entering_new {
        let mut spans = vec![Span::styled("  New branch: ", Style::default().fg(Color::Cyan))];
        spans.extend(text_with_cursor(new_branch_text, new_branch_cursor, white, true));
        lines.push(Line::from(spans));
        lines.push(Line::from(vec![
            Span::raw("  "),
            key("Enter"), hint(" create  "), key("Tab"), hint("/"), key("F"), hint(" existing  "), key("Esc"), hint(" cancel"),
        ]));
    } else {
        let mut filter_spans = vec![Span::styled("  Filter: ", Style::default().fg(Color::Cyan))];
        filter_spans.extend(text_with_cursor(filter, filter_cursor, white, true));
        lines.push(Line::from(filter_spans));

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

fn render_folder_name_overlay(frame: &mut Frame, area: Rect, folder: &str, cursor: usize) {
    let mut path_spans = vec![Span::styled("  Path: ", Style::default().fg(Color::Cyan))];
    path_spans.extend(text_with_cursor(folder, cursor, Style::default().fg(Color::White), true));
    let lines: Vec<Line> = vec![
        Line::from(path_spans),
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
                .title(" New worktree — path ")
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
    let (lb, vb) = field_style(5);

    let mut lines: Vec<Line> = vec![
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
    ];

    if let Some(base) = &opts.base_branch {
        let mut base_spans = vec![Span::styled("  Base:    ", lb)];
        base_spans.extend(text_with_cursor(base, opts.base_branch_cursor, vb, opts.field == 5));
        lines.push(Line::from(base_spans));
    }

    lines.push(Line::from(vec![
        Span::raw("  "),
        key("Tab"), hint(" next  "), key("◀▶"), hint(" cycle  "), key("Space"), hint(" toggle  "), key("Enter"), hint(" create"),
    ]));

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

fn render_folder_pick_overlay(frame: &mut Frame, app: &App, area: Rect) {
    use crate::sidebar::mode::FolderPickStep;
    use std::path::PathBuf;

    let bg = Color::Rgb(22, 25, 34);
    let border_clr = Color::Rgb(80, 100, 160);
    let title_clr = Color::Rgb(140, 170, 220);
    let sel_bg = Color::Rgb(45, 55, 80);
    let dim_clr = Color::Rgb(100, 110, 130);
    let git_clr = Color::Rgb(100, 200, 140);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_clr))
        .title(Span::styled(" New Session ", Style::default().fg(title_clr)))
        .style(Style::default().bg(bg));

    let inner = block.inner(area);
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);

    match &app.mode {
        Mode::FolderPick(FolderPickStep::Scanning) => {
            let line = Line::from(Span::styled(" Scanning…", Style::default().fg(dim_clr)));
            frame.render_widget(Paragraph::new(line).style(Style::default().bg(bg)), inner);
        }
        Mode::FolderPick(FolderPickStep::Picking { root, dirs, filter, filter_cursor, cursor }) => {
            let h = inner.height as usize;
            if h < 3 { return; }

            // Row 0: current root path
            let root_str = shorten_path(root);
            let root_line = Line::from(vec![
                Span::styled(" ", Style::default().bg(bg)),
                Span::styled(root_str, Style::default().fg(dim_clr).bg(bg)),
                Span::styled("/", Style::default().fg(border_clr).bg(bg)),
            ]);

            // Row 1: filter input
            let mut filter_spans = vec![Span::styled(" > ", Style::default().fg(border_clr).bg(bg))];
            filter_spans.extend(text_with_cursor(filter.as_str(), *filter_cursor, Style::default().fg(Color::White).bg(bg), true));
            let filter_line = Line::from(filter_spans);

            let list_h = h.saturating_sub(3); // root + filter + hint
            let filtered: Vec<&PathBuf> = dirs.iter()
                .filter(|d| d.file_name()
                    .map(|n| n.to_string_lossy().to_lowercase().contains(&filter.to_lowercase()))
                    .unwrap_or(false))
                .collect();
            let filtered_len = filtered.len();
            let cursor = *cursor;
            let clamped = if filtered_len == 0 { 0 } else { cursor.min(filtered_len - 1) };

            // Scroll to keep cursor visible
            let scroll = if clamped >= list_h { clamped + 1 - list_h } else { 0 };

            let can_create = filtered_len == 0 && !filter.is_empty();

            let mut list_lines: Vec<Line> = Vec::new();
            if can_create {
                // No matching dir — offer to create one.
                list_lines.push(Line::from(vec![
                    Span::styled(" + ", Style::default().fg(git_clr).bg(sel_bg)),
                    Span::styled("Create  ", Style::default().fg(git_clr).bg(sel_bg)),
                    Span::styled(filter.as_str(), Style::default().fg(Color::White).bg(sel_bg).add_modifier(Modifier::BOLD)),
                ]));
            } else {
                for (i, path) in filtered.iter().enumerate().skip(scroll).take(list_h) {
                    let name = path.file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    let is_git = path.join(".git").exists();
                    let is_selected = i == clamped;

                    let (line_bg, name_style) = if is_selected {
                        (sel_bg, Style::default().fg(Color::White).bg(sel_bg).add_modifier(Modifier::BOLD))
                    } else {
                        (bg, Style::default().fg(Color::Rgb(200, 210, 230)).bg(bg))
                    };

                    let prefix = if is_git { "⎇ " } else { "  " };
                    let prefix_clr = if is_git { git_clr } else { dim_clr };
                    let branch = if is_git { read_git_branch(path) } else { None };

                    let mut spans = vec![
                        Span::styled(" ", Style::default().bg(line_bg)),
                        Span::styled(prefix, Style::default().fg(prefix_clr).bg(line_bg)),
                        Span::styled(name, name_style),
                    ];
                    if let Some(b) = branch {
                        spans.push(Span::styled(
                            format!(" ({})", b),
                            Style::default().fg(Color::Rgb(70, 140, 90)).bg(line_bg),
                        ));
                    }
                    list_lines.push(Line::from(spans));
                }
            }

            // Pad remaining rows
            while list_lines.len() < list_h {
                list_lines.push(Line::from(Span::styled("", Style::default().bg(bg))));
            }

            // Hint row
            let hint_line = if can_create {
                Line::from(vec![
                    Span::styled(" Enter", Style::default().fg(border_clr).bg(bg)),
                    Span::styled(":create  ", Style::default().fg(dim_clr).bg(bg)),
                    Span::styled("Esc", Style::default().fg(border_clr).bg(bg)),
                    Span::styled(":cancel", Style::default().fg(dim_clr).bg(bg)),
                ])
            } else {
                Line::from(vec![
                    Span::styled(" Enter", Style::default().fg(border_clr).bg(bg)),
                    Span::styled(":open  ", Style::default().fg(dim_clr).bg(bg)),
                    Span::styled("→", Style::default().fg(border_clr).bg(bg)),
                    Span::styled(":into  ", Style::default().fg(dim_clr).bg(bg)),
                    Span::styled("←", Style::default().fg(border_clr).bg(bg)),
                    Span::styled(":up  ", Style::default().fg(dim_clr).bg(bg)),
                    Span::styled("Esc", Style::default().fg(border_clr).bg(bg)),
                    Span::styled(":cancel", Style::default().fg(dim_clr).bg(bg)),
                ])
            };

            // Layout: root_line, filter_line, list rows, hint
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Min(1),
                    Constraint::Length(1),
                ])
                .split(inner);

            frame.render_widget(Paragraph::new(root_line).style(Style::default().bg(bg)), chunks[0]);
            frame.render_widget(Paragraph::new(filter_line).style(Style::default().bg(bg)), chunks[1]);
            frame.render_widget(
                Paragraph::new(list_lines).style(Style::default().bg(bg)),
                chunks[2],
            );
            frame.render_widget(Paragraph::new(hint_line).style(Style::default().bg(bg)), chunks[3]);
        }
        Mode::FolderPick(FolderPickStep::Options { path, is_new, opts }) => {
            let label = path.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let prefix = if *is_new { "Create" } else { "Open" };
            let title = format!(" {} \"{}\" — options ", prefix, label);
            render_options_overlay_titled(frame, inner, opts, &title);
        }
        _ => {}
    }
}

fn render_options_overlay_titled(
    frame: &mut Frame,
    area: Rect,
    opts: &crate::sidebar::mode::WorktreeOpts,
    title: &str,
) {
    let field_style = |f: u8| -> (Style, Style) {
        if opts.field == f {
            (Style::default().fg(Color::Cyan), Style::default().fg(Color::White))
        } else {
            (Style::default().fg(Color::DarkGray), Style::default().fg(Color::Rgb(100, 100, 110)))
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
            Span::styled("◀ ", vm), Span::styled(model_name, vm), Span::styled(" ▶", vm),
        ]),
        Line::from(vec![
            Span::styled("  Effort:  ", le),
            Span::styled("◀ ", ve), Span::styled(effort_name, ve), Span::styled(" ▶", ve),
        ]),
        Line::from(vec![
            Span::styled("  Claude:  ", lc),
            Span::styled(format!("{} Launch claude", claude_check), vc),
        ]),
        Line::from(vec![
            Span::styled("  Color:   ", lcol),
            Span::styled("◀ ", vcol), Span::styled(color_name, vcol), Span::styled(" ▶", vcol),
        ]),
        Line::from(vec![
            Span::styled("  VSCode:  ", lv),
            Span::styled(format!("{} Open VSCode", vscode_check), vv),
        ]),
        Line::from(vec![
            Span::raw("  "),
            key("Tab"), hint(" next  "), key("◀▶"), hint(" cycle  "),
            key("Space"), hint(" toggle  "), key("Enter"), hint(" confirm"),
        ]),
    ];

    let overlay = overlay_rect(area, lines.len());
    frame.render_widget(Clear, overlay);
    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::default()
                .title(title.to_string())
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan)))
            .style(Style::default().bg(Color::Rgb(18, 20, 26))),
        overlay,
    );
}

/// Read the current branch name from a git repo's .git/HEAD without spawning a subprocess.
fn read_git_branch(path: &std::path::Path) -> Option<String> {
    let git_path = path.join(".git");
    let head_path = if git_path.is_dir() {
        git_path.join("HEAD")
    } else {
        // Worktree: .git is a file "gitdir: /path/to/actual/gitdir"
        let content = std::fs::read_to_string(&git_path).ok()?;
        let gitdir = content.strip_prefix("gitdir: ")?.trim();
        std::path::PathBuf::from(gitdir).join("HEAD")
    };
    let head = std::fs::read_to_string(head_path).ok()?;
    head.trim().strip_prefix("ref: refs/heads/").map(|s| s.to_string())
}

fn status_color(status: &ClaudeCodeStatus) -> Color {
    match status {
        ClaudeCodeStatus::Working => Color::Green,
        ClaudeCodeStatus::Thinking => Color::Rgb(160, 100, 220), // purple
        ClaudeCodeStatus::WaitingInput => Color::Yellow,
        ClaudeCodeStatus::Idle => Color::Cyan,
        ClaudeCodeStatus::Unknown => Color::DarkGray,
    }
}

fn render_history(frame: &mut Frame, app: &App, area: Rect) {
    use crate::sidebar::mode::HistoryStep;

    let bg = Color::Rgb(22, 25, 34);
    let border_clr = Color::Rgb(80, 100, 160);
    let title_clr = Color::Rgb(140, 170, 220);
    let sel_bg = Color::Rgb(45, 55, 80);
    let dim_clr = Color::Rgb(100, 110, 130);
    let dead_clr = Color::Rgb(70, 75, 90);
    let meta_clr = Color::Rgb(90, 100, 120);
    let branch_clr = Color::Rgb(100, 200, 140);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_clr))
        .title(Span::styled(" Session History ", Style::default().fg(title_clr)))
        .style(Style::default().bg(bg));

    let inner = block.inner(area);
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);

    match &app.mode {
        Mode::History(HistoryStep::Loading) => {
            let line = Line::from(Span::styled(
                " Loading Claude history\u{2026}",
                Style::default().fg(dim_clr),
            ));
            frame.render_widget(Paragraph::new(line).style(Style::default().bg(bg)), inner);
        }
        Mode::History(HistoryStep::List { entries, filter, filter_cursor, cursor, .. }) => {
            let h = inner.height as usize;
            if h < 3 { return; }

            // Row 0: filter input
            let mut filter_spans = vec![Span::styled(" > ", Style::default().fg(border_clr).bg(bg))];
            filter_spans.extend(text_with_cursor(
                filter.as_str(),
                *filter_cursor,
                Style::default().fg(Color::White).bg(bg),
                true,
            ));
            let filter_line = Line::from(filter_spans);

            // Fuzzy filter entries
            let filtered = crate::sidebar::input::fuzzy_sort(
                entries,
                filter,
                |e| format!("{} {}", e.title, e.branch.clone().unwrap_or_default()),
            );

            let now = std::time::SystemTime::now();

            // Build list rows (each entry = 2 lines: title + meta)
            // A group header counts as 1 line.
            // We compute rows first, then scroll.
            struct HistRow<'a> {
                is_header: bool,
                entry_idx: Option<usize>, // index into filtered
                label: String,
                entry: Option<&'a crate::history::SessionEntry>,
                is_meta: bool, // second display line of an entry
            }

            let mut rows: Vec<HistRow> = Vec::new();
            let mut last_wt: Option<String> = None;

            for (i, e) in filtered.iter().enumerate() {
                if last_wt.as_deref() != Some(e.worktree_label.as_str()) {
                    rows.push(HistRow {
                        is_header: true,
                        entry_idx: None,
                        label: format!("\u{250c} {}", e.worktree_label),
                        entry: None,
                        is_meta: false,
                    });
                    last_wt = Some(e.worktree_label.clone());
                }
                // Title line
                rows.push(HistRow {
                    is_header: false,
                    entry_idx: Some(i),
                    label: format!("  {}", e.title),
                    entry: Some(e),
                    is_meta: false,
                });
                // Meta line
                let meta = format!(
                    "    {} \u{00b7} {} \u{00b7} {} msgs",
                    relative_time(e.last_activity, now),
                    e.branch.as_deref().unwrap_or("-"),
                    e.msg_count
                );
                rows.push(HistRow {
                    is_header: false,
                    entry_idx: Some(i),
                    label: meta,
                    entry: Some(e),
                    is_meta: true,
                });
            }

            let list_h = h.saturating_sub(2); // filter line + hint line
            let cursor = *cursor;

            // Find the first display row that belongs to cursor entry to centre scroll
            let cursor_row = rows.iter().position(|r| r.entry_idx == Some(cursor) && !r.is_meta)
                .unwrap_or(0);
            let scroll = if cursor_row >= list_h { cursor_row + 1 - list_h } else { 0 };

            let mut list_lines: Vec<Line> = Vec::new();
            for row in rows.iter().skip(scroll).take(list_h) {
                if row.is_header {
                    list_lines.push(Line::from(Span::styled(
                        row.label.clone(),
                        Style::default().fg(dim_clr).bg(bg),
                    )));
                    continue;
                }
                let e = match row.entry { Some(e) => e, None => continue };
                let is_selected = row.entry_idx == Some(cursor);
                let is_dead = !e.worktree_alive;

                let line_bg = if is_selected { sel_bg } else { bg };

                if row.is_meta {
                    let meta_style = if is_dead {
                        Style::default().fg(dead_clr).bg(line_bg)
                    } else if is_selected {
                        Style::default().fg(Color::Rgb(160, 175, 200)).bg(line_bg)
                    } else {
                        Style::default().fg(meta_clr).bg(line_bg)
                    };
                    // Extract branch and render with colour
                    let parts: Vec<&str> = row.label.trim().splitn(3, " \u{00b7} ").collect();
                    let time_str = parts.first().copied().unwrap_or("");
                    let branch_str = parts.get(1).copied().unwrap_or("");
                    let msgs_str = parts.get(2).copied().unwrap_or("");
                    list_lines.push(Line::from(vec![
                        Span::styled("    ", Style::default().bg(line_bg)),
                        Span::styled(time_str.to_string(), meta_style),
                        Span::styled(" \u{00b7} ", Style::default().fg(dim_clr).bg(line_bg)),
                        Span::styled(
                            branch_str.to_string(),
                            if is_dead { meta_style } else { Style::default().fg(branch_clr).bg(line_bg) },
                        ),
                        Span::styled(" \u{00b7} ", Style::default().fg(dim_clr).bg(line_bg)),
                        Span::styled(msgs_str.to_string(), meta_style),
                    ]));
                } else {
                    let title_style = if is_dead {
                        Style::default().fg(dead_clr).bg(line_bg)
                    } else if is_selected {
                        Style::default().fg(Color::White).bg(line_bg).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::Rgb(200, 210, 230)).bg(line_bg)
                    };
                    list_lines.push(Line::from(Span::styled(row.label.clone(), title_style)));
                }
            }

            if filtered.is_empty() {
                list_lines.push(Line::from(Span::styled(
                    " No Claude history for this repo.",
                    Style::default().fg(dim_clr).bg(bg),
                )));
            }

            // Pad remaining rows
            while list_lines.len() < list_h {
                list_lines.push(Line::from(Span::styled("", Style::default().bg(bg))));
            }

            // Hint row
            let hint_line = Line::from(vec![
                Span::styled(" Enter", Style::default().fg(border_clr).bg(bg)),
                Span::styled(":preview  ", Style::default().fg(dim_clr).bg(bg)),
                Span::styled("^r", Style::default().fg(border_clr).bg(bg)),
                Span::styled(":resume  ", Style::default().fg(dim_clr).bg(bg)),
                Span::styled("Esc", Style::default().fg(border_clr).bg(bg)),
                Span::styled(":back", Style::default().fg(dim_clr).bg(bg)),
            ]);

            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(1),
                    Constraint::Min(1),
                    Constraint::Length(1),
                ])
                .split(inner);

            frame.render_widget(Paragraph::new(filter_line).style(Style::default().bg(bg)), chunks[0]);
            frame.render_widget(
                Paragraph::new(list_lines).style(Style::default().bg(bg)),
                chunks[1],
            );
            frame.render_widget(Paragraph::new(hint_line).style(Style::default().bg(bg)), chunks[2]);
        }
        _ => {}
    }
}
