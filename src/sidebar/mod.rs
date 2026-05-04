pub mod input;
pub mod mode;
pub mod render;

pub use mode::Mode;

use std::collections::HashMap;
use std::time::{Duration, Instant};
use anyhow::Result;

use crate::config::Config;
use crate::detection::{detect_static_status, detect_status};
use crate::session::{ClaudeCodeStatus, DetectedPane, WindowGroup};
use crate::tmux::Tmux;

pub struct App {
    /// All detected pane groups (session-wide, all servers)
    pub groups: Vec<WindowGroup>,
    /// Flat index into all panes across all groups (for navigation)
    pub selected: usize,
    pub mode: Mode,
    pub should_quit: bool,
    pub config: Config,
    pub managed_session: String,
    pub managed_server: Option<String>,
    /// Window ID of the pane ccmux itself is running in (used for focus logic)
    pub own_window_id: Option<String>,
    /// Pane ID of the sidebar itself (excluded from session list)
    own_pane_id: Option<String>,
    pub error: Option<String>,
    pub message: Option<String>,
    pub pane_content_cache: HashMap<String, String>,
    last_refresh: Instant,
    last_status_tick: Instant,
    last_focus_tick: Instant,
    /// Maps (terminal_row, flat_pane_idx) for mouse click hit-testing. Updated each render.
    pub pane_click_rows: Vec<(u16, usize)>,
    /// Scroll offset in items (not rows).
    pub scroll_offset: usize,
    /// Whether the sidebar pane currently has tmux focus.
    pub is_focused: bool,
    /// Auto-open sidebar when switching to a window with Claude sessions.
    pub sticky: bool,
    /// Flat index of the item last previewed with Enter (first press). Second Enter on the
    /// same index commits focus to the Claude pane; changing selection clears this.
    pub last_entered_idx: Option<usize>,
}

impl App {
    pub fn new(server: Option<String>, config: Config) -> Result<Self> {
        let tmux = Tmux::new(server.clone());
        let managed_session = tmux.current_session()?.unwrap_or_default();
        let own_window_id = tmux.own_window_id();
        let own_pane_id = std::env::var("TMUX_PANE").ok();

        let groups = if managed_session.is_empty() {
            Vec::new()
        } else {
            Self::load_groups(&server, &managed_session, own_pane_id.as_deref(), &config)
        };

        // Start with the Claude pane in the same window as the sidebar selected,
        // so opening from a Claude window immediately highlights that session.
        let selected = Self::initial_selection(&groups, own_window_id.as_deref());

        let error = if managed_session.is_empty() {
            Some("Not running inside tmux. Launch ccmux from within a tmux session.".into())
        } else {
            None
        };
        let sticky = config.sidebar.sticky;

        Ok(Self {
            groups,
            selected,
            mode: Mode::Normal,
            should_quit: false,
            config,
            managed_session,
            managed_server: server,
            own_window_id,
            own_pane_id,
            error,
            message: None,
            pane_content_cache: HashMap::new(),
            last_refresh: Instant::now(),
            last_status_tick: Instant::now(),
            last_focus_tick: Instant::now(),
            pane_click_rows: Vec::new(),
            scroll_offset: 0,
            is_focused: true,
            sticky,
            last_entered_idx: None,
        })
    }

    fn load_groups(
        server: &Option<String>,
        session: &str,
        exclude_pane_id: Option<&str>,
        config: &Config,
    ) -> Vec<WindowGroup> {
        let mut all_groups = Vec::new();

        // Default server
        let tmux = Tmux::new(server.clone());
        if let Ok(groups) = tmux.list_groups(session, exclude_pane_id, &config.detection.commands) {
            all_groups.extend(groups);
        }

        // Extra servers (may have different sessions — scan all their sessions)
        for extra in &config.servers.extra {
            let extra_tmux = Tmux::new(Some(extra.clone()));
            if let Ok(output) = extra_tmux.cmd()
                .args(["list-sessions", "-F", "#{session_name}"])
                .output()
            {
                for sess in String::from_utf8_lossy(&output.stdout).lines() {
                    if let Ok(groups) = extra_tmux.list_groups(sess, None, &config.detection.commands) {
                        all_groups.extend(groups);
                    }
                }
            }
        }

        all_groups
    }

