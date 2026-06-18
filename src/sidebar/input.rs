use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use super::{App, Mode};
use super::mode::{WorktreeStep, FolderPickStep};
use crate::config::WINDOW_COLORS;

/// Returns `Some(score)` if `needle` appears as a subsequence of `haystack` (case-insensitive).
/// Higher score = better match (consecutive runs, word boundaries).
fn fuzzy_score(haystack: &str, needle: &str) -> Option<i32> {
    if needle.is_empty() { return Some(0); }
    let h: Vec<char> = haystack.to_lowercase().chars().collect();
    let n: Vec<char> = needle.to_lowercase().chars().collect();
    let mut hi = 0;
    let mut ni = 0;
    let mut score = 0i32;
    let mut last_match: Option<usize> = None;
    while hi < h.len() && ni < n.len() {
        if h[hi] == n[ni] {
            if last_match.is_some_and(|lm| lm + 1 == hi) { score += 10; }
            if hi == 0 || matches!(h[hi - 1], '/' | '-' | '_' | ' ' | '.') { score += 5; }
            last_match = Some(hi);
            ni += 1;
        }
        hi += 1;
    }
    if ni == n.len() { Some(score) } else { None }
}

/// Filter and sort `items` by fuzzy score against `filter`. When `filter` is empty,
/// all items are returned in original order. Key extracts the string to match against.
pub(crate) fn fuzzy_sort<'a, T>(items: &'a [T], filter: &str, key: impl Fn(&T) -> String) -> Vec<&'a T> {
    let mut scored: Vec<(i32, &T)> = items.iter()
        .filter_map(|item| fuzzy_score(&key(item), filter).map(|s| (s, item)))
        .collect();
    if !filter.is_empty() {
        scored.sort_by(|a, b| b.0.cmp(&a.0));
    }
    scored.into_iter().map(|(_, item)| item).collect()
}

/// Handle a text-editing key for a field with an in-text cursor.
/// Returns `Some((new_text, new_cursor))` if the key was consumed, `None` otherwise.
/// Cursor is char-indexed. Does NOT handle Esc / Enter / Tab / mode-specific keys.
fn apply_text_key(text: &str, cursor: usize, key: &KeyEvent) -> Option<(String, usize)> {
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let ctrl  = key.modifiers.contains(KeyModifiers::CONTROL);
    let plain = !ctrl && !key.modifiers.contains(KeyModifiers::ALT);

    match key.code {
        KeyCode::Left  if plain => Some((text.to_string(), cursor.saturating_sub(1))),
        KeyCode::Right if plain => Some((text.to_string(), (cursor + 1).min(len))),
        KeyCode::Home            => Some((text.to_string(), 0)),
        KeyCode::End             => Some((text.to_string(), len)),
        KeyCode::Char('a') if ctrl => Some((text.to_string(), 0)),
        KeyCode::Char('e') if ctrl => Some((text.to_string(), len)),
        KeyCode::Char('k') if ctrl => {
            Some((chars[..cursor].iter().collect(), cursor))
        }
        KeyCode::Char('u') if ctrl => {
            Some((chars[cursor..].iter().collect(), 0))
        }
        KeyCode::Char('w') if ctrl => {
            // Delete word before cursor (like readline)
            let trim = chars[..cursor].iter().rposition(|c| !c.is_whitespace()).map(|p| p + 1).unwrap_or(0);
            let word_start = if trim > 0 {
                chars[..trim].iter().rposition(|c| c.is_whitespace()).map(|p| p + 1).unwrap_or(0)
            } else { 0 };
            let mut new: Vec<char> = chars[..word_start].to_vec();
            new.extend_from_slice(&chars[cursor..]);
            Some((new.iter().collect(), word_start))
        }
        KeyCode::Delete if plain => {
            if cursor < len {
                let mut new = chars.clone();
                new.remove(cursor);
                Some((new.iter().collect(), cursor))
            } else {
                Some((text.to_string(), cursor))
            }
        }
        KeyCode::Backspace => {
            if cursor > 0 {
                let mut new = chars.clone();
                new.remove(cursor - 1);
                Some((new.iter().collect(), cursor - 1))
            } else {
                Some((text.to_string(), cursor))
            }
        }
        KeyCode::Char(c) if plain => {
            let mut new = chars.clone();
            new.insert(cursor, c);
            Some((new.iter().collect(), cursor + 1))
        }
        _ => None,
    }
}

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
        Mode::NewWindow { .. } => handle_new_window(app, key),
        Mode::EditWindow { .. } => handle_edit_window(app, key),
        Mode::WorktreeFlow(_) => handle_worktree(app, key),
        Mode::ActionMenu { .. } => handle_action_menu(app, key),
        Mode::FolderPick(_) => handle_folder_pick(app, key),
        Mode::History(_) => handle_history(app, key),
    }
}

