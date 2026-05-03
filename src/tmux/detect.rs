use std::path::PathBuf;
use anyhow::{Context, Result};
use crate::session::{ClaudeCodeStatus, DetectedPane, PaneType, WindowGroup};
use crate::detection::detect_status;
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
                "#{pane_id}\t#{pane_current_command}\t#{window_id}\t#{window_name}\t#{window_index}\t#{pane_active}\t#{pane_current_path}\t#{pane_pid}",
            ])
            .output()
            .context("tmux list-panes failed")?;

        if !output.status.success() {
            return Ok(Vec::new());
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut groups: Vec<WindowGroup> = Vec::new();
        let mut display_num = 1usize;

        // Build process tree once for all panes — used to find versioned binaries
        // like claude's "2.1.126" whose comm name is still "claude".
        let proc_tree = build_process_tree();

        for line in stdout.lines() {
            let parts: Vec<&str> = line.splitn(8, '\t').collect();
            if parts.len() < 7 { continue; }

            let pane_id     = parts[0].to_string();
            let command     = parts[1].to_string();
            let window_id   = parts[2].to_string();
            let window_name = parts[3].to_string();
            let window_index = parts[4].to_string();
            let pane_active = parts[5] == "1";
            let current_path = PathBuf::from(parts[6]);
            let pane_pid: u32 = parts.get(7).and_then(|s| s.trim().parse().ok()).unwrap_or(0);

            // Skip only the sidebar pane itself, not the whole window
            if exclude_pane_id.is_some_and(|excl| excl == pane_id) {
                continue;
            }

            // Try pane_current_command first; fall back to walking process descendants
            // by comm name — version-proof (works regardless of binary filename).
            let pane_type = classify_command(&command, configured_commands)
                .or_else(|| classify_descendant(pane_pid, &proc_tree, configured_commands, 0));
            let Some(pane_type) = pane_type else {
                continue;
            };

            let status = self.capture_pane(&pane_id, 30, true)
                .map(|c| detect_status(&c))
                .unwrap_or(ClaudeCodeStatus::Unknown);

            let pane = DetectedPane {
                pane_id,
                window_id: window_id.clone(),
                window_name: window_name.clone(),
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
                groups.push(WindowGroup {
                    window_id,
                    window_name,
                    server: self.server.clone(),
                    panes: vec![pane],
                });
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
