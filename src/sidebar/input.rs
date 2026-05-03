use crossterm::event::{KeyCode, KeyEvent};
use super::{App, Mode};

pub fn handle_key(app: &mut App, key: KeyEvent) {
    app.clear_messages();

    match &app.mode {
        Mode::Normal => handle_normal(app, key),
        Mode::ActionHints => handle_normal(app, key), // same keys, expanded row shows hints
        Mode::Confirm(_) => handle_confirm(app, key),
        Mode::Help => handle_help(app, key),
    }
}

fn handle_normal(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,
        KeyCode::Char('j') | KeyCode::Down => app.select_next(),
        KeyCode::Char('k') | KeyCode::Up => app.select_prev(),
        KeyCode::Enter => {
            app.focus_selected();
            // Only quit if switching to a different window
            let own = app.own_window_id.as_deref().unwrap_or("");
            if let Some(pane) = app.selected_pane() {
                if pane.window_id != own {
                    app.should_quit = true;
                }
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
        // 1-9 jump to display number
        KeyCode::Char(c) if c.is_ascii_digit() && c != '0' => {
            let n = c as usize - '0' as usize;
            app.select_by_display_num(n);
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
