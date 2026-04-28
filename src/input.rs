use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::{App, CreatePullRequestField, Mode, NewSessionField, NewWorktreeField};

/// Handle a key event and update the application state
pub fn handle_key(app: &mut App, key: KeyEvent) {
    // Clear messages on any key press
    app.clear_messages();

    match &app.mode {
        Mode::Normal => handle_normal_mode(app, key),
        Mode::ActionMenu => handle_action_menu_mode(app, key),
        Mode::Filter { .. } => handle_filter_mode(app, key),
        Mode::ConfirmAction => handle_confirm_action_mode(app, key),
        Mode::NewSession { .. } => handle_new_session_mode(app, key),
        Mode::Rename { .. } => handle_rename_mode(app, key),
        Mode::Commit { .. } => handle_commit_mode(app, key),
        Mode::NewWorktree { .. } => handle_new_worktree_mode(app, key),
        Mode::CreatePullRequest { .. } => handle_create_pr_mode(app, key),
        Mode::Help => handle_help_mode(app, key),
        Mode::WorktreeFlow { .. } => handle_worktree_flow_mode(app, key),
    }
}

fn handle_normal_mode(app: &mut App, key: KeyEvent) {
    match key.code {
        // Quit
        KeyCode::Char('q') | KeyCode::Esc => {
            app.should_quit = true;
        }

        // Navigation
        KeyCode::Char('j') | KeyCode::Down => {
            app.select_next();
        }
        KeyCode::Char('k') | KeyCode::Up => {
            app.select_prev();
        }

        // Enter action menu
        KeyCode::Char('l') | KeyCode::Right => {
            app.enter_action_menu();
        }

        // Switch to session (quick action)
        KeyCode::Enter => {
            app.switch_to_selected();
        }

        // New session
        KeyCode::Char('n') => {
            app.start_new_session();
        }

        // Kill session (capital K to avoid accidents)
        KeyCode::Char('K') => {
            app.start_kill();
        }

        // Rename session
        KeyCode::Char('r') => {
            app.start_rename();
        }

        // Filter
        KeyCode::Char('/') => {
            app.start_filter();
        }

        // Clear filter
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.clear_filter();
        }

        // Refresh
        KeyCode::Char('R') => {
            app.refresh();
        }

        // Help
        KeyCode::Char('?') => {
            app.show_help();
        }

        // Start worktree flow
        KeyCode::Char('w') => {
            app.start_worktree_flow();
        }

        _ => {}
    }
}

fn handle_filter_mode(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            app.cancel();
        }
        KeyCode::Enter => {
            app.apply_filter();
        }
        KeyCode::Backspace => {
            if let Mode::Filter { ref mut input } = app.mode {
                input.pop();
            }
        }
        KeyCode::Char(c) => {
            if let Mode::Filter { ref mut input } = app.mode {
                input.push(c);
            }
        }
        _ => {}
    }
}

fn handle_action_menu_mode(app: &mut App, key: KeyEvent) {
    match key.code {
        // Navigate actions
        KeyCode::Char('j') | KeyCode::Down => {
            app.select_next_action();
        }
        KeyCode::Char('k') | KeyCode::Up => {
            app.select_prev_action();
        }

        // Execute selected action
        KeyCode::Enter | KeyCode::Char('l') | KeyCode::Right => {
            app.execute_selected_action();
        }

        // Back to session list
        KeyCode::Char('h') | KeyCode::Left | KeyCode::Esc => {
            app.cancel();
        }

        // Quit entirely
        KeyCode::Char('q') => {
            app.should_quit = true;
        }

        _ => {}
    }
}

fn handle_confirm_action_mode(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => {
            app.confirm_action();
        }
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
            app.cancel();
        }
        _ => {}
    }
}

