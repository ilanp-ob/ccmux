use crate::app::{
    derive_folder_name, expand_path, is_houston_repo,
    BranchSelectField, Mode, WorktreeFlowState, App,
};
use crate::config::{AVAILABLE_EFFORTS, AVAILABLE_MODELS};
use crate::git::GitContext;
use crate::tmux::Tmux;

impl App {
    pub fn start_worktree_flow(&mut self) {
        self.clear_messages();
        let Some(session) = self.selected_session() else { return };
        let Some(ref git) = session.git_context else {
            self.error = Some("Not a git repository".to_string());
            return;
        };

        let source_repo = if git.is_worktree {
            git.main_repo_path.clone().unwrap_or_else(|| session.working_directory.clone())
        } else {
            session.working_directory.clone()
        };
        let server = session.server.clone();

        self.worktree_flow_source_repo = Some(source_repo.clone());
        self.worktree_flow_server = server;

        self.mode = Mode::WorktreeFlow {
            state: WorktreeFlowState::Fetching,
        };

        let _ = GitContext::fetch(&source_repo);

        let all_branches = GitContext::list_all_branches(&source_repo)
            .unwrap_or_default();

        let base_branch = self.config.worktree.defaults.base_branch.clone();

        self.mode = Mode::WorktreeFlow {
            state: WorktreeFlowState::BranchSelect {
                all_branches,
                filter_input: String::new(),
                selected: None,
                create_new: false,
                base_branch,
                field: BranchSelectField::Filter,
            },
        };
    }

    pub fn worktree_flow_filtered_branches(&self) -> Vec<&str> {
        if let Mode::WorktreeFlow {
            state: WorktreeFlowState::BranchSelect {
                ref all_branches,
                ref filter_input,
                ..
            },
        } = self.mode
        {
            if filter_input.is_empty() {
                all_branches.iter().map(|s| s.as_str()).collect()
            } else {
                let input_lower = filter_input.to_lowercase();
                all_branches
                    .iter()
                    .filter(|b| b.to_lowercase().contains(&input_lower))
                    .map(|s| s.as_str())
                    .collect()
            }
        } else {
            vec![]
        }
    }

    pub fn worktree_flow_confirm_branch(&mut self) {
        let (branch, is_new, base_branch) = if let Mode::WorktreeFlow {
            state: WorktreeFlowState::BranchSelect {
                ref filter_input,
                selected,
                create_new,
                ref base_branch,
                ..
            },
        } = self.mode
        {
            let filtered = self.worktree_flow_filtered_branches();

            if create_new {
                if filter_input.is_empty() {
                    self.error = Some("Branch name cannot be empty".to_string());
                    return;
                }
                (filter_input.clone(), true, base_branch.clone())
            } else if let Some(idx) = selected {
                let branch = filtered.get(idx).unwrap_or(&filter_input.as_str()).to_string();
                (branch, false, base_branch.clone())
            } else if let Some(first) = filtered.first() {
                (first.to_string(), false, base_branch.clone())
            } else {
                (filter_input.clone(), true, base_branch.clone())
            }
        } else {
            return;
        };

        let source_repo = self.worktree_flow_source_repo.as_ref().cloned().unwrap_or_default();
        let is_houston = is_houston_repo(&source_repo, &self.config.worktree.houston_path);
        let folder = derive_folder_name(&branch, is_houston);

        self.mode = Mode::WorktreeFlow {
            state: WorktreeFlowState::FolderName {
                branch,
                is_new_branch: is_new,
                base_branch,
                folder,
            },
        };
    }

    pub fn worktree_flow_confirm_folder(&mut self) {
        let (branch, is_new, base_branch, folder) = if let Mode::WorktreeFlow {
            state: WorktreeFlowState::FolderName {
                ref branch,
                is_new_branch,
                ref base_branch,
                ref folder,
            },
        } = self.mode
        {
            if folder.is_empty() {
                self.error = Some("Folder name cannot be empty".to_string());
                return;
            }
            (branch.clone(), is_new_branch, base_branch.clone(), folder.clone())
        } else {
            return;
        };

        let model_index = AVAILABLE_MODELS
            .iter()
            .position(|m| *m == self.config.claude.default_model)
            .unwrap_or(1);

        let effort_index = AVAILABLE_EFFORTS
            .iter()
            .position(|e| *e == self.config.claude.default_effort)
            .unwrap_or(2);

        self.mode = Mode::WorktreeFlow {
            state: WorktreeFlowState::ClaudeOptions {
                branch,
                is_new_branch: is_new,
                base_branch,
                folder,
                model_index,
                effort_index,
                launch_claude: true,
                color_index: 0,
                open_vscode: false,
                field: 0,
            },
        };
    }

    pub fn worktree_flow_execute(&mut self) {
        let (branch, is_new_branch, _base_branch, folder, model_index, effort_index, launch_claude, color_index, open_vscode) =
            if let Mode::WorktreeFlow {
                state: WorktreeFlowState::ClaudeOptions {
                    ref branch,
                    is_new_branch,
                    ref base_branch,
                    ref folder,
                    model_index,
                    effort_index,
                    launch_claude,
                    color_index,
                    open_vscode,
                    ..
                },
            } = self.mode
            {
                (
                    branch.clone(),
                    is_new_branch,
                    base_branch.clone(),
                    folder.clone(),
                    model_index,
                    effort_index,
                    launch_claude,
                    color_index,
                    open_vscode,
                )
            } else {
                return;
            };

        let source_repo = self.worktree_flow_source_repo.take().unwrap_or_default();
        let _server_unused = self.worktree_flow_server.take();
        let server = self.managed_server.clone();
        // Place worktrees as siblings of the main repo (parent dir), falling
        // back to config base_dir if the parent can't be determined.
        let base_dir = source_repo
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| expand_path(&self.config.worktree.base_dir));
        let worktree_path = base_dir.join(&folder);