    fn initial_selection(groups: &[WindowGroup], own_window_id: Option<&str>) -> usize {
        let panes = Self::flat_panes_ref(groups);
        // Prefer the Claude pane that lives in the same window as the sidebar itself.
        if let Some(id) = own_window_id {
            if let Some(idx) = panes.iter().position(|p| p.window_id == id) {
                return idx;
            }
        }
        0
    }

    /// Flatten all panes from all groups into a single ordered slice.
    pub fn flat_panes_ref(groups: &[WindowGroup]) -> Vec<&DetectedPane> {
        groups.iter().flat_map(|g| g.panes.iter()).collect()
    }

    pub fn flat_panes(&self) -> Vec<&DetectedPane> {
        Self::flat_panes_ref(&self.groups)
    }

    pub fn selected_pane(&self) -> Option<&DetectedPane> {
        self.flat_panes().get(self.selected).copied()
    }

    pub fn select_next(&mut self) {
        let count = self.flat_panes().len();
        if count > 0 {
            self.selected = (self.selected + 1) % count;
            self.last_entered_idx = None;
        }
    }

    pub fn select_prev(&mut self) {
        let count = self.flat_panes().len();
        if count > 0 {
            self.selected = if self.selected == 0 { count - 1 } else { self.selected - 1 };
            self.last_entered_idx = None;
        }
    }

    pub fn select_by_display_num(&mut self, n: usize) {
        if let Some(idx) = self.flat_panes().iter().position(|p| p.display_num == n) {
            self.selected = idx;
            self.last_entered_idx = None;
        }
    }

    pub fn clear_messages(&mut self) {
        self.error = None;
        self.message = None;
    }

    /// Refresh groups from tmux. Returns true if anything changed.
    pub fn refresh(&mut self) -> bool {
        if self.last_refresh.elapsed() < Duration::from_millis(self.config.sidebar.refresh_ms) {
            return false;
        }
        self.last_refresh = Instant::now();

        let new_groups = Self::load_groups(
            &self.managed_server,
            &self.managed_session,
            self.own_pane_id.as_deref(),
            &self.config,
        );

        let changed = format!("{:?}", new_groups) != format!("{:?}", self.groups);
        self.groups = new_groups;

        // Clamp selection
        let count = self.flat_panes().len();
        if count > 0 && self.selected >= count {
            self.selected = count - 1;
        }

        changed
    }

    /// Update Claude status via content-change detection. Returns true if any status changed.
    pub fn tick_status(&mut self) -> bool {
        if self.last_status_tick.elapsed() < Duration::from_millis(500) {
            return false;
        }
        self.last_status_tick = Instant::now();

        let tmux = Tmux::new(self.managed_server.clone());

        // Collect pane IDs first to avoid borrow conflicts
        let pane_ids: Vec<String> = self.groups.iter()
            .flat_map(|g| g.panes.iter())
            .map(|p| p.pane_id.clone())
            .collect();

        let mut updates: Vec<(String, ClaudeCodeStatus, String)> = Vec::new();

        for pane_id in &pane_ids {
            let content = match tmux.capture_pane(pane_id, 30, true) {
                Ok(c) => c,
                Err(_) => continue,
            };

            let new_status = if let Some(prev) = self.pane_content_cache.get(pane_id) {
                if &content != prev {
                    ClaudeCodeStatus::Working
                } else {
                    detect_static_status(&content)
                }
            } else {
                detect_status(&content)
            };

            updates.push((pane_id.clone(), new_status, content));
        }

        let mut changed = false;
        for (pane_id, new_status, content) in updates {
            for group in &mut self.groups {
                for pane in &mut group.panes {
                    if pane.pane_id == pane_id && new_status != pane.status {
                        pane.status = new_status.clone();
                        changed = true;
                    }
                }
            }
            self.pane_content_cache.insert(pane_id, content);
        }

        changed
    }

