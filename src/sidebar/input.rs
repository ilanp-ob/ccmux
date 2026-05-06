use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use super::{App, Mode};
use super::mode::{WorktreeStep, FolderPickStep};

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
        Mode::Rename { .. } => handle_rename(app, key),
        Mode::NewWindow { .. } => handle_new_window(app, key),
        Mode::WorktreeFlow(_) => handle_worktree(app, key),
        Mode::ActionMenu { .. } => handle_action_menu(app, key),
        Mode::FolderPick(_) => handle_folder_pick(app, key),
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
        KeyCode::Char('r') => {
            const ICON: &str = "\u{1F916}"; // 🤖
            let prefill = app.selected_pane()
                .map(|p| {
                    if let Some(rest) = p.window_name.strip_prefix("cc:") {
                        format!("{} {}", ICON, rest)
                    } else {
                        p.window_name.clone()
                    }
                })
                .unwrap_or_default();
            app.mode = Mode::Rename { text: prefill };
        }
        KeyCode::Char('c') => {
            app.start_folder_pick();
        }
        KeyCode::Char('w') => {
            app.start_worktree_flow();
        }
        KeyCode::Char('o') => {
            let houston = std::path::PathBuf::from(
                std::env::var("HOME").unwrap_or_default()
            ).join("dev").join("houston");
            app.start_worktree_flow_for_path(houston);
        }
        KeyCode::Char('l') => {
            let items = app.action_items_for_selected();
            if !items.is_empty() {
                app.mode = Mode::ActionMenu { items, cursor: 0 };
            }
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
                    ConfirmKind::DeleteWorktree { worktree_path, .. } => {
                        if let Err(e) = std::fs::remove_dir_all(&worktree_path) {
                            app.error = Some(format!("Failed to delete worktree {}: {}", worktree_path, e));
                        } else {
                            app.message = Some("Worktree deleted".into());
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

fn handle_rename(app: &mut App, key: KeyEvent) {
    let text = match &app.mode {
        Mode::Rename { text } => text.clone(),
        _ => return,
    };
    match key.code {
        KeyCode::Esc => {
            app.mode = Mode::Normal;
        }
        KeyCode::Tab => {
            const PREFIX: &str = "\u{1F916} "; // 🤖
            let new_text = if text.starts_with(PREFIX) {
                text[PREFIX.len()..].to_string()
            } else {
                format!("{}{}", PREFIX, text)
            };
            app.mode = Mode::Rename { text: new_text };
        }
        KeyCode::Enter => {
            let t = text.clone();
            app.execute_rename(&t);
            // execute_rename sets mode to Normal already, but set it again for safety
            app.mode = Mode::Normal;
        }
        KeyCode::Backspace => {
            let mut t = text;
            t.pop();
            app.mode = Mode::Rename { text: t };
        }
        KeyCode::Char(c) => {
            let mut t = text;
            t.push(c);
            app.mode = Mode::Rename { text: t };
        }
        _ => {}
    }
}

fn handle_new_window(app: &mut App, key: KeyEvent) {
    use crate::config::WINDOW_COLORS;

    // Extract current state
    let (name, color_idx, launch_claude, field) = match &app.mode {
        Mode::NewWindow { name, color_idx, launch_claude, field } => {
            (name.clone(), *color_idx, *launch_claude, *field)
        }
        _ => return,
    };

    match key.code {
        KeyCode::Esc => {
            app.mode = Mode::Normal;
        }
        KeyCode::Enter => {
            app.execute_new_window(&name, color_idx, launch_claude);
        }
        KeyCode::Tab => {
            // Advance field: 0→1→2→0
            let new_field = (field + 1) % 3;
            app.mode = Mode::NewWindow { name, color_idx, launch_claude, field: new_field };
        }
        KeyCode::BackTab => {
            // Retreat field: 0→2→1→0
            let new_field = if field == 0 { 2 } else { field - 1 };
            app.mode = Mode::NewWindow { name, color_idx, launch_claude, field: new_field };
        }
        KeyCode::Backspace if field == 0 => {
            let mut n = name;
            n.pop();
            app.mode = Mode::NewWindow { name: n, color_idx, launch_claude, field };
        }
        KeyCode::Char(c) if field == 0 => {
            let mut n = name;
            n.push(c);
            app.mode = Mode::NewWindow { name: n, color_idx, launch_claude, field };
        }
        KeyCode::Left if field == 1 => {
            let new_idx = if color_idx == 0 { WINDOW_COLORS.len() - 1 } else { color_idx - 1 };
            app.mode = Mode::NewWindow { name, color_idx: new_idx, launch_claude, field };
        }
        KeyCode::Right if field == 1 => {
            let new_idx = (color_idx + 1) % WINDOW_COLORS.len();
            app.mode = Mode::NewWindow { name, color_idx: new_idx, launch_claude, field };
        }
        KeyCode::Char(' ') if field == 2 => {
            app.mode = Mode::NewWindow { name, color_idx, launch_claude: !launch_claude, field };
        }
        _ => {}
    }
}

fn handle_action_menu(app: &mut App, key: KeyEvent) {
    let (items, cursor) = match &app.mode {
        Mode::ActionMenu { items, cursor } => (items.clone(), *cursor),
        _ => return,
    };

    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            let new_cursor = (cursor + 1) % items.len();
            app.mode = Mode::ActionMenu { items, cursor: new_cursor };
        }
        KeyCode::Char('k') | KeyCode::Up => {
            let new_cursor = if cursor == 0 { items.len() - 1 } else { cursor - 1 };
            app.mode = Mode::ActionMenu { items, cursor: new_cursor };
        }
        KeyCode::Enter => {
            let item = items[cursor].clone();
            app.send_action(item);
            // send_action handles mode transition internally
        }
        KeyCode::Esc | KeyCode::Char('q') => {
            app.mode = Mode::Normal;
        }
        _ => {}
    }
}

fn handle_worktree(app: &mut App, key: KeyEvent) {
    let step = match &app.mode {
        Mode::WorktreeFlow(step) => step.clone(),
        _ => return,
    };

    match step {
        WorktreeStep::Fetching => {
            if key.code == KeyCode::Esc {
                app.mode = Mode::Normal;
                app.fetch_handle = None;
                app.fetch_repo_root = None;
            }
        }

        WorktreeStep::BranchSelect {
            repo_root, branches, filter, cursor, entering_new, new_branch_text,
        } => {
            handle_worktree_branch_select(
                app, key,
                repo_root, branches, filter, cursor, entering_new, new_branch_text,
            );
        }

        WorktreeStep::FolderName { repo_root, branch, folder } => {
            handle_worktree_folder_name(app, key, repo_root, branch, folder);
        }

        WorktreeStep::Options { repo_root, branch, folder, opts, existing_wt_path } => {
            handle_worktree_options(app, key, repo_root, branch, folder, opts, existing_wt_path);
        }

        WorktreeStep::Executing { .. } => {
            if key.code == KeyCode::Esc {
                app.mode = Mode::Normal;
            }
        }
    }
}

fn handle_worktree_branch_select(
    app: &mut App,
    key: KeyEvent,
    repo_root: String,
    branches: Vec<crate::git::BranchEntry>,
    filter: String,
    cursor: usize,
    entering_new: bool,
    new_branch_text: String,
) {
    let filtered: Vec<&crate::git::BranchEntry> = branches.iter()
        .filter(|b| b.name.to_lowercase().contains(&filter.to_lowercase()))
        .collect();
    let filtered_len = filtered.len();

    match key.code {
        KeyCode::Esc => {
            app.mode = Mode::Normal;
        }

        // F or Tab toggles entering_new
        KeyCode::Char('F') | KeyCode::Tab => {
            app.mode = Mode::WorktreeFlow(WorktreeStep::BranchSelect {
                repo_root,
                branches,
                filter,
                cursor,
                entering_new: !entering_new,
                new_branch_text,
            });
        }

        KeyCode::Up | KeyCode::Char('k') => {
            let new_cursor = if cursor == 0 {
                filtered_len.saturating_sub(1)
            } else {
                cursor - 1
            };
            app.mode = Mode::WorktreeFlow(WorktreeStep::BranchSelect {
                repo_root, branches, filter, cursor: new_cursor, entering_new, new_branch_text,
            });
        }

        KeyCode::Down | KeyCode::Char('j') => {
            let new_cursor = if filtered_len == 0 {
                0
            } else {
                (cursor + 1) % filtered_len
            };
            app.mode = Mode::WorktreeFlow(WorktreeStep::BranchSelect {
                repo_root, branches, filter, cursor: new_cursor, entering_new, new_branch_text,
            });
        }

        KeyCode::Backspace => {
            if entering_new {
                let mut t = new_branch_text;
                t.pop();
                app.mode = Mode::WorktreeFlow(WorktreeStep::BranchSelect {
                    repo_root, branches, filter, cursor, entering_new, new_branch_text: t,
                });
            } else {
                let mut f = filter;
                f.pop();
                app.mode = Mode::WorktreeFlow(WorktreeStep::BranchSelect {
                    repo_root, branches, filter: f, cursor, entering_new, new_branch_text,
                });
            }
        }

        KeyCode::Char(c) => {
            if entering_new {
                let mut t = new_branch_text;
                t.push(c);
                app.mode = Mode::WorktreeFlow(WorktreeStep::BranchSelect {
                    repo_root, branches, filter, cursor, entering_new, new_branch_text: t,
                });
            } else {
                let mut f = filter;
                f.push(c);
                // Re-filter and clamp cursor
                let new_filtered_len = branches.iter()
                    .filter(|b| b.name.to_lowercase().contains(&f.to_lowercase()))
                    .count();
                let new_cursor = if new_filtered_len == 0 { 0 } else { cursor.min(new_filtered_len - 1) };
                app.mode = Mode::WorktreeFlow(WorktreeStep::BranchSelect {
                    repo_root, branches, filter: f, cursor: new_cursor, entering_new, new_branch_text,
                });
            }
        }

        KeyCode::Enter => {
            let (branch, existing_wt) = if entering_new {
                if new_branch_text.is_empty() { return; }
                (new_branch_text.clone(), None)
            } else {
                if filtered_len == 0 { return; }
                let entry = filtered[cursor.min(filtered_len - 1)];
                (entry.name.clone(), entry.worktree_path.clone())
            };

            let repo_path = std::path::PathBuf::from(&repo_root);

            if let Some(wt_path) = existing_wt {
                // Worktree exists — skip FolderName but still show Options so the
                // user can choose to launch Claude, open VS Code, etc.
                let folder = std::path::Path::new(&wt_path)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| branch.clone());
                app.mode = Mode::WorktreeFlow(WorktreeStep::Options {
                    repo_root,
                    branch,
                    folder,
                    opts: crate::sidebar::mode::WorktreeOpts::default(),
                    existing_wt_path: Some(wt_path),
                });
                return;
            }

            let folder_name = crate::git::branch_to_folder(&repo_path, &branch);
            let parent = repo_path.parent().unwrap_or(&repo_path).to_path_buf();
            let folder = parent.join(&folder_name).to_string_lossy().into_owned();
            app.mode = Mode::WorktreeFlow(WorktreeStep::FolderName {
                repo_root,
                branch,
                folder,
            });
        }

        _ => {}
    }
}

