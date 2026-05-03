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

/// Fallback: check the full executable path via `ps` for versioned binaries
/// (e.g. Claude Code installs as "2.1.126" but lives at .../claude/versions/2.1.126).
fn classify_via_ps(pid: &str, configured: &[String]) -> Option<PaneType> {
    if pid.is_empty() { return None; }
    let output = std::process::Command::new("ps")
        .args(["-p", pid, "-o", "command="])
        .output().ok()?;
    let full_cmd = String::from_utf8_lossy(&output.stdout).to_lowercase();
    classify_command_str(&full_cmd, configured)
}

fn classify_command_str(full_cmd: &str, configured: &[String]) -> Option<PaneType> {
    for configured_cmd in configured {
        if full_cmd.contains(&configured_cmd.to_lowercase()) {
            let t = if configured_cmd.to_lowercase().contains("claude") {
                PaneType::Claude
            } else if configured_cmd.to_lowercase().contains("ocli")
                || configured_cmd.to_lowercase().contains("ops-cli") {
                PaneType::Ocli
            } else {
                PaneType::Other(configured_cmd.clone())
            };
            return Some(t);
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
        exclude_window_id: Option<&str>,
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
            let pane_pid    = parts.get(7).unwrap_or(&"").trim().to_string();

            if exclude_window_id.is_some_and(|excl| excl == window_id) {
                continue;
            }

            // First try matching pane_current_command; fall back to full process path via ps
            // (needed for versioned binaries like claude's "2.1.126")
            let pane_type = classify_command(&command, configured_commands)
                .or_else(|| classify_via_ps(&pane_pid, configured_commands));
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
