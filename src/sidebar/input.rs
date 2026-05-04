use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use super::{App, Mode};

pub fn handle_key(app: &mut App, key: KeyEvent) {
    // Don't clear messages while composing — let the user see what they're doing
    if !matches!(app.mode, Mode::Compose { .. }) {
        app.clear_messages();
    }

    match &app.mode {
        Mode::Normal => handle_normal(app, key),
        Mode::ActionHints => handle_normal(app, key),
        Mode::Confirm(_) => handle_confirm(app, key),
        Mode::Help => handle_help(app, key),
        Mode::Compose { .. } => handle_compose(app, key),
    }
}

fn handle_normal(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,
        KeyCode::Char('j') | KeyCode::Down => app.select_next(),
        KeyCode::Char('k') | KeyCode::Up => app.select_prev(),
        KeyCode::Enter => {
            let cur = app.selected;
            let is_cross_window = app.selected_pane()
                .map(|p| Some(&p.window_id) != app.own_window_id.as_ref())
                .unwrap_or(false);
            if is_cross_window {
                // Cross-window: single step — sidebar loses keyboard focus after select_window,
                // so we must open the sidebar and focus the Claude pane in one shot.
                app.focus_selected();
                app.last_entered_idx = None;
            } else if app.last_entered_idx == Some(cur) {
                // Same window, second Enter → focus the Claude pane.
                app.focus_selected();
                app.last_entered_idx = None;
            } else {
                // Same window, first Enter → mark as previewed (sidebar retains keyboard focus).
                app.last_entered_idx = Some(cur);
            }
        }
        KeyCode::Char('?') => app.mode = Mode::Help,
        KeyCode::Char('K') => {
            if let Some(pane) = app.selected_pane() {
                app.mode = Mode::Confirm(super::mode::ConfirmKind::KillWindow {
                    window_id: pane.window_id.clone(),
                    window_name: pane.window_name.clone(),
                });
            }
        }
        // 0-9: jump to session by display number (0 = session 10); press again to focus
        KeyCode::Char(c) if c.is_ascii_digit() => {
            let n = if c == '0' { 10 } else { c as usize - '0' as usize };
            if app.selected_pane().map(|p| p.display_num == n).unwrap_or(false) {
                app.focus_selected();
            } else {
                app.select_by_display_num(n);
            }
        }
        KeyCode::Char('s') => app.toggle_sticky(),
        KeyCode::Char('i') => {
            app.mode = Mode::Compose { text: String::new() };
        }
        // r (rename), n (new window), w (new worktree) — stubbed, Plan 2
        KeyCode::Char('r') | KeyCode::Char('n') | KeyCode::Char('w') => {
            app.message = Some("Not yet implemented (Plan 2)".into());
        }
        _ => {}
    }
}

fn handle_confirm(app: &mut App, key: KeyEvent) {
    use super::mode::ConfirmKind;
    match key.code {
        KeyCode::Char('y') | KeyCode::Enter => {
            if let Mode::Confirm(kind) = app.mode.clone() {
                match kind {
                    ConfirmKind::KillWindow { window_id, .. } => {
                        let tmux = crate::tmux::Tmux::new(app.managed_server.clone());
                        match tmux.kill_window(&window_id) {
                            Ok(_) => app.message = Some("Window killed".into()),
                            Err(e) => app.error = Some(format!("Kill failed: {}", e)),
                        }
                    }
                    ConfirmKind::KillAndDeleteWorktree { window_id, path } => {
                        let tmux = crate::tmux::Tmux::new(app.managed_server.clone());
                        let _ = tmux.kill_window(&window_id);
                        if let Err(e) = std::fs::remove_dir_all(&path) {
                            app.error = Some(format!("Killed window but failed to delete {}: {}", path, e));
                        } else {
                            app.message = Some("Window killed and worktree deleted".into());
                        }
                    }
                }
                app.mode = Mode::Normal;
                let _ = app.refresh();
            }
        }
        KeyCode::Char('n') | KeyCode::Esc => {
            app.mode = Mode::Normal;
        }
        _ => {}
    }
}

fn handle_help(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('?') | KeyCode::Esc | KeyCode::Char('q') => {
            app.mode = Mode::Normal;
        }
        _ => {}
    }
}

fn handle_compose(app: &mut App, key: KeyEvent) {
    let text = match &app.mode {
        Mode::Compose { text } => text.clone(),
        _ => return,
    };
    match key.code {
        KeyCode::Esc => {
            app.mode = Mode::Normal;
        }
        KeyCode::Enter => {
            let t = text.clone();
            app.send_message(&t);
            app.mode = Mode::Normal;
        }
        KeyCode::Backspace => {
            let mut t = text;
            t.pop();
            app.mode = Mode::Compose { text: t };
        }
        KeyCode::Char(c) => {
            let mut t = text;
            t.push(c);
            app.mode = Mode::Compose { text: t };
        }
        _ => {}
    }
}

pub fn handle_mouse(app: &mut App, mouse: MouseEvent) {
    if mouse.kind != MouseEventKind::Down(MouseButton::Left) {
        return;
    }
    let row = mouse.row;
    // Each item occupies 2 rows; hit if click falls on row 0 or row 1 of the item.
    let hit = app.pane_click_rows.iter()
        .find(|(r, _)| row >= *r && row < r + 2)
        .map(|(_, idx)| *idx);
    if let Some(idx) = hit {
        if app.selected == idx {
            app.focus_selected();
        } else {
            app.selected = idx;
        }
    }
}
