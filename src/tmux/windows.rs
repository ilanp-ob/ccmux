use std::path::Path;
use anyhow::{Context, Result};
use super::Tmux;

impl Tmux {
    /// Create a new window in session, cd to path, return window_id.
    pub fn new_window(
        &self,
        session: &str,
        window_name: &str,
        path: &Path,
    ) -> Result<String> {
        let path_str = path.to_string_lossy();
        let output = self.cmd()
            .args([
                "new-window", "-t", session, "-n", window_name,
                "-c", path_str.as_ref(),
                "-P", "-F", "#{window_id}",
            ])
            .output()
            .context("tmux new-window failed")?;
        let window_id = String::from_utf8_lossy(&output.stdout).trim().to_string();
        // Explicit cd to handle any working-dir race
        let cd_cmd = format!("cd '{}'", path_str.replace('\'', "'\\''"));
        let _ = self.cmd()
            .args(["send-keys", "-t", &window_id, &cd_cmd, "Enter"])
            .status();
        Ok(window_id)
    }

    /// Split current window horizontally to create the sidebar pane.
    /// Returns the new pane_id.
    pub fn split_sidebar(&self, session: &str, width: u16, cmd: &str) -> Result<String> {
        let width_str = width.to_string();
        let output = self.cmd()
            .args([
                "split-window", "-hb",
                "-l", &width_str,
                "-t", session,
                "-P", "-F", "#{pane_id}",
                cmd,
            ])
            .output()
            .context("tmux split-window failed")?;
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    /// Kill a pane by its pane_id.
    pub fn kill_pane(&self, pane_id: &str) -> Result<()> {
        self.cmd()
            .args(["kill-pane", "-t", pane_id])
            .status()
            .context("tmux kill-pane failed")?;
        Ok(())
    }

    /// Kill a window by window_id.
    pub fn kill_window(&self, window_id: &str) -> Result<()> {
        self.cmd()
            .args(["kill-window", "-t", window_id])
            .status()
            .context("tmux kill-window failed")?;
        Ok(())
    }

    /// Rename a window.
    pub fn rename_window(&self, window_id: &str, new_name: &str) -> Result<()> {
        self.cmd()
            .args(["rename-window", "-t", window_id, new_name])
            .status()
            .context("tmux rename-window failed")?;
        Ok(())
    }

    /// Send keys to a pane (appends Enter).
    pub fn send_keys(&self, target: &str, keys: &str) -> Result<()> {
        self.cmd()
            .args(["send-keys", "-t", target, keys, "Enter"])
            .status()
            .context("tmux send-keys failed")?;
        Ok(())
    }

    /// Set @ccmux_color on a window for status-bar coloring.
    pub fn set_window_color(&self, window_id: &str, tmux_colour: &str) -> Result<()> {
        self.set_window_var(window_id, "@ccmux_color", tmux_colour)
    }

    /// Focus a specific pane (select-pane).
    pub fn select_pane(&self, pane_id: &str) -> Result<()> {
        self.cmd()
            .args(["select-pane", "-t", pane_id])
            .status()
            .context("tmux select-pane failed")?;
        Ok(())
    }

    /// Switch to a window (select-window).
    pub fn select_window(&self, window_id: &str) -> Result<()> {
        self.cmd()
            .args(["select-window", "-t", window_id])
            .status()
            .context("tmux select-window failed")?;
        Ok(())
    }

    /// Check whether a pane is still alive.
    pub fn pane_exists(&self, pane_id: &str) -> bool {
        self.cmd()
            .args(["list-panes", "-t", pane_id])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
}