fn handle_normal(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,
        KeyCode::Char('j') | KeyCode::Down => app.select_next(),
        KeyCode::Char('k') | KeyCode::Up => app.select_prev(),
        KeyCode::Enter => {
            // Daemon job selected → open it in a new tmux window
            if app.selected_job().is_some() {
                app.resume_selected_job();
                return;
            }
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
        KeyCode::Char('r') => {
            if app.selected_job().is_some() {
                app.resume_selected_job();
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
            let cur_pane_num = app.selected_pane().map(|p| p.display_num);
            let cur_job_num  = app.selected_job().map(|j| j.display_num);
            if cur_pane_num == Some(n) {
                app.focus_selected();
            } else if cur_job_num == Some(n) {
                app.resume_selected_job();
            } else {
                app.select_by_display_num(n);
            }
        }
        KeyCode::Char('s') => app.toggle_sticky(),
        KeyCode::Char('i') => {
            let choices = if let Some(job) = app.selected_job() {
                let src = job.needs.as_deref().unwrap_or(&job.detail);
                parse_choices(src)
            } else if let Some(pane) = app.selected_pane() {
                app.pane_content_cache.get(&pane.pane_id)
                    .and_then(|c| parse_choices_from_pane(c))
            } else {
                None
            };

            if let Some(items) = choices {
                app.mode = Mode::ActionMenu { items, cursor: 0 };
            } else {
                app.mode = Mode::Compose { text: String::new(), cursor: 0 };
            }
        }
        KeyCode::Char('e') => {
            if let Some(pane) = app.selected_pane() {
                let window_id = pane.window_id.clone();
                let name = pane.window_name.clone();
                let name_cursor = name.chars().count();
                let color_idx = app.groups.iter()
                    .find(|g| g.window_id == window_id)
                    .and_then(|g| g.color_name.as_deref())
                    .and_then(|c| WINDOW_COLORS.iter().position(|(_, _, tc)| *tc == c))
                    .unwrap_or(0);
                app.mode = Mode::EditWindow { window_id, name, color_idx, field: 0, name_cursor };
            }
        }
        KeyCode::Char('c') => {
            app.start_folder_pick();
        }
        KeyCode::Char('h') => {
            app.start_history();
        }
        KeyCode::Char('g') => {
            app.open_git_popup();
        }
        KeyCode::Char('p') => {
            app.open_pr_popup();
        }
        KeyCode::Char('w') => {
            app.start_worktree_flow();
        }
        KeyCode::Char('o') => {
            let raw = &app.config.worktree.houston_path;
            let path = if let Some(rest) = raw.strip_prefix("~/") {
                std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(rest)
            } else if raw == "~" {
                std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default())
            } else {
                std::path::PathBuf::from(raw)
            };
            app.start_worktree_flow_for_path(path);
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
    let (text, cursor) = match &app.mode {
        Mode::Compose { text, cursor } => (text.clone(), *cursor),
        _ => return,
    };
    match key.code {
        KeyCode::Esc   => { app.mode = Mode::Normal; }
        KeyCode::Enter => {
            if app.selected_job().is_some() {
                app.reply_to_selected_job(&text);
            } else {
                app.send_message(&text);
            }
            app.mode = Mode::Normal;
        }
        _ => {
            if let Some((new_text, new_cursor)) = apply_text_key(&text, cursor, &key) {
                app.mode = Mode::Compose { text: new_text, cursor: new_cursor };
            }
        }
    }
}

fn handle_edit_window(app: &mut App, key: KeyEvent) {
    let (window_id, name, color_idx, field, name_cursor) = match &app.mode {
        Mode::EditWindow { window_id, name, color_idx, field, name_cursor } =>
            (window_id.clone(), name.clone(), *color_idx, *field, *name_cursor),
        _ => return,
    };

    macro_rules! set {
        ($name:expr, $nc:expr, $ci:expr, $f:expr) => {
            app.mode = Mode::EditWindow {
                window_id: window_id.clone(), name: $name,
                color_idx: $ci, field: $f, name_cursor: $nc,
            }
        };
    }

    match key.code {
        KeyCode::Esc   => { app.mode = Mode::Normal; }
        KeyCode::Enter => { app.execute_edit_window(&window_id, &name, color_idx); }
        KeyCode::Tab    => { set!(name, name_cursor, color_idx, (field + 1) % 2); }
        KeyCode::BackTab => { set!(name, name_cursor, color_idx, if field == 0 { 1 } else { 0 }); }
        // Ctrl+B toggles 🤖 prefix; adjust cursor to stay at same logical position
        KeyCode::Char('b') if field == 0 && key.modifiers.contains(KeyModifiers::CONTROL) => {
            const BOT: &str = "\u{1F916} ";
            const BOT_LEN: usize = 2; // 🤖 (1 char) + space (1 char)
            let (n, nc) = if name.starts_with(BOT) {
                let stripped = name[BOT.len()..].to_string();
                (stripped, name_cursor.saturating_sub(BOT_LEN))
            } else {
                (format!("{}{}", BOT, name), name_cursor + BOT_LEN)
            };
            set!(n, nc, color_idx, field);
        }
        // Color field navigation
        KeyCode::Left if field == 1 => {
            let new_idx = if color_idx == 0 { WINDOW_COLORS.len() - 1 } else { color_idx - 1 };
            set!(name, name_cursor, new_idx, field);
        }
        KeyCode::Right if field == 1 => {
            set!(name, name_cursor, (color_idx + 1) % WINDOW_COLORS.len(), field);
        }
        // Text editing on name field
        _ if field == 0 => {
            if let Some((new_name, new_nc)) = apply_text_key(&name, name_cursor, &key) {
                set!(new_name, new_nc, color_idx, field);
            }
        }
        _ => {}
    }
}

fn handle_new_window(app: &mut App, key: KeyEvent) {
    use crate::config::WINDOW_COLORS;

    let (name, color_idx, launch_claude, field, name_cursor) = match &app.mode {
        Mode::NewWindow { name, color_idx, launch_claude, field, name_cursor } =>
            (name.clone(), *color_idx, *launch_claude, *field, *name_cursor),
        _ => return,
    };

    macro_rules! set {
        ($name:expr, $nc:expr, $ci:expr, $lc:expr, $f:expr) => {
            app.mode = Mode::NewWindow {
                name: $name, color_idx: $ci,
                launch_claude: $lc, field: $f, name_cursor: $nc,
            }
        };
    }

    match key.code {
        KeyCode::Esc   => { app.mode = Mode::Normal; }
        KeyCode::Enter => { app.execute_new_window(&name, color_idx, launch_claude); }
        KeyCode::Tab    => { set!(name, name_cursor, color_idx, launch_claude, (field + 1) % 3); }
        KeyCode::BackTab => { set!(name, name_cursor, color_idx, launch_claude, if field == 0 { 2 } else { field - 1 }); }
        KeyCode::Left if field == 1 => {
            let new_idx = if color_idx == 0 { WINDOW_COLORS.len() - 1 } else { color_idx - 1 };
            set!(name, name_cursor, new_idx, launch_claude, field);
        }
        KeyCode::Right if field == 1 => {
            set!(name, name_cursor, (color_idx + 1) % WINDOW_COLORS.len(), launch_claude, field);
        }
        KeyCode::Char(' ') if field == 2 => {
            set!(name, name_cursor, color_idx, !launch_claude, field);
        }
        _ if field == 0 => {
            if let Some((new_name, new_nc)) = apply_text_key(&name, name_cursor, &key) {
                set!(new_name, new_nc, color_idx, launch_claude, field);
            }
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
            repo_root, branches, filter, filter_cursor, cursor, entering_new, new_branch_text, new_branch_cursor,
        } => {
            handle_worktree_branch_select(
                app, key,
                repo_root, branches, filter, filter_cursor, cursor, entering_new, new_branch_text, new_branch_cursor,
            );
        }

        WorktreeStep::FolderName { repo_root, branch, folder, cursor, base_branch } => {
            handle_worktree_folder_name(app, key, repo_root, branch, folder, cursor, base_branch);
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
    filter_cursor: usize,
    cursor: usize,
    entering_new: bool,
    new_branch_text: String,
    new_branch_cursor: usize,
) {
    let filtered = fuzzy_sort(&branches, &filter, |b| b.name.clone());
    let filtered_len = filtered.len();

    macro_rules! set {
        ($f:expr, $fc:expr, $c:expr, $en:expr, $nbt:expr, $nbc:expr) => {
            app.mode = Mode::WorktreeFlow(WorktreeStep::BranchSelect {
                repo_root: repo_root.clone(), branches: branches.clone(),
                filter: $f, filter_cursor: $fc, cursor: $c,
                entering_new: $en, new_branch_text: $nbt, new_branch_cursor: $nbc,
            })
        };
    }

    match key.code {
        KeyCode::Esc => { app.mode = Mode::Normal; }

        KeyCode::Char('F') | KeyCode::Tab => {
            set!(filter, filter_cursor, cursor, !entering_new, new_branch_text, new_branch_cursor);
        }

        KeyCode::Up | KeyCode::Char('k') if !entering_new => {
            let nc = if cursor == 0 { filtered_len.saturating_sub(1) } else { cursor - 1 };
            set!(filter, filter_cursor, nc, entering_new, new_branch_text, new_branch_cursor);
        }

        KeyCode::Down | KeyCode::Char('j') if !entering_new => {
            let nc = if filtered_len == 0 { 0 } else { (cursor + 1) % filtered_len };
            set!(filter, filter_cursor, nc, entering_new, new_branch_text, new_branch_cursor);
        }

        KeyCode::Enter => {
            let (branch, existing_wt, is_new) = if entering_new {
                if new_branch_text.is_empty() { return; }
                (new_branch_text.clone(), None, true)
            } else {
                if filtered_len == 0 { return; }
                let entry = filtered[cursor.min(filtered_len - 1)];
                (entry.name.clone(), entry.worktree_path.clone(), false)
            };
            let repo_path = std::path::PathBuf::from(&repo_root);
            if let Some(wt_path) = existing_wt {
                let folder = std::path::Path::new(&wt_path)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| branch.clone());
                app.mode = Mode::WorktreeFlow(WorktreeStep::Options {
                    repo_root, branch, folder,
                    opts: app.default_new_session_opts(),
                    existing_wt_path: Some(wt_path),
                });
                return;
            }
            // For a new branch, detect the default remote branch as the base.
            let base_branch = if is_new {
                let base = crate::git::default_branch(&repo_path);
                let full = format!("origin/{}", base);
                Some(full)
            } else {
                None
            };
            let folder_name = crate::git::branch_to_folder(&repo_path, &branch);
            let parent = repo_path.parent().unwrap_or(&repo_path).to_path_buf();
            let folder = parent.join(&folder_name).to_string_lossy().into_owned();
            let folder_cursor = folder.chars().count();
            app.mode = Mode::WorktreeFlow(WorktreeStep::FolderName {
                repo_root, branch, folder, cursor: folder_cursor, base_branch,
            });
        }

        _ => {
            if entering_new {
                if let Some((new_t, new_nc)) = apply_text_key(&new_branch_text, new_branch_cursor, &key) {
                    set!(filter, filter_cursor, cursor, entering_new, new_t, new_nc);
                }
            } else {
                if let Some((new_f, new_fc)) = apply_text_key(&filter, filter_cursor, &key) {
                    set!(new_f, new_fc, 0, entering_new, new_branch_text, new_branch_cursor);
                }
            }
        }
    }
}

fn handle_worktree_folder_name(
    app: &mut App,
    key: KeyEvent,
    repo_root: String,
    branch: String,
    folder: String,
    cursor: usize,
    base_branch: Option<String>,
) {
    match key.code {
        KeyCode::Esc   => { app.mode = Mode::Normal; }
        KeyCode::Enter => {
            let base_branch_cursor = base_branch.as_deref().map(|b| b.chars().count()).unwrap_or(0);
            let mut opts = app.default_new_session_opts();
            opts.base_branch = base_branch;
            opts.base_branch_cursor = base_branch_cursor;
            app.mode = Mode::WorktreeFlow(WorktreeStep::Options {
                repo_root, branch, folder, opts, existing_wt_path: None,
            });
        }
        _ => {
            if let Some((new_f, new_c)) = apply_text_key(&folder, cursor, &key) {
                app.mode = Mode::WorktreeFlow(WorktreeStep::FolderName {
                    repo_root, branch, folder: new_f, cursor: new_c, base_branch,
                });
            }
        }
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

    let field_count: u8 = if opts.base_branch.is_some() { 6 } else { 5 };

    match key.code {
        KeyCode::Esc => { app.mode = Mode::Normal; }
        KeyCode::Enter => {
            app.execute_worktree(&repo_root, &branch, &folder, &opts, existing_wt_path.as_deref());
        }
        KeyCode::Tab => {
            opts.field = (opts.field + 1) % field_count;
            app.mode = back!();
        }
        KeyCode::BackTab => {
            opts.field = if opts.field == 0 { field_count - 1 } else { opts.field - 1 };
            app.mode = back!();
        }
        KeyCode::Left if opts.field != 5 => {
            match opts.field {
                0 => opts.model_idx = if opts.model_idx == 0 { AVAILABLE_MODELS.len() - 1 } else { opts.model_idx - 1 },
                1 => opts.effort_idx = if opts.effort_idx == 0 { AVAILABLE_EFFORTS.len() - 1 } else { opts.effort_idx - 1 },
                3 => opts.color_idx = if opts.color_idx == 0 { WINDOW_COLORS.len() - 1 } else { opts.color_idx - 1 },
                _ => {}
            }
            app.mode = back!();
        }
        KeyCode::Right if opts.field != 5 => {
            match opts.field {
                0 => opts.model_idx = (opts.model_idx + 1) % AVAILABLE_MODELS.len(),
                1 => opts.effort_idx = (opts.effort_idx + 1) % AVAILABLE_EFFORTS.len(),
                3 => opts.color_idx = (opts.color_idx + 1) % WINDOW_COLORS.len(),
                _ => {}
            }
            app.mode = back!();
        }
        KeyCode::Char(' ') if opts.field != 5 => {
            match opts.field {
                2 => opts.launch_claude = !opts.launch_claude,
                4 => opts.open_vscode = !opts.open_vscode,
                _ => {}
            }
            app.mode = back!();
        }
        _ => {
            // Text editing for the base branch field (field 5).
            if opts.field == 5 {
                if let Some(base) = opts.base_branch.take() {
                    if let Some((new_b, new_bc)) = apply_text_key(&base, opts.base_branch_cursor, &key) {
                        opts.base_branch = Some(new_b);
                        opts.base_branch_cursor = new_bc;
                    } else {
                        opts.base_branch = Some(base);
                    }
                    app.mode = back!();
                }
            }
        }
    }
}

fn handle_history(app: &mut App, key: KeyEvent) {
    use super::mode::HistoryStep;
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

    // Esc always exits the browser.
    if key.code == KeyCode::Esc {
        app.mode = Mode::Normal;
        return;
    }

    // While loading, ignore everything except Esc (handled above).
    let (entries, repo_root, filter, filter_cursor, cursor) = match &app.mode {
        Mode::History(HistoryStep::List { entries, repo_root, filter, filter_cursor, cursor }) =>
            (entries.clone(), repo_root.clone(), filter.clone(), *filter_cursor, *cursor),
        _ => return,
    };

    // Compute the currently-filtered view (same fuzzy idiom as the branch/folder pickers).
    let filtered: Vec<&crate::history::SessionEntry> =
        fuzzy_sort(&entries, &filter, |e| format!("{} {}", e.title, e.branch.clone().unwrap_or_default()));

    match key.code {
        KeyCode::Up => {
            let new_cursor = if cursor == 0 { 0 } else { cursor - 1 };
            set_history_cursor(app, new_cursor);
        }
        KeyCode::Down => {
            let max = filtered.len().saturating_sub(1);
            set_history_cursor(app, (cursor + 1).min(max));
        }
        KeyCode::Char('n') if ctrl => {
            let max = filtered.len().saturating_sub(1);
            set_history_cursor(app, (cursor + 1).min(max));
        }
        KeyCode::Char('p') if ctrl => {
            let new_cursor = if cursor == 0 { 0 } else { cursor - 1 };
            set_history_cursor(app, new_cursor);
        }
        KeyCode::Enter => {
            if let Some(entry) = filtered.get(cursor).copied().cloned() {
                app.preview_session(&entry);
            }
        }
        KeyCode::Char('r') if ctrl => {
            if let Some(entry) = filtered.get(cursor).copied().cloned() {
                app.resume_session(&entry, &repo_root);
            }
        }
        _ => {
            // Text editing on the filter; reset cursor to 0 when the filter changes.
            if let Some((new_text, new_cur)) = apply_text_key(&filter, filter_cursor, &key) {
                if let Mode::History(HistoryStep::List { filter, filter_cursor, cursor, .. }) = &mut app.mode {
                    let changed = *filter != new_text;
                    *filter = new_text;
                    *filter_cursor = new_cur;
                    if changed { *cursor = 0; }
                }
            }
        }
    }
}

/// Set the list cursor inside `Mode::History(List)`.
fn set_history_cursor(app: &mut App, new_cursor: usize) {
    if let Mode::History(super::mode::HistoryStep::List { cursor, .. }) = &mut app.mode {
        *cursor = new_cursor;
    }
}

fn handle_folder_pick(app: &mut App, key: KeyEvent) {
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
        FolderPickStep::Picking { root, dirs, filter, filter_cursor, cursor } => {
            let filtered = fuzzy_sort(&dirs, &filter, |d| {
                d.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
            });
            let filtered_len = filtered.len();
            let clamped = if filtered_len == 0 { 0 } else { cursor.min(filtered_len - 1) };

            macro_rules! set_pick {
                ($f:expr, $fc:expr, $c:expr) => {
                    app.mode = Mode::FolderPick(FolderPickStep::Picking {
                        root: root.clone(), dirs: dirs.clone(),
                        filter: $f, filter_cursor: $fc, cursor: $c,
                    })
                };
            }

            match key.code {
                KeyCode::Esc => { app.mode = Mode::Normal; }

                KeyCode::Up | KeyCode::Char('k') => {
                    let nc = if cursor == 0 { filtered_len.saturating_sub(1) } else { cursor - 1 };
                    set_pick!(filter, filter_cursor, nc);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    let nc = if filtered_len == 0 { 0 } else { (cursor + 1) % filtered_len };
                    set_pick!(filter, filter_cursor, nc);
                }
                KeyCode::Enter => {
                    if let Some(path) = filtered.get(clamped) {
                        app.mode = Mode::FolderPick(FolderPickStep::Options {
                            path: (*path).clone(), is_new: false,
                            opts: app.default_new_session_opts(),
                        });
                    } else if !filter.is_empty() {
                        app.mode = Mode::FolderPick(FolderPickStep::Options {
                            path: root.join(&filter), is_new: true,
                            opts: app.default_new_session_opts(),
                        });
                    }
                }
                KeyCode::Right if !filtered.is_empty() => {
                    if let Some(path) = filtered.get(clamped) {
                        app.navigate_folder_into((*path).clone());
                    }
                }
                // Left / Backspace-on-empty → navigate up (not text editing)
                KeyCode::Left => { app.navigate_folder_up(); }
                KeyCode::Backspace if filter.is_empty() => { app.navigate_folder_up(); }
                _ => {
                    if let Some((new_f, new_fc)) = apply_text_key(&filter, filter_cursor, &key) {
                        set_pick!(new_f, new_fc, 0);
                    }
                }
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

// ─── Smart choice parsing ─────────────────────────────────────────────────────

use crate::sidebar::mode::ActionItem;

/// Parse a short text (e.g. an agent's `needs` field) for selectable choices.
/// Returns `Some(items)` only when 2–6 distinct options are unambiguously detected.
pub fn parse_choices(text: &str) -> Option<Vec<ActionItem>> {
    // y/n binary prompt
    if let Some(items) = parse_yn(text) { return Some(items); }
    // "choose A (...), B (...), or C (...)" inline style
    if let Some(items) = parse_inline_options(text) { return Some(items); }
    None
}

/// Parse the last visible lines of a pane's content for selectable choices.
pub fn parse_choices_from_pane(content: &str) -> Option<Vec<ActionItem>> {
    let lines: Vec<&str> = content.lines().collect();
    let tail: Vec<&str> = lines.iter().rev().take(30).copied().collect::<Vec<_>>()
        .into_iter().rev().collect();
    let joined = tail.join("\n");

    if let Some(items) = parse_yn(&joined) { return Some(items); }
    if let Some(items) = parse_numbered_list(&tail) { return Some(items); }
    if let Some(items) = parse_inline_options(&joined) { return Some(items); }
    None
}

fn make_send(label: impl Into<String>, text: impl Into<String>) -> ActionItem {
    ActionItem::SendText { label: label.into(), text: text.into() }
}

fn parse_yn(text: &str) -> Option<Vec<ActionItem>> {
    let lower = text.to_lowercase();
    if lower.contains("[y/n]") || lower.contains("(y/n)") {
        return Some(vec![make_send("y — yes", "y"), make_send("n — no", "n")]);
    }
    if lower.contains("[y/n") || lower.contains("[yes/no]") {
        return Some(vec![make_send("yes", "yes"), make_send("no", "no")]);
    }
    None
}

/// Parse numbered or lettered list items from recent pane lines.
/// Matches: "1. text", "2) text", "A. text" (if ≥2 items).
/// Handles leading selection cursors like "❯", ">", "▶" that terminals draw next to the
/// currently-highlighted option — those characters appear before "1. Yes" and would
/// otherwise cause the first option to be skipped.
fn parse_numbered_list(lines: &[&str]) -> Option<Vec<ActionItem>> {
    let mut items = Vec::new();
    for &line in lines {
        let trimmed = line.trim();
        // Strip any leading non-alphanumeric prefix (cursor/selection indicators like ❯, >, ▶)
        let stripped = trimmed
            .trim_start_matches(|c: char| !c.is_ascii_alphanumeric())
            .trim_start();

        // Match "1." "1)" "A." "A)" at the start of the stripped line
        let rest = if let Some(r) = stripped.strip_prefix(|c: char| c.is_ascii_digit())
            .and_then(|s| s.strip_prefix('.').or_else(|| s.strip_prefix(')')))
        {
            r.trim()
        } else if let Some(r) = stripped.strip_prefix(|c: char| c.is_ascii_alphabetic() && c.is_uppercase())
            .and_then(|s| s.strip_prefix('.').or_else(|| s.strip_prefix(')')))
        {
            r.trim()
        } else {
            continue;
        };
        if rest.len() >= 2 {
            // Use stripped (without cursor indicator) as the label, rest as the sent text
            items.push(make_send(stripped, rest));
        }
    }
    if items.len() >= 2 && items.len() <= 8 {
        Some(items)
    } else {
        None
    }
}

/// Parse inline "choose A (desc), B (desc), or C (desc)" style options.
/// Returns items when 2–6 single-letter options with parenthetical descriptions are found.
fn parse_inline_options(text: &str) -> Option<Vec<ActionItem>> {
    // Strip "choose" / "pick" / "select" prefix
    let cleaned = {
        let lower = text.to_lowercase();
        let start = ["choose ", "pick ", "select "]
            .iter()
            .find_map(|p| lower.find(p).map(|i| i + p.len()))
            .unwrap_or(0);
        &text[start..]
    };

    // Split on ", " and " or " separators
    let parts: Vec<&str> = cleaned
        .split(|c| c == ',')
        .flat_map(|s| s.split(" or "))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();

    if parts.len() < 2 || parts.len() > 6 { return None; }

    let mut items = Vec::new();
    for part in &parts {
        // Must start with a single letter/digit optionally followed by description in parens
        let first_char = part.chars().next()?;
        if !first_char.is_ascii_alphanumeric() { return None; }
        // The send text is just the first token (the letter/identifier)
        let send_text = part.split_whitespace().next().unwrap_or(part);
        // Strip trailing period/colon from send text if it accidentally got included
        let send_text = send_text.trim_end_matches(|c| c == '.' || c == ':');
        items.push(make_send(*part, send_text));
    }

    if items.len() >= 2 { Some(items) } else { None }
}