fn handle_new_session_mode(app: &mut App, key: KeyEvent) {
    // Get current field to determine behavior
    let current_field = if let Mode::NewSession { field, .. } = &app.mode {
        *field
    } else {
        return;
    };

    match key.code {
        KeyCode::Esc => {
            app.cancel();
        }
        KeyCode::Tab => {
            // Toggle between name and path fields
            if let Mode::NewSession { ref mut field, .. } = app.mode {
                *field = match field {
                    NewSessionField::Name => NewSessionField::Path,
                    NewSessionField::Path => NewSessionField::Name,
                };
            }
        }
        KeyCode::Enter => {
            app.confirm_new_session(true); // Start claude by default
        }
        // Path completion navigation (only when path field is active)
        KeyCode::Up if current_field == NewSessionField::Path => {
            app.select_prev_new_session_path();
        }
        KeyCode::Down if current_field == NewSessionField::Path => {
            app.select_next_new_session_path();
        }
        // Accept completion with Right arrow (only when path field is active)
        KeyCode::Right if current_field == NewSessionField::Path => {
            app.accept_new_session_path_completion();
        }
        KeyCode::Backspace => {
            if let Mode::NewSession {
                ref mut name,
                ref mut path,
                ref field,
                ref mut path_selected,
                ..
            } = app.mode
            {
                match field {
                    NewSessionField::Name => {
                        name.pop();
                    }
                    NewSessionField::Path => {
                        path.pop();
                        *path_selected = None; // Reset selection on edit
                    }
                }
            }
            if current_field == NewSessionField::Path {
                app.update_new_session_path_suggestions();
            }
        }
        KeyCode::Char(c) => {
            if let Mode::NewSession {
                ref mut name,
                ref mut path,
                ref field,
                ref mut path_selected,
                ..
            } = app.mode
            {
                match field {
                    NewSessionField::Name => {
                        // Only allow valid session name characters
                        if c.is_alphanumeric() || c == '-' || c == '_' {
                            name.push(c);
                        }
                    }
                    NewSessionField::Path => {
                        path.push(c);
                        *path_selected = None; // Reset selection on edit
                    }
                }
            }
            if current_field == NewSessionField::Path {
                app.update_new_session_path_suggestions();
            }
        }
        _ => {}
    }
}

fn handle_rename_mode(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            app.cancel();
        }
        KeyCode::Enter => {
            app.confirm_rename();
        }
        KeyCode::Backspace => {
            if let Mode::Rename { ref mut new_name, .. } = app.mode {
                new_name.pop();
            }
        }
        KeyCode::Char(c) => {
            if let Mode::Rename { ref mut new_name, .. } = app.mode {
                // Only allow valid session name characters
                if c.is_alphanumeric() || c == '-' || c == '_' {
                    new_name.push(c);
                }
            }
        }
        _ => {}
    }
}

fn handle_commit_mode(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            app.cancel();
        }
        KeyCode::Enter => {
            app.confirm_commit();
        }
        KeyCode::Backspace => {
            if let Mode::Commit { ref mut message } = app.mode {
                message.pop();
            }
        }
        KeyCode::Char(c) => {
            if let Mode::Commit { ref mut message } = app.mode {
                message.push(c);
            }
        }
        _ => {}
    }
}