fn handle_worktree_folder_name(
    app: &mut App,
    key: KeyEvent,
    repo_root: String,
    branch: String,
    folder: String,
) {
    match key.code {
        KeyCode::Esc => {
            // Go back to Normal; branches will be re-fetched on next 'w'
            app.mode = Mode::Normal;
        }
        KeyCode::Backspace => {
            let mut f = folder;
            f.pop();
            app.mode = Mode::WorktreeFlow(WorktreeStep::FolderName {
                repo_root, branch, folder: f,
            });
        }
        KeyCode::Char(c) => {
            let mut f = folder;
            f.push(c);
            app.mode = Mode::WorktreeFlow(WorktreeStep::FolderName {
                repo_root, branch, folder: f,
            });
        }
        KeyCode::Enter => {
            app.mode = Mode::WorktreeFlow(WorktreeStep::Options {
                repo_root,
                branch,
                folder,
                opts: crate::sidebar::mode::WorktreeOpts::default(),
                existing_wt_path: None,
            });
        }
        _ => {}
    }
}

fn handle_worktree_options(
    app: &mut App,
    key: KeyEvent,
    repo_root: String,
    branch: String,
    folder: String,
    mut opts: crate::sidebar::mode::WorktreeOpts,
    existing_wt_path: Option<String>,
) {
    use crate::config::{AVAILABLE_MODELS, AVAILABLE_EFFORTS, WINDOW_COLORS};

    macro_rules! back {
        () => { Mode::WorktreeFlow(WorktreeStep::Options {
            repo_root: repo_root.clone(), branch: branch.clone(),
            folder: folder.clone(), opts: opts.clone(),
            existing_wt_path: existing_wt_path.clone(),
        }) };
    }

    match key.code {
        KeyCode::Esc => { app.mode = Mode::Normal; }
        KeyCode::Enter => {
            app.execute_worktree(&repo_root, &branch, &folder, &opts, existing_wt_path.as_deref());
        }
        KeyCode::Tab => {
            opts.field = (opts.field + 1) % 5;
            app.mode = back!();
        }
        KeyCode::BackTab => {
            opts.field = if opts.field == 0 { 4 } else { opts.field - 1 };
            app.mode = back!();
        }
        KeyCode::Left => {
            match opts.field {
                0 => opts.model_idx = if opts.model_idx == 0 { AVAILABLE_MODELS.len() - 1 } else { opts.model_idx - 1 },
                1 => opts.effort_idx = if opts.effort_idx == 0 { AVAILABLE_EFFORTS.len() - 1 } else { opts.effort_idx - 1 },
                3 => opts.color_idx = if opts.color_idx == 0 { WINDOW_COLORS.len() - 1 } else { opts.color_idx - 1 },
                _ => {}
            }
            app.mode = back!();
        }
        KeyCode::Right => {
            match opts.field {
                0 => opts.model_idx = (opts.model_idx + 1) % AVAILABLE_MODELS.len(),
                1 => opts.effort_idx = (opts.effort_idx + 1) % AVAILABLE_EFFORTS.len(),
                3 => opts.color_idx = (opts.color_idx + 1) % WINDOW_COLORS.len(),
                _ => {}
            }
            app.mode = back!();
        }
        KeyCode::Char(' ') => {
            match opts.field {
                2 => opts.launch_claude = !opts.launch_claude,
                4 => opts.open_vscode = !opts.open_vscode,
                _ => {}
            }
            app.mode = back!();
        }
        _ => {}
    }
}