    /// Switch tmux focus to the selected Claude pane, auto-opening a sidebar there if needed.
    pub fn focus_selected(&mut self) {
        let Some(pane) = self.selected_pane() else { return };
        let pane_id = pane.pane_id.clone();
        let window_id = pane.window_id.clone();
        let tmux = Tmux::new(self.managed_server.clone());

        let own_window = self.own_window_id.as_deref().unwrap_or("");
        if window_id != own_window {
            let _ = tmux.select_window(&window_id);
            // Auto-open a sidebar in the target window so the user sees it immediately.
            self.ensure_sidebar_in_window(&window_id, &pane_id);
        }
        // Always land on the Claude pane (split-window steals focus, so this re-focuses it).
        let _ = tmux.select_pane(&pane_id);
    }

    /// Spawn a sidebar in `window_id` if one isn't already there.
    /// Re-selects `claude_pane_id` afterwards since split-window steals focus.
    fn ensure_sidebar_in_window(&self, window_id: &str, claude_pane_id: &str) {
        let tmux = Tmux::new(self.managed_server.clone());
        let var_key = format!("@ccmux_sidebar_{}_{}", self.managed_session, window_id);

        // Already open?
        if let Some(pane_id) = tmux.get_var(&var_key) {
            let alive = tmux.cmd()
                .args(["list-panes", "-s", "-t", &self.managed_session, "-F", "#{pane_id}"])
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).lines().any(|l| l.trim() == pane_id))
                .unwrap_or(false);
            if alive { return; }
            let _ = tmux.del_var(&var_key);
        }

        let binary = std::env::current_exe()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| "ccmux".to_string());
        let sidebar_cmd = match &self.managed_server {
            Some(s) => format!("{} sidebar --server {}", binary, s),
            None => format!("{} sidebar", binary),
        };
        let width = self.config.sidebar.width.to_string();

        let output = tmux.cmd()
            .args([
                "split-window", "-hb",
                "-l", &width,
                "-t", window_id,
                "-P", "-F", "#{pane_id}",
                &sidebar_cmd,
            ])
            .output();

        if let Ok(out) = output {
            let new_pane = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !new_pane.is_empty() {
                let _ = tmux.set_var(&var_key, &new_pane);
                // Return focus to the Claude pane — split-window moved it to the new sidebar.
                let _ = tmux.select_pane(claude_pane_id);
            }
        }
    }

    /// First-Enter: switch to the selected pane's window so the user can preview it,
    /// but keep focus on the sidebar. Does nothing if already in the same window.
    pub fn preview_selected(&mut self) {
        let Some(pane) = self.selected_pane() else { return };
        let window_id = pane.window_id.clone();
        let tmux = Tmux::new(self.managed_server.clone());
        let own_window = self.own_window_id.as_deref().unwrap_or("");
        if window_id != own_window {
            let _ = tmux.select_window(&window_id);
        }
        // Don't select_pane — sidebar stays focused.
    }

    /// Toggle sticky mode and persist in tmux global var.
    pub fn toggle_sticky(&mut self) {
        self.sticky = !self.sticky;
        let tmux = Tmux::new(self.managed_server.clone());
        let val = if self.sticky { "1" } else { "0" };
        let _ = tmux.set_var("@ccmux_sticky", val);
        self.message = Some(if self.sticky { "Sticky on" } else { "Sticky off" }.into());
    }

    /// Send `text` followed by Enter to the selected Claude pane.
    pub fn send_message(&mut self, text: &str) {
        let Some(pane) = self.selected_pane() else { return };
        let pane_id = pane.pane_id.clone();
        let tmux = Tmux::new(self.managed_server.clone());
        match tmux.send_keys(&pane_id, text) {
            Ok(_) => self.message = Some("Sent".into()),
            Err(e) => self.error = Some(format!("Send failed: {}", e)),
        }
    }

    /// Poll whether the sidebar pane is still the active tmux pane. Returns true if changed.
    pub fn tick_focus(&mut self) -> bool {
        if self.last_focus_tick.elapsed() < Duration::from_millis(300) {
            return false;
        }
        self.last_focus_tick = Instant::now();
        let Some(own) = self.own_pane_id.clone() else { return false };
        let tmux = Tmux::new(self.managed_server.clone());
        let active = tmux.cmd()
            .args(["display-message", "-t", &own, "-p", "#{pane_active}"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "1")
            .unwrap_or(true);
        if active != self.is_focused {
            self.is_focused = active;
            true
        } else {
            false
        }
    }
}
