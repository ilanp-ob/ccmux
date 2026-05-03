pub mod detect;
pub mod state;
pub mod windows;

use std::process::Command;
use anyhow::{Context, Result};

pub struct Tmux {
    pub server: Option<String>,
}

impl Tmux {
    pub fn new(server: Option<String>) -> Self {
        Self { server }
    }

    /// Build a tmux Command, optionally targeting a specific server.
    pub fn cmd(&self) -> Command {
        let mut cmd = Command::new("tmux");
        if let Some(s) = &self.server {
            cmd.args(["-L", s]);
        }
        cmd
    }

    /// Return the name of the tmux session we're running inside.
    pub fn current_session(&self) -> Result<Option<String>> {
        let output = self.cmd()
            .args(["display-message", "-p", "#{session_name}"])
            .output()
            .context("tmux display-message failed")?;
        if !output.status.success() {
            return Ok(None);
        }
        let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(if name.is_empty() { None } else { Some(name) })
    }

    /// Return the window_id (@N) of the pane ccmux is running in.
    pub fn own_window_id(&self) -> Option<String> {
        let pane_id = std::env::var("TMUX_PANE").ok()?;
        let output = self.cmd()
            .args(["display-message", "-t", &pane_id, "-p", "#{window_id}"])
            .output().ok()?;
        let id = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if id.is_empty() { None } else { Some(id) }
    }

    /// Return the window_id that was last active before the current one.
    pub fn last_active_window_id(&self, session: &str) -> Option<String> {
        let output = self.cmd()
            .args(["list-windows", "-t", session, "-F",
                   "#{window_id}\t#{window_last_flag}"])
            .output().ok()?;
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

    /// Capture visible content of a pane (last `lines` lines, ANSI stripped if strip=true).
    pub fn capture_pane(&self, pane_id: &str, lines: usize, strip: bool) -> Result<String> {
        let lines_str = format!("-{}", lines);
        let mut args = vec!["capture-pane", "-p", "-t", pane_id, "-S", &lines_str];
        if !strip {
            args.push("-e");
        }
        let output = self.cmd().args(&args).output()
            .context("capture-pane failed")?;
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }
}
