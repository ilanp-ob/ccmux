use std::path::PathBuf;
use anyhow::{Context, Result};
use crate::session::{ClaudeCodeStatus, DetectedPane, PaneType, WindowGroup};
use super::Tmux;

/// Return the PaneType if this command should be tracked, else None.
pub fn classify_command(cmd: &str, configured: &[String]) -> Option<PaneType> {
    let lower = cmd.to_lowercase();
    for configured_cmd in configured {
        if lower.contains(&configured_cmd.to_lowercase()) {
            let t = if configured_cmd.contains("claude") {
                PaneType::Claude
            } else if configured_cmd.contains("ocli") || configured_cmd.contains("ops-cli") {
                PaneType::Ocli
            } else {
                PaneType::Other(configured_cmd.clone())
            };
            return Some(t);
        }
    }
    None
}

fn is_idle_shell(cmd: &str) -> bool {
    let name = cmd.rsplit('/').next().unwrap_or(cmd);
    matches!(name.to_lowercase().as_str(),
        "zsh" | "bash" | "fish" | "sh" | "dash" | "csh" | "tcsh" | "nu")
}

/// Walk children of root_pid for the first non-shell process (foreground or background).
/// Returns (child_pid, comm) so the caller can optionally fetch full args.
fn find_foreground_command(
    root_pid: u32,
    tree: &std::collections::HashMap<u32, Vec<(u32, String)>>,
    depth: u8,
) -> Option<(u32, String)> {
    if depth > 6 { return None; }
    for (child_pid, comm) in tree.get(&root_pid)? {
        if !is_idle_shell(comm) {
            return Some((*child_pid, comm.clone()));
        }
        if let Some(found) = find_foreground_command(*child_pid, tree, depth + 1) {
            return Some(found);
        }
    }
    None
}

/// Get full command-line arguments for a single PID (fast — only scans one process).
fn get_full_args(pid: u32) -> Option<String> {
    let out = std::process::Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "args="])
        .output().ok()?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() { None } else { Some(s) }
}

/// Build a pid→[(child_pid, comm)] map from the full process table.
/// Called once per list_groups invocation, shared across all panes.
fn build_process_tree() -> std::collections::HashMap<u32, Vec<(u32, String)>> {
    let mut map: std::collections::HashMap<u32, Vec<(u32, String)>> =
        std::collections::HashMap::new();
    let Ok(output) = std::process::Command::new("ps")
        .args(["-eo", "pid,ppid,comm"])
        .output()
    else { return map; };
    for line in String::from_utf8_lossy(&output.stdout).lines().skip(1) {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 3 { continue; }
        let (Ok(pid), Ok(ppid)) = (parts[0].parse::<u32>(), parts[1].parse::<u32>()) else { continue };
        map.entry(ppid).or_default().push((pid, parts[2].to_string()));
    }
    map
}

/// Walk descendants of root_pid looking for a comm that matches configured commands.
/// Uses comm (set by the process itself), so it finds "claude" even when the binary
/// is a versioned file like "2.1.126".
fn classify_descendant(
    root_pid: u32,
    tree: &std::collections::HashMap<u32, Vec<(u32, String)>>,
    configured: &[String],
    depth: u8,
) -> Option<PaneType> {
    if depth > 8 { return None; }
    for (child_pid, comm) in tree.get(&root_pid)? {
        if let Some(pt) = classify_command(comm, configured) {
            return Some(pt);
        }
        if let Some(pt) = classify_descendant(*child_pid, tree, configured, depth + 1) {
            return Some(pt);
        }
    }
    None
}