fn handle_folder_pick(app: &mut App, key: KeyEvent) {
    use std::path::PathBuf;

    let step = match &app.mode {
        Mode::FolderPick(s) => s.clone(),
        _ => return,
    };

    match step {
        FolderPickStep::Scanning => {
            if key.code == KeyCode::Esc {
                app.mode = Mode::Normal;
                app.folder_scan_handle = None;
                app.folder_scan_root = None;
            }
        }
        FolderPickStep::Options { path, is_new, mut opts } => {
            use crate::config::{AVAILABLE_MODELS, AVAILABLE_EFFORTS, WINDOW_COLORS};
            macro_rules! back {
                () => { Mode::FolderPick(FolderPickStep::Options {
                    path: path.clone(), is_new, opts: opts.clone(),
                }) };
            }
            match key.code {
                KeyCode::Esc => { app.mode = Mode::Normal; }
                KeyCode::Enter => { app.execute_folder_pick(path, is_new, &opts); }
                KeyCode::Tab => {
                    opts.field = (opts.field + 1) % 5;
                    app.mode = back!();
                }
                KeyCode::BackTab => {
                    opts.field = if opts.field == 0 { 4 } else { opts.field - 1 };
                    app.mode = back!();
                }
                KeyCode::Left => {
                    match opts.field {
                        0 => opts.model_idx = if opts.model_idx == 0 { AVAILABLE_MODELS.len() - 1 } else { opts.model_idx - 1 },
                        1 => opts.effort_idx = if opts.effort_idx == 0 { AVAILABLE_EFFORTS.len() - 1 } else { opts.effort_idx - 1 },
                        3 => opts.color_idx = if opts.color_idx == 0 { WINDOW_COLORS.len() - 1 } else { opts.color_idx - 1 },
                        _ => {}
                    }
                    app.mode = back!();
                }
                KeyCode::Right => {
                    match opts.field {
                        0 => opts.model_idx = (opts.model_idx + 1) % AVAILABLE_MODELS.len(),
                        1 => opts.effort_idx = (opts.effort_idx + 1) % AVAILABLE_EFFORTS.len(),
                        3 => opts.color_idx = (opts.color_idx + 1) % WINDOW_COLORS.len(),
                        _ => {}
                    }
                    app.mode = back!();
                }
                KeyCode::Char(' ') => {
                    match opts.field {
                        2 => opts.launch_claude = !opts.launch_claude,
                        4 => opts.open_vscode = !opts.open_vscode,
                        _ => {}
                    }
                    app.mode = back!();
                }
                _ => {}
            }
        }
        FolderPickStep::Picking { root, dirs, filter, cursor } => {
            let filtered: Vec<&PathBuf> = dirs.iter()
                .filter(|d| d.file_name()
                    .map(|n| n.to_string_lossy().to_lowercase().contains(&filter.to_lowercase()))
                    .unwrap_or(false))
                .collect();
            let filtered_len = filtered.len();
            let clamped = if filtered_len == 0 { 0 } else { cursor.min(filtered_len - 1) };

            match key.code {
                KeyCode::Esc => { app.mode = Mode::Normal; }

                KeyCode::Up | KeyCode::Char('k') => {
                    let new_cursor = if cursor == 0 { filtered_len.saturating_sub(1) } else { cursor - 1 };
                    app.mode = Mode::FolderPick(FolderPickStep::Picking { root, dirs, filter, cursor: new_cursor });
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    let new_cursor = if filtered_len == 0 { 0 } else { (cursor + 1) % filtered_len };
                    app.mode = Mode::FolderPick(FolderPickStep::Picking { root, dirs, filter, cursor: new_cursor });
                }
                KeyCode::Enter => {
                    if let Some(path) = filtered.get(clamped) {
                        app.mode = Mode::FolderPick(FolderPickStep::Options {
                            path: (*path).clone(),
                            is_new: false,
                            opts: crate::sidebar::mode::WorktreeOpts::default(),
                        });
                    } else if !filter.is_empty() {
                        app.mode = Mode::FolderPick(FolderPickStep::Options {
                            path: root.join(&filter),
                            is_new: true,
                            opts: crate::sidebar::mode::WorktreeOpts::default(),
                        });
                    }
                }
                KeyCode::Right if !filtered.is_empty() => {
                    if let Some(path) = filtered.get(clamped) {
                        app.navigate_folder_into((*path).clone());
                    }
                }
                KeyCode::Left => {
                    app.navigate_folder_up();
                }
                KeyCode::Backspace if filter.is_empty() => {
                    app.navigate_folder_up();
                }
                KeyCode::Backspace => {
                    let mut f = filter;
                    f.pop();
                    let new_len = dirs.iter()
                        .filter(|d| d.file_name()
                            .map(|n| n.to_string_lossy().to_lowercase().contains(&f.to_lowercase()))
                            .unwrap_or(false))
                        .count();
                    let new_cursor = if new_len == 0 { 0 } else { cursor.min(new_len - 1) };
                    app.mode = Mode::FolderPick(FolderPickStep::Picking { root, dirs, filter: f, cursor: new_cursor });
                }
                KeyCode::Char(c) => {
                    let mut f = filter;
                    f.push(c);
                    let new_len = dirs.iter()
                        .filter(|d| d.file_name()
                            .map(|n| n.to_string_lossy().to_lowercase().contains(&f.to_lowercase()))
                            .unwrap_or(false))
                        .count();
                    let new_cursor = if new_len == 0 { 0 } else { cursor.min(new_len.saturating_sub(1)) };
                    app.mode = Mode::FolderPick(FolderPickStep::Picking { root, dirs, filter: f, cursor: new_cursor });
                }
                _ => {}
            }
        }
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
