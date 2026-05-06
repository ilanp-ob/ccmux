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

    /// Split a specific window horizontally to create the sidebar pane.
    /// Targets the leftmost pane in that window so -hb always places the
    /// sidebar at the left edge regardless of which pane has focus.
    /// Returns the new pane_id.
    pub fn split_sidebar(&self, window_id: &str, width: u16, cmd: &str) -> Result<String> {
        let width_str = width.to_string();
        let target = self.leftmost_pane_in_window(window_id)
            .unwrap_or_else(|| window_id.to_string());
        let output = self.cmd()
            .args([
                "split-window", "-hb",
                "-l", &width_str,
                "-t", &target,
                "-P", "-F", "#{pane_id}",
                cmd,
            ])
            .output()
            .context("tmux split-window failed")?;
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    /// Return the pane_id of the leftmost pane in `window_id`.
    pub fn leftmost_pane_in_window(&self, window_id: &str) -> Option<String> {
        let out = self.cmd()
            .args(["list-panes", "-t", window_id, "-F", "#{pane_id} #{pane_left}"])
            .output().ok()?;
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|line| {
                let mut parts = line.split_whitespace();
                let id = parts.next()?.to_string();
                let left: u32 = parts.next()?.parse().ok()?;
                Some((left, id))
            })
            .min_by_key(|(left, _)| *left)
            .map(|(_, id)| id)
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

    /// Set @ccmux_color on a window and sync window-status-style so the
    /// tmux status bar reflects the chosen color.
    pub fn set_window_color(&self, window_id: &str, tmux_colour: &str) -> Result<()> {
        self.set_window_var(window_id, "@ccmux_color", tmux_colour)?;
        if tmux_colour.is_empty() {
            let _ = self.cmd()
                .args(["set-window-option", "-ut", window_id, "window-status-style"])
                .status();
        } else {
            let _ = self.set_window_var(
                window_id, "window-status-style",
                &format!("fg={}", tmux_colour),
            );
        }
        Ok(())
    }

    /// Apply window-status-style to every window in the list that has a color.
    /// Called once at startup to sync existing @ccmux_color values.
    pub fn sync_status_styles(&self, windows: &[(&str, &str)]) {
        for (window_id, colour) in windows {
            if colour.is_empty() { continue; }
            let _ = self.set_window_var(
                window_id, "window-status-style",
                &format!("fg={}", colour),
            );
        }
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
            .output()                       // captures stdout so tmux doesn't echo it to the window
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
}