impl Tmux {
    /// Scan all panes in the tmux session, filter to detected commands,
    /// group by window, and assign sequential display numbers.
    pub fn list_groups(
        &self,
        session: &str,
        exclude_pane_id: Option<&str>,
        configured_commands: &[String],
    ) -> Result<Vec<WindowGroup>> {
        let output = self.cmd()
            .args([
                "list-panes", "-s", "-t", session,
                "-F",
                "#{pane_id}\t#{pane_current_command}\t#{window_id}\t#{window_name}\t#{window_index}\t#{pane_active}\t#{pane_current_path}\t#{pane_pid}\t#{@ccmux_color}\t#{@ccmux_name}",
            ])
            .output()
            .context("tmux list-panes failed")?;

        if !output.status.success() {
            return Ok(Vec::new());
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut groups: Vec<WindowGroup> = Vec::new();
        let mut display_num = 1usize;
        // Collect non-Claude panes per window for display in the sidebar.
        let mut extra_map: std::collections::HashMap<String, Vec<crate::session::ExtraPane>> =
            std::collections::HashMap::new();

        // Build process tree once for all panes — used to find versioned binaries
        // like claude's "2.1.126" whose comm name is still "claude".
        let proc_tree = build_process_tree();

        for line in stdout.lines() {
            let parts: Vec<&str> = line.splitn(10, '\t').collect();
            if parts.len() < 7 { continue; }

            let pane_id     = parts[0].to_string();
            let command     = parts[1].to_string();
            let window_id   = parts[2].to_string();
            let window_name = parts[3].to_string();
            // Use @ccmux_name as the authoritative display name — it's a user-defined
            // tmux variable that automatic-rename never touches. Snapshot window_name
            // on first encounter so it stays stable even if tmux auto-renames the window.
            let stored_name = parts.get(9)
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            let display_name = match stored_name {
                Some(n) => n,
                None => {
                    let _ = self.cmd()
                        .args(["set-window-option", "-t", &parts[2], "@ccmux_name", &window_name])
                        .output();
                    window_name.clone()
                }
            };
            let window_index = parts[4].to_string();
            let pane_active = parts[5] == "1";
            let current_path = PathBuf::from(parts[6]);
            let pane_pid: u32 = parts.get(7).and_then(|s| s.trim().parse().ok()).unwrap_or(0);
            let window_color: Option<String> = parts.get(8)
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());

            // Skip only the sidebar pane itself, not the whole window
            if exclude_pane_id.is_some_and(|excl| excl == pane_id) {
                continue;
            }

            // Try pane_current_command first; fall back to walking process descendants
            // by comm name — version-proof (works regardless of binary filename).
            let pane_type = classify_command(&command, configured_commands)
                .or_else(|| classify_descendant(pane_pid, &proc_tree, configured_commands, 0));

            let Some(pane_type) = pane_type else {
                // Not a tracked command. Record as an extra pane unless it's another
                // ccmux sidebar (auto-opened by ensure_sidebar_in_window in other windows).
                if !command.to_lowercase().contains("ccmux") {
                    let effective_cmd = if is_idle_shell(&command) {
                        // Bare shell: descend process tree to find what's actually running.
                        match find_foreground_command(pane_pid, &proc_tree, 0) {
                            Some((child_pid, child_comm)) =>
                                get_full_args(child_pid).unwrap_or(child_comm),
                            None => command,
                        }
                    } else {
                        // Known foreground command (e.g. "ol"): look for it as a direct
                        // child of the shell to get full args ("ol start"). If it's a
                        // script wrapper the direct child comm won't match, so we fall
                        // back to the tmux-reported name rather than surfacing a deeply
                        // nested subprocess (e.g. "npm exec tsx --watch").
                        let cmd_base = command.rsplit('/').next().unwrap_or(&command).to_lowercase();
                        proc_tree.get(&pane_pid)
                            .and_then(|children| children.iter()
                                .find(|(_, comm)| comm.to_lowercase() == cmd_base)
                                .map(|(pid, _)| *pid))
                            .and_then(get_full_args)
                            .unwrap_or(command)
                    };
                    extra_map.entry(window_id).or_default()
                        .push(crate::session::ExtraPane { command: effective_cmd, path: current_path });
                }
                continue;
            };

            // Only list Claude Code sessions
            if !matches!(pane_type, PaneType::Claude) {
                extra_map.entry(window_id.clone()).or_default()
                    .push(crate::session::ExtraPane { command, path: current_path });
                continue;
            }

            // Status is filled in by tick_status() — skip capture_pane here so
            // refresh() stays fast and never blocks the event loop for multiple panes.
            let status = ClaudeCodeStatus::Unknown;

            let pane = DetectedPane {
                pane_id,
                window_id: window_id.clone(),
                window_name: display_name.clone(),
                window_index,
                pane_active,
                current_command: command,
                current_path,
                pane_type,
                status,
                server: self.server.clone(),
                display_num,
            };

            display_num += 1;

            if let Some(group) = groups.iter_mut().find(|g| g.window_id == window_id) {
                group.panes.push(pane);
            } else {
                // Lock the name the first time we see this window so that tmux
                // automatic-rename can't overwrite it with a non-Claude pane's command
                // (e.g. "cargo" or "zsh") when the user has multiple panes open.
                let _ = self.cmd()
                    .args(["set-window-option", "-t", &window_id, "automatic-rename", "off"])
                    .output();
                groups.push(WindowGroup {
                    window_id,
                    window_name: display_name,
                    server: self.server.clone(),
                    panes: vec![pane],
                    extra_panes: Vec::new(),
                    color_name: window_color,
                });
            }
        }

        // Attach extra panes collected above.
        for group in &mut groups {
            if let Some(extras) = extra_map.remove(&group.window_id) {
                group.extra_panes = extras;
            }
        }

        Ok(groups)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmds(v: &[&str]) -> Vec<String> { v.iter().map(|s| s.to_string()).collect() }

    #[test]
    fn classify_claude() {
        assert_eq!(classify_command("claude", &cmds(&["claude"])), Some(PaneType::Claude));
    }

    #[test]
    fn classify_ocli() {
        assert_eq!(classify_command("ocli", &cmds(&["claude", "ocli"])), Some(PaneType::Ocli));
    }

    #[test]
    fn classify_unknown_command_not_in_list() {
        assert_eq!(classify_command("vim", &cmds(&["claude"])), None);
    }

    #[test]
    fn classify_is_case_insensitive() {
        assert_eq!(classify_command("Claude", &cmds(&["claude"])), Some(PaneType::Claude));
    }
}