fn handle_new_worktree_mode(app: &mut App, key: KeyEvent) {
    // Get current field to determine behavior
    let current_field = if let Mode::NewWorktree { field, .. } = &app.mode {
        *field
    } else {
        return;
    };

    match key.code {
        KeyCode::Esc => {
            app.cancel();
        }
        KeyCode::Tab => {
            // Cycle through fields
            if let Mode::NewWorktree { ref mut field, .. } = app.mode {
                *field = match field {
                    NewWorktreeField::Branch => NewWorktreeField::Path,
                    NewWorktreeField::Path => NewWorktreeField::SessionName,
                    NewWorktreeField::SessionName => NewWorktreeField::Branch,
                };
            }
        }
        KeyCode::BackTab => {
            // Cycle backwards through fields
            if let Mode::NewWorktree { ref mut field, .. } = app.mode {
                *field = match field {
                    NewWorktreeField::Branch => NewWorktreeField::SessionName,
                    NewWorktreeField::Path => NewWorktreeField::Branch,
                    NewWorktreeField::SessionName => NewWorktreeField::Path,
                };
            }
        }
        KeyCode::Enter => {
            app.confirm_new_worktree();
        }
        KeyCode::Backspace => {
            if let Mode::NewWorktree {
                ref mut branch_input,
                ref mut worktree_path,
                ref mut session_name,
                ref mut path_selected,
                field,
                ..
            } = app.mode
            {
                match field {
                    NewWorktreeField::Branch => {
                        branch_input.pop();
                    }
                    NewWorktreeField::Path => {
                        worktree_path.pop();
                        *path_selected = None; // Reset selection on edit
                    }
                    NewWorktreeField::SessionName => {
                        session_name.pop();
                    }
                }
            }
            // Update suggestions after input changes
            if current_field == NewWorktreeField::Branch {
                app.update_worktree_suggestions();
            } else if current_field == NewWorktreeField::Path {
                app.update_worktree_path_suggestions();
            }
        }
        KeyCode::Char(c) => {
            if let Mode::NewWorktree {
                ref mut branch_input,
                ref mut worktree_path,
                ref mut session_name,
                ref mut path_selected,
                field,
                ..
            } = app.mode
            {
                match field {
                    NewWorktreeField::Branch => {
                        branch_input.push(c);
                    }
                    NewWorktreeField::Path => {
                        worktree_path.push(c);
                        *path_selected = None; // Reset selection on edit
                    }
                    NewWorktreeField::SessionName => {
                        // Only allow valid session name characters
                        if c.is_alphanumeric() || c == '-' || c == '_' {
                            session_name.push(c);
                        }
                    }
                }
            }
            // Update suggestions after input changes
            if current_field == NewWorktreeField::Branch {
                app.update_worktree_suggestions();
            } else if current_field == NewWorktreeField::Path {
                app.update_worktree_path_suggestions();
            }
        }
        // Navigate branch suggestions when in Branch field
        KeyCode::Down if current_field == NewWorktreeField::Branch => {
            let filtered_count = app.filtered_branches().len();
            if filtered_count > 0 {
                if let Mode::NewWorktree {
                    ref mut selected_branch,
                    ..
                } = app.mode
                {
                    *selected_branch =
                        Some(selected_branch.map(|i| (i + 1) % filtered_count).unwrap_or(0));
                }
                app.update_worktree_suggestions();
            }
        }
        KeyCode::Up if current_field == NewWorktreeField::Branch => {
            let filtered_count = app.filtered_branches().len();
            if filtered_count > 0 {
                if let Mode::NewWorktree {
                    ref mut selected_branch,
                    ..
                } = app.mode
                {
                    *selected_branch = Some(
                        selected_branch
                            .map(|i| if i == 0 { filtered_count - 1 } else { i - 1 })
                            .unwrap_or(filtered_count - 1),
                    );
                }
                app.update_worktree_suggestions();
            }
        }
        // Accept branch completion with Right arrow
        KeyCode::Right if current_field == NewWorktreeField::Branch => {
            app.accept_branch_completion();
        }
        // Navigate path suggestions when in Path field
        KeyCode::Down if current_field == NewWorktreeField::Path => {
            app.select_next_worktree_path();
        }
        KeyCode::Up if current_field == NewWorktreeField::Path => {
            app.select_prev_worktree_path();
        }
        // Accept path completion with Right arrow
        KeyCode::Right if current_field == NewWorktreeField::Path => {
            app.accept_worktree_path_completion();
        }
        _ => {}
    }
}