        let (local_branch, actually_new) = if is_new_branch {
            (branch.clone(), true)
        } else if branch.contains('/') {
            let local_name = branch.split('/').skip(1).collect::<Vec<_>>().join("/");
            (local_name, true)
        } else {
            (branch.clone(), false)
        };

        // If the path already exists, check whether it's an existing worktree we can reuse.
        let already_exists = worktree_path.exists();
        let worktree_ok = if already_exists {
            let is_worktree = git2::Repository::discover(&worktree_path)
                .map(|r| r.is_worktree())
                .unwrap_or(false);
            if is_worktree {
                self.message = Some(format!("Attaching to existing worktree '{}'", folder));
                true
            } else {
                self.error = Some(format!(
                    "Path '{}' already exists and is not a git worktree",
                    worktree_path.display()
                ));
                false
            }
        } else {
            match GitContext::create_worktree(&source_repo, &worktree_path, &local_branch, actually_new) {
                Ok(_) => true,
                Err(e) => {
                    self.error = Some(format!("Failed to create worktree: {}", e));
                    false
                }
            }
        };

        if worktree_ok {
            let current_session = self.managed_session.clone();

            match Tmux::new_window(server.as_deref(), &current_session, &folder, &worktree_path) {
                Ok(window_id) => {
                    // Apply window color if one was chosen
                    let (_, color_hex, tmux_colour) = crate::config::WINDOW_COLORS[color_index];
                    if !tmux_colour.is_empty() {
                        let _ = Tmux::set_window_color(server.as_deref(), &window_id, tmux_colour);
                    }

                    if launch_claude {
                        let model = AVAILABLE_MODELS[model_index];
                        let effort = AVAILABLE_EFFORTS[effort_index];
                        let alias = &self.config.claude.alias;
                        let safe_name = folder.replace('\'', "'\\''");
                        let cmd = format!(
                            "{} --model {} --effort {} --name '{}'",
                            alias, model, effort, safe_name
                        );
                        let _ = Tmux::send_keys(server.as_deref(), &window_id, &cmd);
                    }

                    if open_vscode {
                        // Write VS Code workspace color before opening so it's picked up immediately
                        if !color_hex.is_empty() {
                            apply_vscode_color(&worktree_path, color_hex);
                        }
                        open_vscode_window(&worktree_path, &mut self.error, &mut self.message);
                    }

                    self.refresh_sessions();
                    if !already_exists {
                        self.message = Some(format!(
                            "Created worktree '{}' in window '{}'",
                            local_branch, folder
                        ));
                    }
                }
                Err(e) => {
                    self.error = Some(format!(
                        "Worktree ready but failed to create window: {}",
                        e
                    ));
                }
            }
        }

        self.mode = Mode::Normal;
    }
}

/// Merge `workbench.colorCustomizations` into `.vscode/settings.json`.
fn apply_vscode_color(path: &std::path::Path, hex: &str) {
    let vscode_dir = path.join(".vscode");
    let settings_path = vscode_dir.join("settings.json");

    // Read existing settings if present
    let mut settings: serde_json::Value = if settings_path.exists() {
        std::fs::read_to_string(&settings_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or(serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    // Derive a slightly darker shade for inactive title bar
    let inactive = darken_hex(hex, 40);

    let color_section = serde_json::json!({
        "titleBar.activeBackground": hex,
        "titleBar.activeForeground": "#ffffff",
        "titleBar.inactiveBackground": inactive,
        "titleBar.inactiveForeground": "#cccccc",
    });

    if let Some(obj) = settings.as_object_mut() {
        let entry = obj
            .entry("workbench.colorCustomizations")
            .or_insert(serde_json::json!({}));
        if let (Some(target), Some(src)) = (entry.as_object_mut(), color_section.as_object()) {
            for (k, v) in src {
                target.insert(k.clone(), v.clone());
            }
        }
    }

    let _ = std::fs::create_dir_all(&vscode_dir);
    if let Ok(json) = serde_json::to_string_pretty(&settings) {
        let _ = std::fs::write(&settings_path, json);
    }
}

fn open_vscode_window(
    path: &std::path::Path,
    error: &mut Option<String>,
    _message: &mut Option<String>,
) {
    let status = std::process::Command::new("code")
        .args(["--new-window", &path.to_string_lossy()])
        .status();
    match status {
        Ok(s) if s.success() => {}
        Ok(_) => {
            *error = Some("VS Code ('code') exited with an error".to_string());
        }
        Err(_) => {
            *error = Some("VS Code CLI ('code') not found on PATH — install it from VS Code: Shell Command > Install 'code' command".to_string());
        }
    }
}

/// Darken a hex color (#RRGGBB) by subtracting `amount` from each channel.
fn darken_hex(hex: &str, amount: u8) -> String {
    let hex = hex.trim_start_matches('#');
    if hex.len() != 6 {
        return format!("#{}", hex);
    }
    let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(0).saturating_sub(amount);
    let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(0).saturating_sub(amount);
    let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(0).saturating_sub(amount);
    format!("#{:02X}{:02X}{:02X}", r, g, b)
}
