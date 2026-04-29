use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result};

use crate::detection::detect_status;
use crate::git::GitContext;
use crate::session::{ClaudeCodeStatus, Pane, PaneType, Session};

pub struct Tmux;

fn classify_pane_command(command: &str) -> Option<PaneType> {
    if command == "claude" || command.contains("claude") {
        Some(PaneType::Claude)
    } else if command == "ops-cli" || command == "ocli" || command == "ops" || command.contains("ops-cli") {
        Some(PaneType::Ocli)
    } else {
        None
    }
}

impl Tmux {
    fn cmd(server: Option<&str>) -> Command {
        let mut cmd = Command::new("tmux");
        if let Some(s) = server {
            cmd.args(["-L", s]);
        }
        cmd
    }

    pub fn discover_servers() -> Vec<String> {
        let uid = unsafe { libc::getuid() };
        let socket_dir = PathBuf::from(format!("/tmp/tmux-{}", uid));
        if !socket_dir.is_dir() {
            return vec![];
        }
        let mut servers = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&socket_dir) {
            for entry in entries.flatten() {
                if let Some(name) = entry.file_name().to_str() {
                    if !name.starts_with('.') {
                        servers.push(name.to_string());
                    }
                }
            }
        }
        servers.sort();
        servers
    }

    pub fn list_all_sessions(filter_server: Option<&str>) -> Result<Vec<Session>> {
        let servers = match filter_server {
            Some(s) => vec![s.to_string()],
            None => Self::discover_servers(),
        };

        let mut all_sessions = Vec::new();

        for server_name in &servers {
            if let Ok(sessions) = Self::list_sessions(Some(server_name)) {
                all_sessions.extend(sessions);
            }
        }

        all_sessions.sort_by(|a, b| {
            b.attached
                .cmp(&a.attached)
                .then_with(|| a.server.cmp(&b.server))
                .then_with(|| a.name.cmp(&b.name))
                .then_with(|| a.window_label.cmp(&b.window_label))
        });

        Ok(all_sessions)
    }

    pub fn list_sessions(server: Option<&str>) -> Result<Vec<Session>> {
        let output = Self::cmd(server)
            .args([
                "list-sessions",
                "-F",
                "#{session_name}\t#{session_created}\t#{session_attached}\t#{session_windows}",
            ])
            .output()
            .context("Failed to execute tmux list-sessions")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("no server running") || stderr.contains("no sessions") {
                return Ok(Vec::new());
            }
            anyhow::bail!("tmux list-sessions failed: {}", stderr);
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut sessions = Vec::new();
        let server_str = server.map(|s| s.to_string());

        for line in stdout.lines() {
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() >= 4 {
                let name = parts[0].to_string();
                let created = parts[1].parse().unwrap_or(0);
                let attached = parts[2] == "1";
                let window_count = parts[3].parse().unwrap_or(1);

                let panes = Self::list_panes(server, &name).unwrap_or_default();

                let detected_panes: Vec<(&Pane, PaneType)> = panes
                    .iter()
                    .filter_map(|p| classify_pane_command(&p.current_command).map(|t| (p, t)))
                    .collect();

                let multi = detected_panes.len() > 1;

                if detected_panes.is_empty() {
                    let working_directory = panes
                        .first()
                        .map(|p| p.current_path.clone())
                        .unwrap_or_default();
                    let git_context = GitContext::detect(&working_directory);

                    sessions.push(Session {
                        name: name.clone(),
                        window_id: String::new(),
                        created,
                        attached,
                        working_directory,
                        window_count,
                        panes: panes.clone(),
                        claude_code_pane: None,
                        pane_type: PaneType::Claude,
                        claude_code_status: ClaudeCodeStatus::Unknown,
                        window_label: None,
                        target_window_index: None,
                        git_context,
                        server: server_str.clone(),
                    });
                } else {
                    for (detected_pane, pane_type) in detected_panes {
                        let status = Self::capture_pane(server, &detected_pane.id, 15, true)
                            .map(|content| detect_status(&content))
                            .unwrap_or(ClaudeCodeStatus::Unknown);

                        let working_directory = detected_pane.current_path.clone();
                        let git_context = GitContext::detect(&working_directory);

                        let (window_label, target_window_index) = if multi {
                            (
                                Some(detected_pane.window_name.clone()),
                                Some(detected_pane.window_index.clone()),
                            )
                        } else {
                            (None, None)
                        };

                        sessions.push(Session {
                            name: name.clone(),
                            window_id: String::new(),
                            created,
                            attached,
                            working_directory,
                            window_count,
                            panes: panes.clone(),
                            claude_code_pane: Some(detected_pane.id.clone()),
                            pane_type,
                            claude_code_status: status,
                            window_label,
                            target_window_index,
                            git_context,
                            server: server_str.clone(),
                        });
                    }
                }
            }
        }

        sessions.sort_by(|a, b| {
            b.attached
                .cmp(&a.attached)
                .then_with(|| a.name.cmp(&b.name))
                .then_with(|| a.window_label.cmp(&b.window_label))
        });

        Ok(sessions)
    }

    fn list_panes(server: Option<&str>, session: &str) -> Result<Vec<Pane>> {
        let output = Self::cmd(server)
            .args([
                "list-panes",
                "-s",
                "-t",
                session,
                "-F",
                "#{pane_id}\t#{pane_current_command}\t#{pane_current_path}\t#{window_index}\t#{window_name}",
            ])
            .output()
            .context("Failed to execute tmux list-panes")?;

        if !output.status.success() {
            return Ok(Vec::new());
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut panes = Vec::new();

        for line in stdout.lines() {
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() >= 5 {
                panes.push(Pane {
                    id: parts[0].to_string(),
                    current_command: parts[1].to_string(),
                    current_path: PathBuf::from(parts[2]),
                    window_index: parts[3].to_string(),
                    window_name: parts[4].to_string(),
                });
            }
        }

        Ok(panes)
    }

    pub fn capture_pane(server: Option<&str>, pane_id: &str, lines: usize, strip_empty: bool) -> Result<String> {
        let output = Self::cmd(server)
            .args([
                "capture-pane",
                "-t",
                pane_id,
                "-p",
                "-J",
                "-e",
            ])
            .output()
            .context("Failed to capture pane")?;

        if !output.status.success() {
            anyhow::bail!("Failed to capture pane {}", pane_id);
        }

        let content = String::from_utf8_lossy(&output.stdout);

        if strip_empty {
            let non_empty: Vec<&str> = content.lines().filter(|l| !l.trim().is_empty()).collect();
            let start = non_empty.len().saturating_sub(lines);
            let last_lines = &non_empty[start..];
            Ok(last_lines.join("\n"))
        } else {
            let all_lines: Vec<&str> = content.lines().collect();
            let last_non_empty = all_lines
                .iter()
                .rposition(|l| !l.trim().is_empty())
                .map(|i| i + 1)
                .unwrap_or(0);
            let trimmed = &all_lines[..last_non_empty];
            let start = trimmed.len().saturating_sub(lines);
            let last_lines = &trimmed[start..];
            Ok(last_lines.join("\n"))
        }
    }

    pub fn switch_to_session(server: Option<&str>, session: &str) -> Result<()> {
        let status = Self::cmd(server)
            .args(["switch-client", "-t", session])
            .status()
            .context("Failed to switch session")?;

        if !status.success() {
            anyhow::bail!("Failed to switch to session {}", session);
        }

        Ok(())
    }

    pub fn new_session(server: Option<&str>, name: &str, path: &std::path::Path, start_claude: bool) -> Result<()> {
        let path_str = path.to_string_lossy();

        let status = Self::cmd(server)
            .args(["new-session", "-d", "-s", name, "-c", &path_str])
            .status()
            .context("Failed to create new session")?;

        if !status.success() {
            anyhow::bail!("Failed to create session {}", name);
        }

        // tmux's `default-command "cd ~ && exec $SHELL"` overrides -c, so cd explicitly.
        let cd_cmd = format!("cd '{}'", path_str.replace('\'', "'\\''"));
        let _ = Self::cmd(server)
            .args(["send-keys", "-t", name, &cd_cmd, "Enter"])
            .status();

        if start_claude {
            let claude_cmd = format!("claude --name '{}'", name.replace('\'', "'\\''"));
            let _ = Self::cmd(server)
                .args(["send-keys", "-t", name, &claude_cmd, "Enter"])
                .status();
        }

        Ok(())
    }

    pub fn kill_session(server: Option<&str>, session: &str) -> Result<()> {
        let status = Self::cmd(server)
            .args(["kill-session", "-t", session])
            .status()
            .context("Failed to kill session")?;

        if !status.success() {
            anyhow::bail!("Failed to kill session {}", session);
        }

        Ok(())
    }

    pub fn rename_session(server: Option<&str>, old_name: &str, new_name: &str) -> Result<()> {
        let status = Self::cmd(server)
            .args(["rename-session", "-t", old_name, new_name])
            .status()
            .context("Failed to rename session")?;

        if !status.success() {
            anyhow::bail!("Failed to rename session {} to {}", old_name, new_name);
        }

        Ok(())
    }

    pub fn current_session(server: Option<&str>) -> Result<Option<String>> {
        let output = Self::cmd(server)
            .args(["display-message", "-p", "#{session_name}"])
            .output()
            .context("Failed to get current session")?;

        if !output.status.success() {
            return Ok(None);
        }

        let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if name.is_empty() {
            Ok(None)
        } else {
            Ok(Some(name))
        }
    }

    /// Create a new window and return its window ID (e.g. "@5").
    /// Using the ID for subsequent send-keys avoids automatic-rename race conditions.
    pub fn new_window(
        server: Option<&str>,
        session: &str,
        window_name: &str,
        path: &std::path::Path,
    ) -> Result<String> {
        let path_str = path.to_string_lossy();
        let output = Self::cmd(server)
            .args([
                "new-window", "-t", session, "-n", window_name, "-c", &path_str,
                "-P", "-F", "#{window_id}",
            ])
            .output()
            .context("Failed to create new window")?;
        if !output.status.success() {
            anyhow::bail!("Failed to create window '{}' in session '{}'", window_name, session);
        }
        let window_id = String::from_utf8_lossy(&output.stdout).trim().to_string();
        // tmux's `default-command "cd ~ && exec $SHELL"` overrides -c; target by
        // window_id so the send-keys lands even after automatic-rename.
        let cd_cmd = format!("cd '{}'", path_str.replace('\'', "'\\''"));
        let _ = Self::cmd(server)
            .args(["send-keys", "-t", &window_id, &cd_cmd, "Enter"])
            .status();
        Ok(window_id)
    }

    pub fn kill_window(server: Option<&str>, target: &str) -> Result<()> {
        let status = Self::cmd(server)
            .args(["kill-window", "-t", target])
            .status()
            .context("Failed to kill window")?;
        if !status.success() {
            anyhow::bail!("Failed to kill window {}", target);
        }
        Ok(())
    }

    pub fn rename_window(server: Option<&str>, target: &str, new_name: &str) -> Result<()> {
        let status = Self::cmd(server)
            .args(["rename-window", "-t", target, new_name])
            .status()
            .context("Failed to rename window")?;
        if !status.success() {
            anyhow::bail!("Failed to rename window to {}", new_name);
        }
        Ok(())
    }

    /// List windows in `session_name` as `Session` structs.
    /// Windows whose ID equals `exclude_window_id` are skipped (used to hide
    /// the ccmux window itself from the list).
    pub fn list_windows_as_sessions(
        server: Option<&str>,
        session_name: &str,
        exclude_window_id: Option<&str>,
    ) -> Result<Vec<crate::session::Session>> {
        use crate::session::{ClaudeCodeStatus, Session};

        let output = Self::cmd(server)
            .args([
                "list-windows",
                "-t", session_name,
                "-F", "#{window_id}\t#{window_name}\t#{window_active}\t#{window_activity}\t#{window_index}",
            ])
            .output()
            .context("Failed to list windows")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("no server running")
                || stderr.contains("can't find session")
                || stderr.contains("no such session")
            {
                return Ok(Vec::new());
            }
            anyhow::bail!("tmux list-windows failed: {}", stderr);
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let server_str = server.map(|s| s.to_string());
        let mut sessions = Vec::new();

        for line in stdout.lines() {
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() < 5 {
                continue;
            }
            let window_id = parts[0].to_string();
            let window_name = parts[1].to_string();
            let active = parts[2] == "1";
            let activity: i64 = parts[3].parse().unwrap_or(0);
            let window_index = parts[4].to_string();

            if exclude_window_id.is_some_and(|excl| excl == window_id) {
                continue;
            }

            let pane_target = format!("{}:{}", session_name, window_index);
            let panes = Self::list_panes_in_window(server, &pane_target).unwrap_or_default();

            // Extract what we need from the detected pane BEFORE moving `panes`
            let detected = panes.iter().find_map(|p| {
                classify_pane_command(&p.current_command).map(|t| {
                    (p.id.clone(), t, p.current_path.clone())
                })
            });

            if let Some((pane_id, pane_type, pane_path)) = detected {
                let status = Self::capture_pane(server, &pane_id, 15, true)
                    .map(|content| detect_status(&content))
                    .unwrap_or(ClaudeCodeStatus::Unknown);

                let git_context = GitContext::detect(&pane_path);

                sessions.push(Session {
                    name: window_name,
                    window_id,
                    created: activity,
                    attached: active,
                    working_directory: pane_path,
                    window_count: 1,
                    panes,
                    claude_code_pane: Some(pane_id),
                    pane_type,
                    claude_code_status: status,
                    window_label: None,
                    target_window_index: Some(window_index),
                    git_context,
                    server: server_str.clone(),
                });
            } else {
                let working_directory = panes.first().map(|p| p.current_path.clone()).unwrap_or_default();
                let git_context = GitContext::detect(&working_directory);

                sessions.push(Session {
                    name: window_name,
                    window_id,
                    created: activity,
                    attached: active,
                    working_directory,
                    window_count: 1,
                    panes,
                    claude_code_pane: None,
                    pane_type: PaneType::Claude,
                    claude_code_status: ClaudeCodeStatus::Unknown,
                    window_label: None,
                    target_window_index: Some(window_index),
                    git_context,
                    server: server_str.clone(),
                });
            }
        }

        Ok(sessions)
    }

    /// List panes in a specific window (`target` = `session:window_index`).
    fn list_panes_in_window(server: Option<&str>, target: &str) -> Result<Vec<Pane>> {
        let output = Self::cmd(server)
            .args([
                "list-panes",
                "-t", target,
                "-F",
                "#{pane_id}\t#{pane_current_command}\t#{pane_current_path}\t#{window_index}\t#{window_name}",
            ])
            .output()
            .context("Failed to list panes")?;

        if !output.status.success() {
            return Ok(Vec::new());
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut panes = Vec::new();

        for line in stdout.lines() {
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() >= 5 {
                panes.push(Pane {
                    id: parts[0].to_string(),
                    current_command: parts[1].to_string(),
                    current_path: PathBuf::from(parts[2]),
                    window_index: parts[3].to_string(),
                    window_name: parts[4].to_string(),
                });
            }
        }

        Ok(panes)
    }

    /// Return the window ID that was last active before the current window
    /// (i.e., where the user was before opening ccmux).
    pub fn last_active_window_id(server: Option<&str>, session_name: &str) -> Option<String> {
        let output = Self::cmd(server)
            .args([
                "list-windows",
                "-t", session_name,
                "-F", "#{window_id}\t#{window_last_flag}",
            ])
            .output()
            .ok()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            let mut parts = line.splitn(2, '\t');
            let id = parts.next()?;
            let flag = parts.next().unwrap_or("0");
            if flag.trim() == "1" {
                return Some(id.to_string());
            }
        }
        None
    }

    /// Return the window ID of the pane ccmux is running in, used to exclude
    /// ccmux's own window from the listed sessions.
    pub fn own_window_id(server: Option<&str>) -> Option<String> {
        let pane_id = std::env::var("TMUX_PANE").ok()?;
        let output = Self::cmd(server)
            .args(["display-message", "-t", &pane_id, "-p", "#{window_id}"])
            .output()
            .ok()?;
        let id = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if id.is_empty() { None } else { Some(id) }
    }

    /// Set the @ccmux_color window variable so `window-status-format` can read it.
    pub fn set_window_color(server: Option<&str>, window_id: &str, tmux_colour: &str) -> Result<()> {
        Self::cmd(server)
            .args(["set-window-option", "-t", window_id, "@ccmux_color", tmux_colour])
            .status()
            .context("Failed to set window color")?;
        Ok(())
    }

    pub fn send_keys(
        server: Option<&str>,
        target: &str,
        keys: &str,
    ) -> Result<()> {
        let status = Self::cmd(server)
            .args(["send-keys", "-t", target, keys, "Enter"])
            .status()
            .context("Failed to send keys")?;
        if !status.success() {
            anyhow::bail!("Failed to send keys to '{}'", target);
        }
        Ok(())
    }
}