fn handle_create_pr_mode(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            app.cancel();
        }
        KeyCode::Tab => {
            // Cycle through fields
            if let Mode::CreatePullRequest { ref mut field, .. } = app.mode {
                *field = match field {
                    CreatePullRequestField::Title => CreatePullRequestField::Body,
                    CreatePullRequestField::Body => CreatePullRequestField::BaseBranch,
                    CreatePullRequestField::BaseBranch => CreatePullRequestField::Title,
                };
            }
        }
        KeyCode::BackTab => {
            // Cycle backwards through fields
            if let Mode::CreatePullRequest { ref mut field, .. } = app.mode {
                *field = match field {
                    CreatePullRequestField::Title => CreatePullRequestField::BaseBranch,
                    CreatePullRequestField::Body => CreatePullRequestField::Title,
                    CreatePullRequestField::BaseBranch => CreatePullRequestField::Body,
                };
            }
        }
        KeyCode::Enter => {
            app.confirm_create_pull_request();
        }
        KeyCode::Backspace => {
            if let Mode::CreatePullRequest {
                ref mut title,
                ref mut body,
                ref mut base_branch,
                field,
            } = app.mode
            {
                match field {
                    CreatePullRequestField::Title => {
                        title.pop();
                    }
                    CreatePullRequestField::Body => {
                        body.pop();
                    }
                    CreatePullRequestField::BaseBranch => {
                        base_branch.pop();
                    }
                }
            }
        }
        KeyCode::Char(c) => {
            if let Mode::CreatePullRequest {
                ref mut title,
                ref mut body,
                ref mut base_branch,
                field,
            } = app.mode
            {
                match field {
                    CreatePullRequestField::Title => {
                        title.push(c);
                    }
                    CreatePullRequestField::Body => {
                        body.push(c);
                    }
                    CreatePullRequestField::BaseBranch => {
                        // Branch names have specific allowed characters
                        if c.is_alphanumeric() || c == '-' || c == '_' || c == '/' {
                            base_branch.push(c);
                        }
                    }
                }
            }
        }
        _ => {}
    }
}

fn handle_help_mode(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('q') | KeyCode::Esc | KeyCode::Char('?') => {
            app.cancel();
        }
        _ => {}
    }
}

