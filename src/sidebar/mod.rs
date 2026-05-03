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
    /// Window ID of the pane ccmux itself is running in (excluded from list)
    pub own_window_id: Option<String>,
    pub error: Option<String>,
    pub message: Option<String>,
    pane_content_cache: HashMap<String, String>,
    last_refresh: Instant,
    last_status_tick: Instant,
}

impl App {
    pub fn new(server: Option<String>, config: Config) -> Result<Self> {
        let tmux = Tmux::new(server.clone());
        let managed_session = tmux.current_session()?.unwrap_or_default();
        let own_window_id = tmux.own_window_id();

        let groups = if managed_session.is_empty() {
            Vec::new()
        } else {
            Self::load_groups(&server, &managed_session, own_window_id.as_deref(), &config)
        };

        let last_active = if !managed_session.is_empty() {
            tmux.last_active_window_id(&managed_session)
        } else {
            None
        };

        let selected = Self::initial_selection(&groups, last_active.as_deref());

        let error = if managed_session.is_empty() {
            Some("Not running inside tmux. Launch ccmux from within a tmux session.".into())
        } else {
            None
        };

        Ok(Self {
            groups,
            selected,
            mode: Mode::Normal,
            should_quit: false,
            config,
            managed_session,
            managed_server: server,
            own_window_id,
            error,
            message: None,
            pane_content_cache: HashMap::new(),
            last_refresh: Instant::now(),
            last_status_tick: Instant::now(),
        })
    }

    fn load_groups(
        server: &Option<String>,
        session: &str,
        exclude_window_id: Option<&str>,
        config: &Config,
    ) -> Vec<WindowGroup> {
        let mut all_groups = Vec::new();

        // Default server
        let tmux = Tmux::new(server.clone());
        if let Ok(groups) = tmux.list_groups(session, exclude_window_id, &config.detection.commands) {
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

    fn initial_selection(groups: &[WindowGroup], last_window_id: Option<&str>) -> usize {
        let panes = Self::flat_panes_ref(groups);
        if let Some(id) = last_window_id {
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
        }
    }

    pub fn select_prev(&mut self) {
        let count = self.flat_panes().len();
        if count > 0 {
            self.selected = if self.selected == 0 { count - 1 } else { self.selected - 1 };
        }
    }

    pub fn select_by_display_num(&mut self, n: usize) {
        if let Some(idx) = self.flat_panes().iter().position(|p| p.display_num == n) {
            self.selected = idx;
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
            self.own_window_id.as_deref(),
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

    /// Switch tmux focus to the selected pane.
    pub fn focus_selected(&mut self) {
        let Some(pane) = self.selected_pane() else { return };
        let pane_id = pane.pane_id.clone();
        let window_id = pane.window_id.clone();
        let tmux = Tmux::new(self.managed_server.clone());

        // Switch window first if pane is in a different window
        let own_window = self.own_window_id.as_deref().unwrap_or("");
        if window_id != own_window {
            let _ = tmux.select_window(&window_id);
        } else {
            let _ = tmux.select_pane(&pane_id);
        }
    }
}
