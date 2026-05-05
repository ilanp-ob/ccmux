use super::Tmux;
use anyhow::Result;

impl Tmux {
    /// Read a global tmux variable (@ccmux_*).
    pub fn get_var(&self, key: &str) -> Option<String> {
        let output = self.cmd()
            .args(["display-message", "-p", &format!("#{{{}}}", key)])
            .output().ok()?;
        let val = String::from_utf8_lossy(&output.stdout).trim().to_string();
        // tmux returns the format string itself if the var is unset
        if val.is_empty() || val == format!("#{{{}}}", key) {
            None
        } else {
            Some(val)
        }
    }

    /// Set a global tmux variable.
    pub fn set_var(&self, key: &str, value: &str) -> Result<()> {
        self.cmd()
            .args(["set-option", "-g", key, value])
            .status()?;
        Ok(())
    }

    /// Delete a global tmux variable.
    pub fn del_var(&self, key: &str) -> Result<()> {
        self.cmd()
            .args(["set-option", "-gu", key])
            .status()?;
        Ok(())
    }

    /// Set a window-scoped tmux variable (set-window-option).
    pub fn set_window_var(&self, window_id: &str, key: &str, value: &str) -> Result<()> {
        self.cmd()
            .args(["set-window-option", "-t", window_id, key, value])
            .status()?;
        Ok(())
    }

    /// Unset a window-scoped tmux variable.
    pub fn del_window_var(&self, window_id: &str, key: &str) -> Result<()> {
        self.cmd()
            .args(["set-window-option", "-ut", window_id, key])
            .status()?;
        Ok(())
    }

    /// Read a window-scoped variable from a specific window.
    pub fn get_window_var(&self, window_id: &str, key: &str) -> Option<String> {
        let output = self.cmd()
            .args(["show-window-options", "-t", window_id, key])
            .output().ok()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        // Output format: "@key value\n"
        stdout.lines()
            .find(|l| l.starts_with(key))
            .and_then(|l| l.split_once(' '))
            .map(|(_, v)| v.trim().to_string())
            .filter(|v| !v.is_empty())
    }
}