fn handle_worktree_flow_mode(app: &mut App, key: KeyEvent) {
    use crate::app::mode::{BranchSelectField, WorktreeFlowState};
    use crate::config::{AVAILABLE_EFFORTS, AVAILABLE_MODELS};

    let state = if let Mode::WorktreeFlow { ref state } = app.mode {
        state.clone()
    } else {
        return;
    };

    match state {
        WorktreeFlowState::Fetching => {
            if key.code == KeyCode::Esc {
                app.cancel();
            }
        }

        WorktreeFlowState::BranchSelect { field, .. } => match field {
            BranchSelectField::Filter => match key.code {
                KeyCode::Esc => app.cancel(),
                KeyCode::Enter => app.worktree_flow_confirm_branch(),
                KeyCode::Tab => {
                    if let Mode::WorktreeFlow {
                        state: WorktreeFlowState::BranchSelect {
                            ref mut create_new,
                            ref mut field,
                            ..
                        },
                    } = app.mode
                    {
                        if *create_new {
                            *field = BranchSelectField::BaseBranch;
                        } else {
                            *create_new = true;
                        }
                    }
                }
                KeyCode::Down => {
                    let count = app.worktree_flow_filtered_branches().len();
                    if count > 0 {
                        if let Mode::WorktreeFlow {
                            state: WorktreeFlowState::BranchSelect {
                                ref mut selected, ..
                            },
                        } = app.mode
                        {
                            *selected = Some(selected.map(|i| (i + 1) % count).unwrap_or(0));
                        }
                    }
                }
                KeyCode::Up => {
                    let count = app.worktree_flow_filtered_branches().len();
                    if count > 0 {
                        if let Mode::WorktreeFlow {
                            state: WorktreeFlowState::BranchSelect {
                                ref mut selected, ..
                            },
                        } = app.mode
                        {
                            *selected = Some(
                                selected
                                    .map(|i| if i == 0 { count - 1 } else { i - 1 })
                                    .unwrap_or(count - 1),
                            );
                        }
                    }
                }
                KeyCode::Backspace => {
                    if let Mode::WorktreeFlow {
                        state: WorktreeFlowState::BranchSelect {
                            ref mut filter_input,
                            ref mut selected,
                            ref mut create_new,
                            ..
                        },
                    } = app.mode
                    {
                        filter_input.pop();
                        *selected = None;
                        *create_new = false;
                    }
                }
                KeyCode::Char(c) => {
                    if let Mode::WorktreeFlow {
                        state: WorktreeFlowState::BranchSelect {
                            ref mut filter_input,
                            ref mut selected,
                            ref mut create_new,
                            ..
                        },
                    } = app.mode
                    {
                        filter_input.push(c);
                        *selected = None;
                        *create_new = false;
                    }
                }
                _ => {}
            },
            BranchSelectField::BaseBranch => match key.code {
                KeyCode::Esc => app.cancel(),
                KeyCode::Enter => app.worktree_flow_confirm_branch(),
                KeyCode::Tab | KeyCode::BackTab => {
                    if let Mode::WorktreeFlow {
                        state: WorktreeFlowState::BranchSelect { ref mut field, .. },
                    } = app.mode
                    {
                        *field = BranchSelectField::Filter;
                    }
                }
                KeyCode::Backspace => {
                    if let Mode::WorktreeFlow {
                        state: WorktreeFlowState::BranchSelect {
                            ref mut base_branch,
                            ..
                        },
                    } = app.mode
                    {
                        base_branch.pop();
                    }
                }
                KeyCode::Char(c) => {
                    if c.is_alphanumeric() || c == '-' || c == '_' || c == '/' {
                        if let Mode::WorktreeFlow {
                            state: WorktreeFlowState::BranchSelect {
                                ref mut base_branch,
                                ..
                            },
                        } = app.mode
                        {
                            base_branch.push(c);
                        }
                    }
                }
                _ => {}
            },
        },

        WorktreeFlowState::FolderName { .. } => match key.code {
            KeyCode::Esc => app.cancel(),
            KeyCode::Enter => app.worktree_flow_confirm_folder(),
            KeyCode::Backspace => {
                if let Mode::WorktreeFlow {
                    state: WorktreeFlowState::FolderName { ref mut folder, .. },
                } = app.mode
                {
                    folder.pop();
                }
            }
            KeyCode::Char(c) => {
                if c.is_alphanumeric() || c == '-' || c == '_' {
                    if let Mode::WorktreeFlow {
                        state: WorktreeFlowState::FolderName { ref mut folder, .. },
                    } = app.mode
                    {
                        folder.push(c);
                    }
                }
            }
            _ => {}
        },

        WorktreeFlowState::ClaudeOptions { .. } => match key.code {
            KeyCode::Esc => app.cancel(),
            KeyCode::Enter => app.worktree_flow_execute(),
            KeyCode::Tab => {
                if let Mode::WorktreeFlow {
                    state: WorktreeFlowState::ClaudeOptions { ref mut field, .. },
                } = app.mode
                {
                    *field = (*field + 1) % 3;
                }
            }
            KeyCode::BackTab => {
                if let Mode::WorktreeFlow {
                    state: WorktreeFlowState::ClaudeOptions { ref mut field, .. },
                } = app.mode
                {
                    *field = if *field == 0 { 2 } else { *field - 1 };
                }
            }
            KeyCode::Left | KeyCode::Char('h') => {
                if let Mode::WorktreeFlow {
                    state: WorktreeFlowState::ClaudeOptions {
                        ref mut model_index,
                        ref mut effort_index,
                        ref mut launch_claude,
                        field,
                        ..
                    },
                } = app.mode
                {
                    match field {
                        0 => {
                            if *model_index > 0 {
                                *model_index -= 1;
                            }
                        }
                        1 => {
                            if *effort_index > 0 {
                                *effort_index -= 1;
                            }
                        }
                        2 => *launch_claude = !*launch_claude,
                        _ => {}
                    }
                }
            }
            KeyCode::Right | KeyCode::Char('l') => {
                if let Mode::WorktreeFlow {
                    state: WorktreeFlowState::ClaudeOptions {
                        ref mut model_index,
                        ref mut effort_index,
                        ref mut launch_claude,
                        field,
                        ..
                    },
                } = app.mode
                {
                    match field {
                        0 => {
                            if *model_index < AVAILABLE_MODELS.len() - 1 {
                                *model_index += 1;
                            }
                        }
                        1 => {
                            if *effort_index < AVAILABLE_EFFORTS.len() - 1 {
                                *effort_index += 1;
                            }
                        }
                        2 => *launch_claude = !*launch_claude,
                        _ => {}
                    }
                }
            }
            _ => {}
        },

        WorktreeFlowState::Executing => {
            // No input during execution
        }
    }
}
