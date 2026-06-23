pub mod input;
pub mod mode;
pub mod render;
pub mod hostmem;

pub use mode::Mode;

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};
use anyhow::Result;

use crate::config::Config;
use crate::detection::{detect_changed_status, detect_static_status, detect_status};
use crate::jobs::{JobEntry, load_jobs, reply_to_job};
use crate::session::{ClaudeCodeStatus, DetectedPane, WindowGroup};
use crate::tmux::Tmux;

/// Global stats shared across all Claude Code sessions, read from statusline cache files.
#[derive(Default, Clone)]
pub struct GlobalInfo {
    pub usage_5h: Option<f32>,
    pub usage_7d: Option<f32>,
    /// Unix epoch secs — formatted at render time so display is always current.
    pub reset_5h_at: Option<i64>,
    pub reset_7d_at: Option<i64>,
    pub usage_updated_at: Option<i64>,
    pub mp_drawers: Option<String>,
    pub mp_size: Option<String>,
    pub mp_wings: Option<u32>,
    pub mp_rooms: Option<u32>,
    pub mp_last_at: Option<i64>,
}

impl GlobalInfo {
    pub fn has_data(&self) -> bool {
        self.usage_5h.is_some() || self.mp_drawers.is_some()
    }

    /// Read from ~/.cache/cc-usage.json and ~/.cache/cc-mempalace.json (written by statusline).
    pub fn load() -> Self {
        let mut info = GlobalInfo::default();
        let home = std::env::var("HOME").unwrap_or_else(|_| String::from("~"));
        let cache = std::path::Path::new(&home).join(".cache");

        let usage_path = cache.join("cc-usage.json");
        if let Ok(s) = std::fs::read_to_string(&usage_path) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&s) {
                info.usage_5h = v["five_hour"]["utilization"].as_f64().map(|x| x as f32);
                info.usage_7d = v["seven_day"]["utilization"].as_f64().map(|x| x as f32);
                info.reset_5h_at = v["five_hour"]["resets_at"].as_str().and_then(utc_to_epoch);
                info.reset_7d_at = v["seven_day"]["resets_at"].as_str().and_then(utc_to_epoch);
            }
            info.usage_updated_at = std::fs::metadata(&usage_path)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64);
        }

        if let Ok(s) = std::fs::read_to_string(cache.join("cc-mempalace.json")) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&s) {
                if let Some(n) = v["drawers"].as_u64() {
                    info.mp_drawers = Some(if n >= 1000 {
                        format!("{:.1}K", n as f64 / 1000.0)
                    } else {
                        n.to_string()
                    });
                }
                info.mp_size = v["size"].as_str().map(String::from);
                info.mp_wings = v["wings"].as_u64().map(|x| x as u32);
                info.mp_rooms = v["rooms"].as_u64().map(|x| x as u32);
                info.mp_last_at = v["last"].as_str().and_then(utc_to_epoch);
            }
        }

        info
    }
}

/// Parse "YYYY-MM-DDTHH:MM:SS..." (UTC) to Unix epoch seconds using Julian Day formula.
pub(super) fn utc_to_epoch(ts: &str) -> Option<i64> {
    let b = ts.as_bytes();
    if b.len() < 19 { return None; }
    let s = std::str::from_utf8(&b[..19]).ok()?;
    let y: i64 = s[0..4].parse().ok()?;
    let mo: i64 = s[5..7].parse().ok()?;
    let d: i64 = s[8..10].parse().ok()?;
    let h: i64 = s[11..13].parse().ok()?;
    let mi: i64 = s[14..16].parse().ok()?;
    let sec: i64 = s[17..19].parse().ok()?;
    let (y, mo) = if mo <= 2 { (y - 1, mo + 12) } else { (y, mo) };
    let a = y / 100;
    let jdn = (365.25_f64 * (y + 4716) as f64) as i64
            + (30.6001_f64 * (mo + 1) as f64) as i64
            + d + (2 - a + a / 4) - 1524;
    Some((jdn - 2_440_588) * 86400 + h * 3600 + mi * 60 + sec)
}

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
    pub own_pane_id: Option<String>,
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
    /// Background branch-fetch thread for the worktree flow.
    pub fetch_handle: Option<std::thread::JoinHandle<anyhow::Result<Vec<crate::git::BranchEntry>>>>,
    pub fetch_repo_root: Option<String>,
    /// Background dir-scan thread for the folder picker.
    pub folder_scan_handle: Option<std::thread::JoinHandle<Vec<std::path::PathBuf>>>,
    pub folder_scan_root: Option<std::path::PathBuf>,
    /// Background history-scan thread for the history browser.
    pub history_handle: Option<std::thread::JoinHandle<Vec<crate::history::SessionEntry>>>,
    pub history_repo_root: Option<String>,
    /// Background `git status` thread for the selected repo.
    pub git_handle: Option<std::thread::JoinHandle<Option<crate::gitstatus::GitStatus>>>,
    /// Repo path the in-flight git_handle is computing.
    pub git_handle_path: Option<std::path::PathBuf>,
    /// Latest git status for the selected repo (None = unknown / not a repo).
    pub gitstatus: Option<crate::gitstatus::GitStatus>,
    /// Repo path that `gitstatus` belongs to.
    pub gitstatus_path: Option<std::path::PathBuf>,
    last_gitstatus_tick: std::time::Instant,
    last_nav_hint_tick: Instant,
    pub global_info: GlobalInfo,
    last_global_info_tick: Instant,
    /// Window IDs that have a pending @ccmux_alert (notified but not yet focused).
    pub alerted_windows: HashSet<String>,
    last_alerts_tick: Instant,
    /// True after the first tick_alerts reconciliation pass clears stale flags from prior runs.
    alerts_reconciled: bool,
    /// When the current `message` was set — used to auto-clear it after ~3 s.
    message_shown_at: Option<Instant>,
    /// ccmux's own CPU% and RSS — polled every 5 s and shown in the footer.
    pub own_cpu_pct: f32,
    pub own_rss_mb: f32,
    /// Rolling window of CPU samples (≤30 entries, 5 s apart = ~2.5 min history).
    pub own_cpu_history: std::collections::VecDeque<f32>,
    last_own_metrics_tick: Instant,
    /// Host terminal app (iTerm2, Terminal, …) — detected once, re-checked ~30 s.
    pub host_app: Option<hostmem::HostApp>,
    last_host_detect: Instant,
    /// Host app resident memory (MB) and system-wide swap used (MB), polled with own metrics.
    pub host_app_rss_mb: f32,
    pub system_swap_mb: f32,
    /// Alternates every ~500 ms while any pane is alerted/waiting, driving the blink effect.
    pub blink_phase: bool,
    last_blink_tick: Instant,
    /// Frame index (0–5) for the thinking spinner animation, advanced every ~120 ms.
    pub thinking_frame: usize,
    last_thinking_tick: Instant,
    /// Background Claude daemon jobs from ~/.claude/jobs/
    pub jobs: Vec<JobEntry>,
    last_jobs_tick: Instant,
    /// Scroll offset within the agents list (items, not rows).
    pub jobs_scroll_offset: usize,
}

fn scan_dirs(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() { continue; }
            dirs.push(path);
        }
    }
    dirs.sort();
    dirs
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

        // Sync window-status-style for windows that already have @ccmux_color set
        // (e.g. from a previous ccmux session).
        {
            let pairs: Vec<(&str, &str)> = groups.iter()
                .filter_map(|g| g.color_name.as_deref().map(|c| (g.window_id.as_str(), c)))
                .collect();
            tmux.sync_status_styles(&pairs);
        }

        // Start with the Claude pane in the same window as the sidebar selected,
        // so opening from a Claude window immediately highlights that session.
        let selected = Self::initial_selection(&groups, own_window_id.as_deref());

        let error = if managed_session.is_empty() {
            Some("Not running inside tmux. Launch ccmux from within a tmux session.".into())
        } else {
            None
        };
        let sticky = config.sidebar.sticky;

        let mut app = Self {
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
            fetch_handle: None,
            fetch_repo_root: None,
            folder_scan_handle: None,
            folder_scan_root: None,
            history_handle: None,
            history_repo_root: None,
            git_handle: None,
            git_handle_path: None,
            gitstatus: None,
            gitstatus_path: None,
            last_gitstatus_tick: Instant::now()
                .checked_sub(Duration::from_secs(10))
                .unwrap_or_else(Instant::now),
            last_nav_hint_tick: Instant::now(),
            global_info: GlobalInfo::load(),
            last_global_info_tick: Instant::now(),
            alerted_windows: HashSet::new(),
            last_alerts_tick: Instant::now(),
            alerts_reconciled: false,
            message_shown_at: None,
            own_cpu_pct: 0.0,
            own_rss_mb: 0.0,
            own_cpu_history: std::collections::VecDeque::with_capacity(30),
            last_own_metrics_tick: Instant::now(),
            host_app: None,
            last_host_detect: Instant::now()
                .checked_sub(Duration::from_secs(60))
                .unwrap_or_else(Instant::now),
            host_app_rss_mb: 0.0,
            system_swap_mb: 0.0,
            blink_phase: false,
            last_blink_tick: Instant::now(),
            thinking_frame: 0,
            last_thinking_tick: Instant::now(),
            jobs: Vec::new(), // populated below after pane count is known
            last_jobs_tick: Instant::now(),
            jobs_scroll_offset: 0,
        };
        // Assign display numbers after struct is initialized (needs flat_panes())
        let pane_count = app.flat_panes().len();
        app.jobs = app.assign_job_display_nums(load_jobs(), pane_count);
        Ok(app)
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
        if let Some(id) = own_window_id {
            // Prefer the currently active pane in own window — most likely the one the user
            // just navigated to. Fall back to any pane in the window.
            if let Some(idx) = panes.iter().position(|p| p.window_id == id && p.pane_active) {
                return idx;
            }
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

    /// Total navigable items: panes + daemon jobs.
    pub fn total_items(&self) -> usize {
        self.flat_panes().len() + self.jobs.len()
    }

    /// Returns the selected job if the cursor is in the agents section.
    pub fn selected_job(&self) -> Option<&JobEntry> {
        let pane_count = self.flat_panes().len();
        if self.selected >= pane_count {
            self.jobs.get(self.selected - pane_count)
        } else {
            None
        }
    }

    pub fn selected_pane(&self) -> Option<&DetectedPane> {
        let panes = self.flat_panes();
        if self.selected < panes.len() {
            panes.get(self.selected).copied()
        } else {
            None
        }
    }

    pub fn select_next(&mut self) {
        let count = self.total_items();
        if count > 0 {
            self.selected = (self.selected + 1) % count;
            self.last_entered_idx = None;
        }
    }

    pub fn select_prev(&mut self) {
        let count = self.total_items();
        if count > 0 {
            self.selected = if self.selected == 0 { count - 1 } else { self.selected - 1 };
            self.last_entered_idx = None;
        }
    }

    pub fn select_by_display_num(&mut self, n: usize) {
        let pane_count = self.flat_panes().len();
        // Check panes first
        if let Some(idx) = self.flat_panes().iter().position(|p| p.display_num == n) {
            self.selected = idx;
            self.last_entered_idx = None;
            return;
        }
        // Then jobs
        if let Some(idx) = self.jobs.iter().position(|j| j.display_num == n) {
            self.selected = pane_count + idx;
            self.last_entered_idx = None;
        }
    }

    /// Returns the index into `WINDOW_COLORS` that is not already used by any live session.
    /// Wraps around if all colors are taken. Never returns 0 ("none").
    pub fn next_available_color_idx(&self) -> usize {
        let colors = crate::config::WINDOW_COLORS;
        let n = colors.len();
        let used: std::collections::HashSet<&str> = self.groups.iter()
            .filter_map(|g| g.color_name.as_deref())
            .filter(|c| !c.is_empty())
            .collect();
        // Try indices 1..n in order; return the first not in use.
        for i in 1..n {
            if !used.contains(colors[i].2) {
                return i;
            }
        }
        // All taken — cycle based on session count so each new session still differs from its
        // immediate predecessor.
        (self.groups.len() % (n - 1)) + 1
    }

    /// Returns `WorktreeOpts::default()` with `color_idx` pre-filled to the next available color.
    pub fn default_new_session_opts(&self) -> crate::sidebar::mode::WorktreeOpts {
        let mut opts = crate::sidebar::mode::WorktreeOpts::default();
        opts.color_idx = self.next_available_color_idx();
        opts
    }

    pub fn clear_messages(&mut self) {
        self.error = None;
        self.message = None;
        self.message_shown_at = None;
    }

    pub fn set_message(&mut self, msg: impl Into<String>) {
        self.message = Some(msg.into());
        self.message_shown_at = Some(Instant::now());
    }

    /// Auto-clear the message after 3 seconds. Returns true if the display changed.
    pub fn tick_message(&mut self) -> bool {
        if let Some(shown_at) = self.message_shown_at {
            if shown_at.elapsed() >= Duration::from_secs(3) {
                self.message = None;
                self.message_shown_at = None;
                return true;
            }
        }
        false
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

        // Remember which pane was selected (by stable ID) so we can re-anchor
        // after the list is rebuilt. Without this, adding or removing a window
        // before the cursor silently shifts the index to a different pane.
        let selected_pane_id: Option<String> = Self::flat_panes_ref(&self.groups)
            .get(self.selected)
            .map(|p| p.pane_id.clone());

        self.groups = new_groups;

        // Re-anchor selection to the same pane if it still exists, otherwise clamp.
        if let Some(ref pid) = selected_pane_id {
            if let Some(idx) = Self::flat_panes_ref(&self.groups)
                .iter().position(|p| &p.pane_id == pid)
            {
                self.selected = idx;
            } else {
                let count = self.flat_panes().len();
                if count > 0 && self.selected >= count {
                    self.selected = count - 1;
                }
            }
        }

        changed
    }

    /// Update Claude status via content-change detection. Returns true if any status changed.
    pub fn tick_status(&mut self) -> bool {
        if self.last_status_tick.elapsed() < Duration::from_millis(self.config.sidebar.status_ms) {
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
                    // Content changed — Working is the safe default; only override
                    // to WaitingInput if a confirmation dialog is visible.
                    // (Calling full detect_status here caused Idle false-positives
                    // when Claude is working but ctrl+c hint isn't in the capture.)
                    detect_changed_status(&content)
                } else {
                    detect_static_status(&content)
                }
            } else {
                detect_status(&content)
            };

            updates.push((pane_id.clone(), new_status, content));
        }

        let mut changed = false;
        // Snapshot the jobs list before the mutable groups loop (disjoint field borrows).
        let jobs_snap: Vec<(std::path::PathBuf, crate::jobs::JobStatus)> = self.jobs
            .iter()
            .map(|j| (j.cwd.clone(), j.status.clone()))
            .collect();

        // Collect window IDs for alert transitions. newly_alerted: set the flag.
        // newly_cleared: clear the flag (WaitingInput dismissed without user focusing).
        let mut newly_alerted: Vec<String> = Vec::new();
        let mut newly_cleared: Vec<String> = Vec::new();

        for (pane_id, new_status, content) in updates {
            for group in &mut self.groups {
                for pane in &mut group.panes {
                    if pane.pane_id == pane_id {
                        // Hybrid fallback: if terminal content gives Idle/Unknown, defer to
                        // the daemon job's state.json (already loaded in self.jobs) for the
                        // same cwd — Working or Blocked there are authoritative.
                        let effective_status = if matches!(new_status, ClaudeCodeStatus::Idle | ClaudeCodeStatus::Unknown) {
                            jobs_snap.iter()
                                .filter(|(cwd, _)| *cwd == pane.current_path)
                                .find_map(|(_, js)| match js {
                                    crate::jobs::JobStatus::Working => Some(ClaudeCodeStatus::Working),
                                    crate::jobs::JobStatus::Blocked => Some(ClaudeCodeStatus::WaitingInput),
                                    _ => None,
                                })
                                .unwrap_or_else(|| new_status.clone())
                        } else {
                            new_status.clone()
                        };
                        if effective_status != pane.status {
                            let was_waiting = pane.status == ClaudeCodeStatus::WaitingInput;
                            let was_busy = matches!(pane.status,
                                ClaudeCodeStatus::Working | ClaudeCodeStatus::Thinking);
                            let now_needs_attention = matches!(effective_status,
                                ClaudeCodeStatus::Idle | ClaudeCodeStatus::WaitingInput);
                            let now_waiting = effective_status == ClaudeCodeStatus::WaitingInput;
                            let now_busy = matches!(effective_status,
                                ClaudeCodeStatus::Working | ClaudeCodeStatus::Thinking);
                            // Alert on Working/Thinking→Idle/WaitingInput, and also on any
                            // transition into WaitingInput (catches agents action items detected
                            // on startup or from a session that was already idle).
                            if (was_busy && now_needs_attention) || (now_waiting && !was_busy) {
                                newly_alerted.push(group.window_id.clone());
                            } else if (now_busy && !was_waiting) || (was_waiting && !now_waiting) {
                                // Session went back to working (from non-waiting state), or
                                // WaitingInput was dismissed (e.g. user pressed Esc) — clear
                                // any stale alert. Exclude was_waiting→now_busy to avoid
                                // clearing on detect_changed_status false-negatives when a
                                // dialog is still present but the timer update flipped content.
                                newly_cleared.push(group.window_id.clone());
                            }
                            pane.status = effective_status;
                            changed = true;
                        }
                    }
                }
            }
            self.pane_content_cache.insert(pane_id, content);
        }

        for window_id in newly_alerted {
            let _ = tmux.cmd()
                .args(["set-window-option", "-t", &window_id, "@ccmux_alert", "1"])
                .status();
            self.alerted_windows.insert(window_id.clone());
        }
        for window_id in newly_cleared {
            let _ = tmux.cmd()
                .args(["set-window-option", "-ut", &window_id, "@ccmux_alert"])
                .status();
            self.alerted_windows.remove(&window_id);
        }

        changed
    }

    /// Reload daemon jobs from disk (called every 2 s). Returns true if the list changed.
    pub fn tick_jobs(&mut self) -> bool {
        if self.last_jobs_tick.elapsed() < Duration::from_secs(2) {
            return false;
        }
        self.last_jobs_tick = Instant::now();

        let pane_count = self.flat_panes().len();

        // Keep only true background daemons: Claude Code names the PTY socket after the
        // job's short ID for headless agents (e.g. "c16e74e0.sock") but draws from a
        // random-hash spare pool for interactive sessions ("dcf2ef6c.pty.sock"). This is
        // more reliable than cwd matching, which incorrectly suppressed background agents
        // sharing a directory with a separate interactive session.
        let raw_jobs: Vec<JobEntry> = load_jobs()
            .into_iter()
            .filter(|j| crate::jobs::is_background_daemon(&j.id))
            .collect();
        let new_jobs = self.assign_job_display_nums(raw_jobs, pane_count);

        let changed = new_jobs.len() != self.jobs.len()
            || new_jobs.iter().zip(&self.jobs).any(|(a, b)| {
                a.id != b.id || a.status != b.status || a.detail != b.detail
            });

        // Re-anchor selection if we were on a job that still exists
        if self.selected >= pane_count {
            let job_idx = self.selected - pane_count;
            if let Some(old_id) = self.jobs.get(job_idx).map(|j| j.id.clone()) {
                if let Some(new_idx) = new_jobs.iter().position(|j| j.id == old_id) {
                    self.selected = pane_count + new_idx;
                } else {
                    self.selected = self.total_items().saturating_sub(1).min(pane_count);
                }
            }
        }

        self.jobs = new_jobs;
        changed
    }

    fn assign_job_display_nums(&self, mut jobs: Vec<JobEntry>, pane_count: usize) -> Vec<JobEntry> {
        let start = pane_count + 1;
        for (i, job) in jobs.iter_mut().enumerate() {
            job.display_num = start + i;
        }
        jobs
    }

    /// Open the selected agent in a new tmux window.
    /// Prefers a direct PTY attach (via `ccmux pty-attach`) when the daemon has a live
    /// socket for the session; falls back to `claude agents` TUI otherwise.
    pub fn resume_selected_job(&mut self) {
        let Some(job) = self.selected_job() else { return };
        let cwd = job.cwd.display().to_string();
        let name = job.name.clone();
        let id = job.id.clone();
        let tmux = Tmux::new(self.managed_server.clone());

        // Direct PTY attach — most natural; gives a real live terminal in the agent
        if let Some(sock) = crate::jobs::pty_sock_for_session(&id) {
            if std::path::Path::new(&sock).exists() {
                let binary = std::env::current_exe()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_else(|_| "ccmux".to_string());
                let cmd = format!("{} pty-attach {}", binary, id);
                match tmux.new_window_cmd(&self.managed_session, &name,
                                          std::path::Path::new(&cwd), &cmd) {
                    Ok(_) => { self.set_message(format!("Attached to: {}", name)); return; }
                    Err(_) => {} // fall through
                }
            }
        }

        // Reuse an existing `claude agents` window if one is already open
        if let Some(window_id) = self.find_agents_window(&tmux) {
            let _ = tmux.select_window(&window_id);
            self.set_message("Switched to existing agents window");
            return;
        }

        // Last resort: open the claude agents TUI
        match tmux.new_window_cmd(&self.managed_session, "agents",
                                  std::path::Path::new(&cwd), "claude agents") {
            Ok(_) => self.set_message(format!("Opening agents view for: {}", name)),
            Err(e) => self.error = Some(format!("Failed to open agents view: {}", e)),
        }
    }

    /// Find an existing tmux window in this session that is running `claude agents`.
    fn find_agents_window(&self, tmux: &Tmux) -> Option<String> {
        let out = tmux.cmd()
            .args(["list-panes", "-a", "-s", "-t", &self.managed_session,
                   "-F", "#{window_id} #{window_name} #{pane_current_command}"])
            .output().ok()?;
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .find(|line| {
                let lower = line.to_lowercase();
                lower.contains("claude") && lower.contains("agents")
            })
            .and_then(|line| line.split_whitespace().next())
            .map(String::from)
    }

    /// Send a text reply to the selected blocked job by appending to its timeline.jsonl.
    pub fn reply_to_selected_job(&mut self, text: &str) {
        let Some(job) = self.selected_job() else { return };
        let job = job.clone();
        match reply_to_job(&job, text) {
            Ok(_) => {
                self.set_message(format!("Reply sent to: {}", job.name));
                // Optimistically flip the job to Working so the amber blink clears immediately.
                // tick_jobs() will sync the real state from state.json within 2 s.
                let pane_count = self.flat_panes().len();
                if self.selected >= pane_count {
                    let job_idx = self.selected - pane_count;
                    if let Some(j) = self.jobs.get_mut(job_idx) {
                        j.status = crate::jobs::JobStatus::Working;
                        j.detail = format!("↩ replied: {}", text);
                        j.needs = None;
                    }
                }
            }
            Err(e) => self.error = Some(format!("Reply failed: {}", e)),
        }
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
            // Only auto-open a sidebar when sticky is on; otherwise just navigate.
            if self.sticky {
                self.ensure_sidebar_in_window(&window_id, Some(&pane_id));
            }
        }
        // Always land on the Claude pane (split-window steals focus, so this re-focuses it).
        let _ = tmux.select_pane(&pane_id);
        // Clear the alert — user is now looking at this session.
        let _ = tmux.del_window_var(&window_id, "@ccmux_alert");
        self.alerted_windows.remove(&window_id);
    }

    /// Spawn a sidebar in `window_id` if one isn't already there.
    /// Re-selects `claude_pane_id` afterwards since split-window steals focus.
    fn ensure_sidebar_in_window(&self, window_id: &str, claude_pane_id: Option<&str>) {
        let tmux = Tmux::new(self.managed_server.clone());
        let var_key = format!("@ccmux_sidebar_{}_{}", self.managed_session, window_id);
        let hint_key = format!("@ccmux_nav_{}_{}", self.managed_session, window_id);

        // Already open? Signal it to select the correct pane and return.
        if let Some(pane_id) = tmux.get_var(&var_key) {
            if tmux.pane_exists(&pane_id) {
                if let Some(cpid) = claude_pane_id {
                    let _ = tmux.set_var(&hint_key, cpid);
                }
                return;
            }
            let _ = tmux.del_var(&var_key);
        }

        let binary = std::env::current_exe()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| "ccmux".to_string());
        let sidebar_cmd = match &self.managed_server {
            Some(s) => format!("{} sidebar --server {}", binary, s),
            None => format!("{} sidebar", binary),
        };

        if let Ok(new_pane) = tmux.split_sidebar(window_id, self.config.sidebar.width, &sidebar_cmd) {
            if !new_pane.is_empty() {
                let _ = tmux.set_var(&var_key, &new_pane);
                if let Some(cpid) = claude_pane_id {
                    // Tell the new sidebar which Claude pane to pre-select.
                    let _ = tmux.set_var(&hint_key, cpid);
                }
                // Return focus to the Claude pane (or stay on sidebar if None).
                let _ = tmux.select_pane(claude_pane_id.unwrap_or(&new_pane));
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
        self.set_message(if self.sticky { "Sticky on" } else { "Sticky off" });
    }

    /// Send `text` followed by Enter to the selected Claude pane.
    pub fn send_message(&mut self, text: &str) {
        let Some(pane) = self.selected_pane() else { return };
        let pane_id = pane.pane_id.clone();
        let tmux = Tmux::new(self.managed_server.clone());
        match tmux.send_keys(&pane_id, text) {
            Ok(_) => self.set_message("Sent"),
            Err(e) => self.error = Some(format!("Send failed: {}", e)),
        }
    }

    /// Poll whether the sidebar pane is still the active tmux pane. Returns true if changed.
    pub fn tick_focus(&mut self) -> bool {
        if self.last_focus_tick.elapsed() < Duration::from_millis(1000) {
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
            // When regaining focus, snap selection back to the current window's pane.
            if active {
                let own_window = self.own_window_id.clone().unwrap_or_default();
                if let Some(idx) = self.flat_panes()
                    .iter()
                    .position(|p| p.window_id == own_window)
                {
                    self.selected = idx;
                }
            }
            true
        } else {
            false
        }
    }

    /// Check for a nav-hint set by another sidebar telling us which pane to select.
    /// Returns true if selection changed.
    pub fn tick_nav_hint(&mut self) -> bool {
        if self.last_nav_hint_tick.elapsed() < Duration::from_millis(1000) {
            return false;
        }
        self.last_nav_hint_tick = Instant::now();

        let own_window = match &self.own_window_id {
            Some(w) => w.clone(),
            None => return false,
        };
        let hint_key = format!("@ccmux_nav_{}_{}", self.managed_session, own_window);
        let tmux = Tmux::new(self.managed_server.clone());
        let Some(target_pane_id) = tmux.get_var(&hint_key) else { return false };
        // Consume the hint immediately so it's not processed twice.
        let _ = tmux.del_var(&hint_key);
        if let Some(idx) = self.flat_panes().iter().position(|p| p.pane_id == target_pane_id) {
            if self.selected != idx {
                self.selected = idx;
                return true;
            }
        }
        false
    }

    /// Poll @ccmux_alert on each visible window. Returns true if alert set changed.
    pub fn tick_alerts(&mut self) -> bool {
        if self.last_alerts_tick.elapsed() < Duration::from_secs(2) {
            return false;
        }
        self.last_alerts_tick = Instant::now();

        // On the very first tick after startup, reconcile: clear any stale @ccmux_alert
        // flags from the previous sidebar run. Keep only genuine WaitingInput alerts.
        let first_run = !self.alerts_reconciled;
        self.alerts_reconciled = true;

        let tmux = Tmux::new(self.managed_server.clone());
        let window_ids: HashSet<String> = self.groups.iter()
            .flat_map(|g| g.panes.iter())
            .map(|p| p.window_id.clone())
            .collect();

        let mut new_alerted: HashSet<String> = HashSet::new();
        for wid in &window_ids {
            if tmux.get_window_var(wid, "@ccmux_alert").as_deref() == Some("1") {
                let window_waiting = self.groups.iter()
                    .flat_map(|g| g.panes.iter())
                    .filter(|p| &p.window_id == wid)
                    .any(|p| p.status == ClaudeCodeStatus::WaitingInput);
                let window_busy = !window_waiting && self.groups.iter()
                    .flat_map(|g| g.panes.iter())
                    .filter(|p| &p.window_id == wid)
                    .any(|p| matches!(p.status,
                        ClaudeCodeStatus::Working | ClaudeCodeStatus::Thinking));

                if window_busy || (first_run && !window_waiting) {
                    // Clear: session is actively working, OR it's the startup reconciliation
                    // pass and the session has no active dialog (stale "task completed" flag).
                    let _ = tmux.cmd()
                        .args(["set-window-option", "-ut", wid, "@ccmux_alert"])
                        .status();
                } else {
                    new_alerted.insert(wid.clone());
                }
            }
        }

        let changed = new_alerted != self.alerted_windows;
        self.alerted_windows = new_alerted;
        changed
    }

    /// Reload global info from statusline cache files. Returns true if called (always triggers redraw).
    pub fn tick_global_info(&mut self) -> bool {
        if self.last_global_info_tick.elapsed() < Duration::from_secs(30) {
            return false;
        }
        self.last_global_info_tick = Instant::now();
        self.global_info = GlobalInfo::load();
        true
    }

    /// Check whether the background branch-fetch thread finished. Returns true if mode changed.
    pub fn tick_worktree(&mut self) -> bool {
        let done = self.fetch_handle.as_ref().map(|h| h.is_finished()).unwrap_or(false);
        if !done { return false; }

        let result = self.fetch_handle.take().unwrap()
            .join()
            .unwrap_or_else(|_| Err(anyhow::anyhow!("thread panicked")));
        let repo_root = self.fetch_repo_root.take().unwrap_or_default();

        match result {
            Ok(branches) => {
                self.mode = crate::sidebar::mode::Mode::WorktreeFlow(
                    crate::sidebar::mode::WorktreeStep::BranchSelect {
                        repo_root,
                        branches,
                        filter: String::new(),
                        filter_cursor: 0,
                        cursor: 0,
                        entering_new: false,
                        new_branch_text: String::new(),
                        new_branch_cursor: 0,
                    },
                );
            }
            Err(e) => {
                self.error = Some(format!("Fetch failed: {}", e));
                self.mode = crate::sidebar::mode::Mode::Normal;
            }
        }
        true
    }

    pub fn start_folder_pick(&mut self) {
        use crate::sidebar::mode::FolderPickStep;
        let home = std::env::var("HOME").unwrap_or_default();
        let dev = std::path::PathBuf::from(&home).join("dev");
        let root = if dev.is_dir() { dev } else { std::path::PathBuf::from(&home) };
        let root_clone = root.clone();
        self.folder_scan_root = Some(root);
        self.folder_scan_handle = Some(std::thread::spawn(move || scan_dirs(&root_clone)));
        self.mode = Mode::FolderPick(FolderPickStep::Scanning);
    }

    pub fn tick_folder_pick(&mut self) -> bool {
        use crate::sidebar::mode::FolderPickStep;
        let done = self.folder_scan_handle.as_ref().map(|h| h.is_finished()).unwrap_or(false);
        if !done { return false; }
        let handle = self.folder_scan_handle.take().unwrap();
        let dirs = handle.join().unwrap_or_default();
        let root = self.folder_scan_root.take().unwrap_or_default();
        self.mode = Mode::FolderPick(FolderPickStep::Picking { root, dirs, filter: String::new(), filter_cursor: 0, cursor: 0 });
        true
    }

    /// Begin loading Claude history for the selected pane's repo.
    pub fn start_history(&mut self) {
        let Some(pane) = self.selected_pane() else { return };
        let cwd = pane.current_path.clone();
        let Some(repo_root) = crate::git::find_main_repo_root(&cwd) else {
            self.set_message("Not inside a git repository");
            return;
        };
        let current_cwd = cwd.to_string_lossy().to_string();
        self.history_repo_root = Some(repo_root.to_string_lossy().to_string());
        let repo_root_clone = repo_root.clone();
        self.history_handle = Some(std::thread::spawn(move || {
            crate::history::scan_repo_sessions(&repo_root_clone, &current_cwd)
        }));
        self.mode = Mode::History(crate::sidebar::mode::HistoryStep::Loading);
    }

    /// Check whether the background history scan finished. Returns true if mode changed.
    pub fn tick_history(&mut self) -> bool {
        let done = self.history_handle.as_ref().map(|h| h.is_finished()).unwrap_or(false);
        if !done { return false; }
        let entries = self.history_handle.take().unwrap().join().unwrap_or_default();
        let repo_root = self.history_repo_root.take().unwrap_or_default();
        self.mode = Mode::History(crate::sidebar::mode::HistoryStep::List {
            entries, repo_root, filter: String::new(), filter_cursor: 0, cursor: 0,
        });
        true
    }

    /// Recompute git status for the selected repo when the selection changes or the
    /// throttle elapses. Reaps the background thread. Returns true if the display changed.
    pub fn tick_gitstatus(&mut self) -> bool {
        // Reap a finished computation.
        if self.git_handle.as_ref().map(|h| h.is_finished()).unwrap_or(false) {
            let res = self.git_handle.take().unwrap().join().unwrap_or(None);
            self.gitstatus = res;
            self.gitstatus_path = self.git_handle_path.take();
            return true;
        }

        // Resolve the selected pane's repo root.
        let repo = self.selected_pane()
            .map(|p| p.current_path.clone())
            .and_then(|p| crate::git::find_repo_root(&p));

        let repo = match repo {
            None => {
                // Not a git repo — drop any stale status.
                let changed = self.gitstatus.is_some() || self.gitstatus_path.is_some();
                self.gitstatus = None;
                self.gitstatus_path = None;
                return changed;
            }
            Some(r) => r,
        };

        let selection_changed = self.gitstatus_path.as_deref() != Some(repo.as_path());
        let throttle_elapsed = self.last_gitstatus_tick.elapsed() >= Duration::from_secs(3);
        let inflight = self.git_handle.is_some();

        let mut changed = false;
        if selection_changed && self.git_handle_path.as_deref() != Some(repo.as_path()) {
            // Selection moved to a different repo — clear stale status so the
            // renderer shows a placeholder until the new result lands.
            if self.gitstatus.is_some() { changed = true; }
            self.gitstatus = None;
            self.gitstatus_path = None;
        }

        if (selection_changed || throttle_elapsed) && !inflight {
            self.last_gitstatus_tick = Instant::now();
            self.git_handle_path = Some(repo.clone());
            self.git_handle = Some(std::thread::spawn(move || {
                crate::gitstatus::run_status(&repo)
            }));
        }
        changed
    }

    pub fn navigate_folder_into(&mut self, path: std::path::PathBuf) {
        use crate::sidebar::mode::FolderPickStep;
        self.folder_scan_root = Some(path.clone());
        self.folder_scan_handle = Some(std::thread::spawn(move || scan_dirs(&path)));
        self.mode = Mode::FolderPick(FolderPickStep::Scanning);
    }

    pub fn navigate_folder_up(&mut self) {
        use crate::sidebar::mode::FolderPickStep;
        if let Mode::FolderPick(FolderPickStep::Picking { ref root, .. }) = self.mode.clone() {
            if let Some(parent) = root.parent() {
                let parent = parent.to_path_buf();
                self.folder_scan_root = Some(parent.clone());
                self.folder_scan_handle = Some(std::thread::spawn(move || scan_dirs(&parent)));
                self.mode = Mode::FolderPick(FolderPickStep::Scanning);
            }
        }
    }

    pub fn execute_folder_pick(
        &mut self,
        path: std::path::PathBuf,
        is_new: bool,
        opts: &crate::sidebar::mode::WorktreeOpts,
    ) {
        use crate::config::{AVAILABLE_MODELS, AVAILABLE_EFFORTS, WINDOW_COLORS};

        if is_new {
            if let Err(e) = std::fs::create_dir_all(&path) {
                self.error = Some(format!("Cannot create folder: {}", e));
                self.mode = Mode::Normal;
                return;
            }
        }

        let tmux = Tmux::new(self.managed_server.clone());
        let window_name = path.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "new".to_string());
        let window_id = match tmux.new_window(&self.managed_session, &window_name, &path) {
            Ok(id) => id,
            Err(e) => {
                self.error = Some(format!("Window error: {}", e));
                self.mode = Mode::Normal;
                return;
            }
        };

        let (_, hex, tmux_colour) = WINDOW_COLORS[opts.color_idx];
        if !tmux_colour.is_empty() {
            let _ = tmux.set_window_color(&window_id, tmux_colour);
        }

        if opts.launch_claude {
            let model = AVAILABLE_MODELS[opts.model_idx];
            let effort = AVAILABLE_EFFORTS[opts.effort_idx];
            let _ = tmux.send_keys(
                &window_id,
                &format!("claude --model {} --effort {} --name '{}'", model, effort, window_name),
            );
        }

        if opts.open_vscode {
            if !hex.is_empty() { Self::write_vscode_color(&path, hex); }
            Self::spawn_vscode(&path);
        }

        self.ensure_sidebar_in_window(&window_id, None);
        self.set_message(format!("✓ Session created: {}", window_name));
        self.mode = Mode::Normal;
        let _ = self.refresh();
    }

    /// Navigate to an already-existing worktree instead of creating a new one.
    /// Finds a tmux window with a pane inside `wt_path`; if none exists, opens a new window there.
    pub fn navigate_to_existing_worktree(&mut self, wt_path: &str) {
        let tmux = Tmux::new(self.managed_server.clone());

        // Scan all panes in the session for one whose path is inside the worktree.
        if let Ok(out) = tmux.cmd()
            .args(["list-panes", "-s", "-t", &self.managed_session,
                   "-F", "#{window_id}\t#{pane_current_path}"])
            .output()
        {
            let stdout = String::from_utf8_lossy(&out.stdout);
            for line in stdout.lines() {
                let mut parts = line.splitn(2, '\t');
                let window_id = parts.next().unwrap_or("").trim().to_string();
                let pane_path  = parts.next().unwrap_or("").trim();
                if pane_path.starts_with(wt_path) {
                    let _ = tmux.select_window(&window_id);
                    self.set_message("Switched to existing worktree window");
                    return;
                }
            }
        }

        // No open window found — create one at the worktree path without running git worktree add.
        let path = std::path::PathBuf::from(wt_path);
        let window_name = path.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "worktree".to_string());
        match tmux.new_window(&self.managed_session, &window_name, &path) {
            Ok(_) => self.set_message(format!("Opened existing worktree: {}", window_name)),
            Err(e) => self.error = Some(format!("Failed to open worktree window: {}", e)),
        }
    }

    /// Begin the worktree creation flow for the selected pane's repo.
    pub fn start_worktree_flow(&mut self) {
        let Some(pane) = self.selected_pane() else { return };
        let path = pane.current_path.clone();

        let Some(repo_root) = crate::git::find_main_repo_root(&path) else {
            self.error = Some("Not inside a git repository".into());
            return;
        };

        let root_str = repo_root.to_string_lossy().to_string();
        self.fetch_repo_root = Some(root_str);
        self.mode = crate::sidebar::mode::Mode::WorktreeFlow(
            crate::sidebar::mode::WorktreeStep::Fetching,
        );

        self.fetch_handle = Some(std::thread::spawn(move || {
            crate::git::fetch_origin(&repo_root).ok(); // fetch errors are non-fatal
            crate::git::list_branches(&repo_root)
        }));
    }

    /// Like start_worktree_flow, but uses a caller-supplied repo path instead of the selected pane.
    pub fn start_worktree_flow_for_path(&mut self, path: std::path::PathBuf) {
        let Some(repo_root) = crate::git::find_main_repo_root(&path) else {
            self.error = Some(format!("Not a git repository: {}", path.display()));
            return;
        };
        let root_str = repo_root.to_string_lossy().to_string();
        self.fetch_repo_root = Some(root_str);
        self.mode = crate::sidebar::mode::Mode::WorktreeFlow(
            crate::sidebar::mode::WorktreeStep::Fetching,
        );
        self.fetch_handle = Some(std::thread::spawn(move || {
            crate::git::fetch_origin(&repo_root).ok();
            crate::git::list_branches(&repo_root)
        }));
    }

    /// Execute worktree creation (or open an existing one) after user confirms options.
    /// `existing_wt_path` is `Some` when the worktree already exists — skips `git worktree add`
    /// and instead finds or opens a tmux window at that path.
    pub fn execute_worktree(
        &mut self,
        repo_root: &str,
        branch: &str,
        folder: &str,
        opts: &crate::sidebar::mode::WorktreeOpts,
        existing_wt_path: Option<&str>,
    ) {
        use crate::config::{AVAILABLE_MODELS, AVAILABLE_EFFORTS, WINDOW_COLORS};

        let tmux = Tmux::new(self.managed_server.clone());
        let (_, hex, tmux_colour) = WINDOW_COLORS[opts.color_idx];

        let window_id = if let Some(wt_path) = existing_wt_path {
            // Existing worktree — find or open a window there without running git worktree add.
            let wt = std::path::PathBuf::from(wt_path);

            // Try to find an existing tmux window with a pane inside the worktree.
            // Include pane_id and pane_current_command so we can target the shell pane
            // directly (not the sidebar) when launching Claude.
            let found = tmux.cmd()
                .args(["list-panes", "-s", "-t", &self.managed_session,
                       "-F", "#{window_id}\t#{pane_current_path}\t#{pane_id}\t#{pane_current_command}"])
                .output()
                .ok()
                .and_then(|out| {
                    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
                    // Two-pass: first find the window_id, then find the best shell pane in it.
                    // Pass 1: find a window that has any pane inside the worktree.
                    let wid = stdout.lines().find_map(|line| {
                        let mut p = line.splitn(4, '\t');
                        let wid   = p.next()?.trim().to_string();
                        let ppath = p.next()?.trim().to_string();
                        if ppath.starts_with(wt_path) { Some(wid) } else { None }
                    })?;
                    // Pass 2: find a non-sidebar pane in that window to use as the send target.
                    let shell_pane = stdout.lines().find_map(|line| {
                        let mut p = line.splitn(4, '\t');
                        let w   = p.next()?.trim().to_string();
                        let _   = p.next(); // pane_current_path
                        let pid = p.next()?.trim().to_string();
                        let cmd = p.next()?.trim().to_string();
                        if w == wid && !cmd.contains("ccmux") { Some(pid) } else { None }
                    });
                    Some((wid, shell_pane))
                });

            if let Some((wid, shell_pane)) = found {
                let _ = tmux.select_window(&wid);
                self.set_message(format!("✓ Switched to existing worktree: {}", folder));
                // Apply options to the already-open window then return.
                if !tmux_colour.is_empty() { let _ = tmux.set_window_color(&wid, tmux_colour); }
                if opts.open_vscode {
                    if !hex.is_empty() { Self::write_vscode_color(&wt, hex); }
                    Self::spawn_vscode(&wt);
                }
                if opts.launch_claude {
                    // Send to the shell pane specifically — NOT the window ID, which would
                    // target the active pane (often the sidebar after ensure_sidebar_in_window).
                    // Sending the launch command to the sidebar TUI is a critical bug: 'c' in
                    // the command triggers start_folder_pick() and the rest of the string
                    // ends up typed into the filter field.
                    if let Some(ref pane_id) = shell_pane {
                        let model = AVAILABLE_MODELS[opts.model_idx];
                        let effort = AVAILABLE_EFFORTS[opts.effort_idx];
                        let display_name = std::path::Path::new(folder)
                            .file_name().map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| folder.to_string());
                        let _ = tmux.send_keys(pane_id, &format!("claude --model {} --effort {} --name '{}'", model, effort, display_name));
                    }
                }
                self.ensure_sidebar_in_window(&wid, None);
                self.mode = crate::sidebar::mode::Mode::Normal;
                let _ = self.refresh();
                return;
            }

            // No window open — create one (no git worktree add needed).
            let window_name = wt.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| folder.to_string());
            match tmux.new_window(&self.managed_session, &window_name, &wt) {
                Ok(id) => id,
                Err(e) => {
                    self.error = Some(format!("Window error: {}", e));
                    self.mode = crate::sidebar::mode::Mode::Normal;
                    return;
                }
            }
        } else {
            // New worktree — create it first.
            self.mode = crate::sidebar::mode::Mode::WorktreeFlow(
                crate::sidebar::mode::WorktreeStep::Executing { status: "Creating worktree…".into() },
            );

            let repo_path = std::path::PathBuf::from(repo_root);
            let worktree_path = if std::path::Path::new(folder).is_absolute() {
                std::path::PathBuf::from(folder)
            } else {
                let parent = repo_path.parent().unwrap_or(&repo_path).to_path_buf();
                parent.join(folder)
            };

            if worktree_path.exists() {
                self.error = Some(format!("Path already exists: {}", worktree_path.display()));
                self.mode = crate::sidebar::mode::Mode::Normal;
                return;
            }

            if let Err(e) = crate::git::create_worktree(&repo_path, &worktree_path, branch, opts.base_branch.as_deref()) {
                self.error = Some(format!("Worktree error: {}", e));
                self.mode = crate::sidebar::mode::Mode::Normal;
                return;
            }

            let window_name = worktree_path.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| folder.to_string());
            match tmux.new_window(&self.managed_session, &window_name, &worktree_path) {
                Ok(id) => id,
                Err(e) => {
                    self.error = Some(format!("Window error: {}", e));
                    self.mode = crate::sidebar::mode::Mode::Normal;
                    return;
                }
            }
        };

        // Apply options to the new/found window.
        if !tmux_colour.is_empty() { let _ = tmux.set_window_color(&window_id, tmux_colour); }

        let wt_path_buf = existing_wt_path
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                let r = std::path::PathBuf::from(repo_root);
                r.parent().unwrap_or(&r).join(folder)
            });

        // VS Code is a GUI app — spawn it directly (invisible, no terminal output).
        // Claude must use send_keys because it's an interactive TUI in the terminal.
        if opts.open_vscode {
            if !hex.is_empty() { Self::write_vscode_color(&wt_path_buf, hex); }
            Self::spawn_vscode(&wt_path_buf);
        }
        if opts.launch_claude {
            let model = AVAILABLE_MODELS[opts.model_idx];
            let effort = AVAILABLE_EFFORTS[opts.effort_idx];
            let display_name = wt_path_buf.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| folder.to_string());
            let _ = tmux.send_keys(&window_id, &format!("claude --model {} --effort {} --name '{}'", model, effort, display_name));
        }
        self.ensure_sidebar_in_window(&window_id, None);

        self.set_message(format!("✓ Worktree ready: {}", folder));
        self.mode = crate::sidebar::mode::Mode::Normal;
        let _ = self.refresh();
    }

    /// Launch VS Code on `path` silently in the background.
    /// Uses `open -a "Visual Studio Code"` on macOS (no terminal output).
    fn spawn_vscode(path: &std::path::Path) {
        let _ = std::process::Command::new("open")
            .args(["-a", "Visual Studio Code", path.to_string_lossy().as_ref()])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }

    /// Poll ccmux's own CPU% and RSS every 5 seconds using a single-PID ps call.
    /// The call itself takes < 5 ms and runs at most once per 5 000 ms — negligible overhead.
    pub fn tick_own_metrics(&mut self) -> bool {
        if self.last_own_metrics_tick.elapsed() < Duration::from_secs(5) {
            return false;
        }
        self.last_own_metrics_tick = Instant::now();
        let pid = std::process::id();
        let Ok(out) = std::process::Command::new("ps")
            .args(["-p", &pid.to_string(), "-o", "pcpu=,rss="])
            .output()
        else { return false; };
        let s = String::from_utf8_lossy(&out.stdout);
        let parts: Vec<&str> = s.split_whitespace().collect();
        if parts.len() >= 2 {
            self.own_cpu_pct = parts[0].parse().unwrap_or(0.0);
            let rss_kb: u64 = parts[1].parse().unwrap_or(0);
            self.own_rss_mb = rss_kb as f32 / 1024.0;
            if self.own_cpu_history.len() >= 30 { self.own_cpu_history.pop_front(); }
            self.own_cpu_history.push_back(self.own_cpu_pct);

            // Host terminal app: re-detect at most every ~30 s; sample each tick.
            if self.last_host_detect.elapsed() >= Duration::from_secs(30) {
                self.last_host_detect = Instant::now();
                self.host_app =
                    hostmem::detect_host_app(&self.managed_server, &self.managed_session);
            }
            // Only sample host/swap when a host app is known — otherwise the
            // values are never displayed, so don't spawn ps/sysctl for nothing
            // (e.g. SSH / headless / non-macOS).
            if let Some(app) = self.host_app.clone() {
                // A stale/dead PID yields Some(0.0); don't clobber a good reading with 0.
                if let Some(rss) = hostmem::sample_host_rss_mb(app.pid) {
                    if rss > 0.0 {
                        self.host_app_rss_mb = rss;
                    }
                }
                if let Some(swap) = hostmem::sample_system_swap_mb() {
                    self.system_swap_mb = swap;
                }
            }
            return true;
        }
        false
    }

    /// Advance the thinking spinner frame every 500 ms while any pane is Thinking.
    /// Returns true when the frame advances (triggers a redraw).
    pub fn tick_thinking(&mut self) -> bool {
        let any_thinking = self.groups.iter()
            .flat_map(|g| g.panes.iter())
            .any(|p| p.status == crate::session::ClaudeCodeStatus::Thinking);

        if !any_thinking {
            return false;
        }
        if self.last_thinking_tick.elapsed() < Duration::from_millis(500) {
            return false;
        }
        self.last_thinking_tick = Instant::now();
        self.thinking_frame = self.thinking_frame.wrapping_add(1);
        true
    }

    /// Toggle blink phase every 500 ms while any pane needs attention.
    /// Returns true when the phase changes (triggers a redraw).
    pub fn tick_blink(&mut self) -> bool {
        let has_attention = self.groups.iter()
            .flat_map(|g| g.panes.iter())
            .any(|p| {
                self.alerted_windows.contains(&p.window_id)
                    || p.status == crate::session::ClaudeCodeStatus::WaitingInput
            });

        if !has_attention {
            if self.blink_phase {
                self.blink_phase = false;
                return true;
            }
            return false;
        }

        if self.last_blink_tick.elapsed() < Duration::from_millis(500) {
            return false;
        }
        self.last_blink_tick = Instant::now();
        self.blink_phase = !self.blink_phase;
        true
    }

    pub fn own_cpu_avg(&self) -> f32 {
        if self.own_cpu_history.is_empty() { return 0.0; }
        self.own_cpu_history.iter().sum::<f32>() / self.own_cpu_history.len() as f32
    }

    pub fn own_cpu_max(&self) -> f32 {
        self.own_cpu_history.iter().cloned().fold(0.0_f32, f32::max)
    }

    fn write_vscode_color(path: &std::path::Path, hex: &str) {
        let vscode_dir = path.join(".vscode");
        let _ = std::fs::create_dir_all(&vscode_dir);
        let settings_path = vscode_dir.join("settings.json");

        let mut val: serde_json::Value = settings_path
            .exists()
            .then(|| std::fs::read_to_string(&settings_path).ok())
            .flatten()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_else(|| serde_json::json!({}));

        val["workbench.colorCustomizations"]["titleBar.activeBackground"] =
            serde_json::Value::String(hex.to_string());
        val["workbench.colorCustomizations"]["titleBar.inactiveBackground"] =
            serde_json::Value::String(hex.to_string());

        if let Ok(text) = serde_json::to_string_pretty(&val) {
            let _ = std::fs::write(&settings_path, text);
        }
    }

    pub fn execute_new_window(&mut self, name: &str, color_idx: usize, launch_claude: bool) {
        use crate::config::WINDOW_COLORS;

        let tmux = Tmux::new(self.managed_server.clone());
        let path = self.selected_pane()
            .map(|p| p.current_path.clone())
            .unwrap_or_else(|| {
                std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default())
            });

        let window_name = if name.trim().is_empty() { "new" } else { name.trim() };

        let window_id = match tmux.new_window(&self.managed_session, window_name, &path) {
            Ok(id) => id,
            Err(e) => {
                self.error = Some(format!("Window error: {}", e));
                self.mode = crate::sidebar::mode::Mode::Normal;
                return;
            }
        };

        let (_, _, tmux_colour) = WINDOW_COLORS[color_idx];
        if !tmux_colour.is_empty() {
            let _ = tmux.set_window_color(&window_id, tmux_colour);
        }

        if launch_claude {
            let _ = tmux.send_keys(&window_id, &format!("claude --name '{}'", window_name));
        }
        if self.sticky {
            self.ensure_sidebar_in_window(&window_id, None);
        }

        self.set_message(format!("✓ Window created: {}", window_name));
        self.mode = crate::sidebar::mode::Mode::Normal;
        let _ = self.refresh();
    }


    /// Dispatch an ActionItem for the selected pane.
    pub fn send_action(&mut self, item: crate::sidebar::mode::ActionItem) {
        use crate::sidebar::mode::ActionItem;
        match item {
            ActionItem::CreatePR | ActionItem::ViewPR | ActionItem::MergePR | ActionItem::ClosePR => {
                let cmd = match &item {
                    ActionItem::CreatePR => "gh pr create --fill",
                    ActionItem::ViewPR   => "gh pr view",
                    ActionItem::MergePR  => "gh pr merge --auto",
                    ActionItem::ClosePR  => "gh pr close",
                    _ => unreachable!(),
                };
                let Some(pane) = self.selected_pane() else { return };
                let pane_id = pane.pane_id.clone();
                let window_id = pane.window_id.clone();
                let tmux = Tmux::new(self.managed_server.clone());
                let _ = tmux.select_window(&window_id);
                let _ = tmux.select_pane(&pane_id);
                let _ = tmux.send_keys(&pane_id, cmd);
                self.set_message(format!("Sent: {}", cmd));
                self.mode = crate::sidebar::mode::Mode::Normal;
            }
            ActionItem::DeleteWorktree { repo_root, worktree_path } => {
                self.mode = crate::sidebar::mode::Mode::Confirm(
                    crate::sidebar::mode::ConfirmKind::DeleteWorktree {
                        repo_root,
                        worktree_path,
                    },
                );
            }
            ActionItem::SendText { text, .. } => {
                if self.selected_job().is_some() {
                    self.reply_to_selected_job(&text);
                } else {
                    self.send_message(&text);
                }
                self.mode = crate::sidebar::mode::Mode::Normal;
            }
        }
    }

    /// Apply name and color edits to an existing window.
    pub fn execute_edit_window(&mut self, window_id: &str, name: &str, color_idx: usize) {
        use crate::config::WINDOW_COLORS;
        let tmux = Tmux::new(self.managed_server.clone());

        if !name.trim().is_empty() {
            if let Err(e) = tmux.rename_window(window_id, name.trim()) {
                self.error = Some(format!("Rename failed: {}", e));
            }
        }

        let (_, _, tmux_colour) = WINDOW_COLORS[color_idx];
        let _ = tmux.set_window_color(window_id, tmux_colour);
        if let Some(group) = self.groups.iter_mut().find(|g| g.window_id == window_id) {
            group.color_name = if tmux_colour.is_empty() { None } else { Some(tmux_colour.to_string()) };
        }

        self.mode = crate::sidebar::mode::Mode::Normal;
        if self.error.is_none() { self.set_message("✓ Window updated".to_string()); }
        let _ = self.refresh();
    }

    /// Build the action items list for the currently selected pane.
    pub fn action_items_for_selected(&self) -> Vec<crate::sidebar::mode::ActionItem> {
        use crate::sidebar::mode::ActionItem;
        let Some(pane) = self.selected_pane() else { return vec![] };

        let mut items = vec![
            ActionItem::CreatePR,
            ActionItem::ViewPR,
            ActionItem::MergePR,
            ActionItem::ClosePR,
        ];

        // Add "Delete worktree" if this pane lives inside a worktree (not the main repo)
        if let Some(repo_root) = crate::git::find_main_repo_root(&pane.current_path) {
            if let Some(wt_root) = crate::git::find_repo_root(&pane.current_path) {
                let wt_str = wt_root.to_string_lossy().to_string();
                let main_str = repo_root.to_string_lossy().to_string();
                if wt_str != main_str {
                    items.push(ActionItem::DeleteWorktree {
                        worktree_path: wt_str,
                        repo_root: main_str,
                    });
                }
            }
        }

        items
    }

    /// Open a wide tmux popup showing the session's formatted transcript via the pager.
    /// Blocks (modally) until the popup is dismissed — fine, the popup owns the screen.
    pub fn preview_session(&mut self, entry: &crate::history::SessionEntry) {
        let text = std::fs::read_to_string(&entry.file_path).unwrap_or_default();
        let rendered = crate::history::render_transcript(&text, 200);
        let tmp = std::env::temp_dir().join(format!("ccmux-history-{}.txt", entry.id));
        if let Err(e) = std::fs::write(&tmp, rendered) {
            self.set_message(format!("Preview failed: {}", e));
            return;
        }
        let pager = std::env::var("PAGER").unwrap_or_else(|_| "less -R".to_string());
        let tmux = Tmux::new(self.managed_server.clone());
        // -E closes the popup when the command exits; single shell-command arg.
        let cmd = format!("{} {}", pager, shell_quote(&tmp.to_string_lossy()));
        let _ = tmux.cmd()
            .args(["display-popup", "-E", "-w", "85%", "-h", "85%", &cmd])
            .status();
    }

    /// Open a popup with full `git status` + `git diff` for the selected repo (read-only).
    pub fn open_git_popup(&mut self) {
        let Some(pane) = self.selected_pane() else { return };
        let Some(repo) = crate::git::find_repo_root(&pane.current_path) else {
            self.set_message("Not a git repository");
            return;
        };
        let pager = std::env::var("PAGER").unwrap_or_else(|_| "less -R".to_string());
        let dir = shell_quote(&repo.to_string_lossy());
        // Prefer lazygit (files + diff panes, ±counts, GitHub-PR feel). Fall back to a
        // plain colorized `git status` + `git diff` through the pager when lazygit isn't
        // installed, so the popup works everywhere.
        //
        // display-popup runs under the tmux server's environment, which is often started
        // without Homebrew's shellenv — so /opt/homebrew/bin (where lazygit lives) isn't on
        // PATH and `command -v lazygit` would wrongly fall back. Prepend the standard macOS
        // Homebrew bin dirs (Apple Silicon + Intel) so the lookup matches an interactive shell.
        let inner = format!(
            "cd {} && export PATH=\"/opt/homebrew/bin:/usr/local/bin:$PATH\" && \
             if command -v lazygit >/dev/null 2>&1; then lazygit; \
             else {{ git -c color.status=always status; echo; git -c color.ui=always diff; }} | {}; fi",
            dir, pager
        );
        let tmux = Tmux::new(self.managed_server.clone());
        let _ = tmux.cmd()
            .args(["display-popup", "-E", "-w", "90%", "-h", "90%", &inner])
            .status();
    }

    /// Open a popup showing the selected branch's PR overview + CI checks (read-only).
    /// On-demand only — runs `gh` when invoked; no background API traffic.
    pub fn open_pr_popup(&mut self) {
        let Some(pane) = self.selected_pane() else { return };
        let Some(repo) = crate::git::find_repo_root(&pane.current_path) else {
            self.set_message("Not a git repository");
            return;
        };
        let pager = std::env::var("PAGER").unwrap_or_else(|_| "less -R".to_string());
        let dir = shell_quote(&repo.to_string_lossy());
        // gh lives in /opt/homebrew/bin, which the tmux-server environment behind
        // display-popup often lacks — prepend it (same reason as open_git_popup).
        // GH_FORCE_TTY keeps gh's colored, formatted output even when piped to the pager.
        // gh prints its own "no pull requests found" / auth errors, which stay visible.
        let inner = format!(
            "cd {} && export PATH=\"/opt/homebrew/bin:/usr/local/bin:$PATH\" && \
             if command -v gh >/dev/null 2>&1; then \
             {{ GH_FORCE_TTY=100% gh pr view; echo; GH_FORCE_TTY=100% gh pr checks; }} 2>&1 | {}; \
             else echo 'gh not installed' | {}; fi",
            dir, pager, pager
        );
        let tmux = Tmux::new(self.managed_server.clone());
        let _ = tmux.cmd()
            .args(["display-popup", "-E", "-w", "85%", "-h", "85%", &inner])
            .status();
    }

    /// Open a popup file browser for the selected pane's project: drill-down folder
    /// navigation (eza-tree / bat preview) that opens the chosen file in an in-terminal
    /// editor ($CCMUX_EDITOR, default nano — never vim). Read-only browse.
    pub fn open_folder_popup(&mut self) {
        let Some(pane) = self.selected_pane() else { return };
        let start = crate::git::find_repo_root(&pane.current_path)
            .unwrap_or_else(|| pane.current_path.clone());

        // The drill-down loop is multi-line shell, so it's embedded and written to a file
        // rather than crammed into one display-popup command string. Write it into a
        // user-private cache dir (mode 0700), NOT the shared temp dir, to avoid a symlink /
        // TOCTOU swap between write and `sh`.
        let Some(dir_base) = private_cache_dir() else {
            self.set_message("Could not create cache dir");
            return;
        };
        let script_path = dir_base.join("browse.sh");
        if std::fs::write(&script_path, BROWSE_SH).is_err() {
            self.set_message("Could not write browse script");
            return;
        }

        let dir = shell_quote(&start.to_string_lossy());
        let script = shell_quote(&script_path.to_string_lossy());
        let pager = std::env::var("PAGER").unwrap_or_else(|_| "less -R".to_string());
        // CCMUX_EDITOR passthrough: display-popup runs under the tmux server env, which may
        // not carry it; bake the value in when set so the script's default-nano honors it.
        let editor_prefix = match std::env::var("CCMUX_EDITOR") {
            Ok(e) if !e.is_empty() => format!("CCMUX_EDITOR={} ", shell_quote(&e)),
            _ => String::new(),
        };
        // fzf is MacPorts (/opt/local/bin); bat/fd/eza are Homebrew; nano is /usr/bin;
        // a `code` override lives in /usr/local/bin — prepend all so the popup shell finds them.
        let inner = format!(
            "export PATH=\"/opt/homebrew/bin:/opt/local/bin:/usr/local/bin:$PATH\" && \
             if command -v fzf >/dev/null 2>&1; then {}sh {} {}; \
             else echo 'fzf not installed' | {}; fi",
            editor_prefix, script, dir, pager
        );
        let tmux = Tmux::new(self.managed_server.clone());
        let _ = tmux.cmd()
            .args(["display-popup", "-E", "-w", "90%", "-h", "90%", &inner])
            .status();
    }

    /// Open a popup with Neovim's neo-tree file explorer rooted at the selected pane's
    /// project. Uses a fully self-contained Neovim config (lazy.nvim + neo-tree bootstrapped
    /// into an isolated cache dir via XDG_* overrides) so it never touches the user's own
    /// Neovim setup. Files open in Neovim buffers — distinct from `f`/nano by design.
    pub fn open_neotree_popup(&mut self) {
        let Some(pane) = self.selected_pane() else { return };
        let start = crate::git::find_repo_root(&pane.current_path)
            .unwrap_or_else(|| pane.current_path.clone());
        let Some(cache) = private_cache_dir() else {
            self.set_message("Could not create cache dir");
            return;
        };
        let init_path = cache.join("neotree-init.lua");
        if std::fs::write(&init_path, NEOTREE_INIT).is_err() {
            self.set_message("Could not write neo-tree config");
            return;
        }
        // Isolate Neovim's config/data/state under the ccmux cache so nothing — not even
        // lazy's lockfile — lands in the user's ~/.config/nvim or ~/.local/{share,state}.
        let nv = cache.join("nvim");
        let cfg = shell_quote(&nv.join("config").to_string_lossy());
        let data = shell_quote(&nv.join("data").to_string_lossy());
        let state = shell_quote(&nv.join("state").to_string_lossy());
        let init = shell_quote(&init_path.to_string_lossy());
        let dir = shell_quote(&start.to_string_lossy());
        let pager = std::env::var("PAGER").unwrap_or_else(|_| "less -R".to_string());
        let inner = format!(
            "export PATH=\"/opt/homebrew/bin:/opt/local/bin:/usr/local/bin:$PATH\" && \
             if command -v nvim >/dev/null 2>&1; then \
             cd {} && env XDG_CONFIG_HOME={} XDG_DATA_HOME={} XDG_STATE_HOME={} nvim -u {}; \
             else echo 'neovim not installed — run: brew install neovim' | {}; fi",
            dir, cfg, data, state, init, pager
        );
        let tmux = Tmux::new(self.managed_server.clone());
        let _ = tmux.cmd()
            .args(["display-popup", "-E", "-w", "90%", "-h", "90%", &inner])
            .status();
    }

    /// Resume a session in a new tmux window. Uses the session's recorded cwd if it still
    /// exists; otherwise falls back to the repo main root and notes it.
    pub fn resume_session(&mut self, entry: &crate::history::SessionEntry, repo_root: &str) {
        // entry.id comes from a session-file name; never interpolate untrusted text
        // into a shell command. Session ids are UUIDs ([A-Za-z0-9-]).
        if entry.id.is_empty() || !entry.id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            self.set_message(format!("Refusing to resume: invalid session id '{}'", entry.id));
            return;
        }

        let mut dir = entry.cwd.clone();
        let mut fell_back = false;
        if !std::path::Path::new(&dir).is_dir() {
            dir = repo_root.to_string();
            fell_back = true;
        }
        let name = if entry.worktree_label.is_empty() { "resume".to_string() } else { entry.worktree_label.clone() };
        let tmux = Tmux::new(self.managed_server.clone());

        // Launch claude inside a shell (new_window + send_keys), NOT as the window's sole
        // process via new_window_cmd. If `claude --resume` exits — e.g. the session is
        // already open elsewhere, or the id can't be resolved — the shell survives, so the
        // window stays put with the error visible instead of closing and bouncing focus
        // back to the previous window. Mirrors execute_folder_pick / execute_worktree.
        let window_id = match tmux.new_window(&self.managed_session, &name, std::path::Path::new(&dir)) {
            Ok(id) => id,
            Err(e) => { self.set_message(format!("Resume failed: {}", e)); return; }
        };
        let _ = tmux.send_keys(&window_id, &format!("claude --resume {}", entry.id));
        self.ensure_sidebar_in_window(&window_id, None);
        if fell_back {
            self.set_message(format!("Resumed in repo root (original worktree gone): {}", name));
        } else {
            self.set_message(format!("Resumed: {}", entry.title));
        }
        self.mode = Mode::Normal;
        let _ = self.refresh();
    }
}

/// User-private cache dir (`~/.cache/ccmux`, mode 0700), created if needed. Helper scripts
/// and isolated tool configs are written here rather than the shared temp dir, so a
/// predictable name can't be symlink/TOCTOU-swapped between write and exec.
fn private_cache_dir() -> Option<std::path::PathBuf> {
    let base = dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("ccmux");
    std::fs::create_dir_all(&base).ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&base, std::fs::Permissions::from_mode(0o700));
    }
    Some(base)
}

/// Wrap a string in single quotes for safe use in a shell command.
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Embedded drill-down folder browser run inside the `f` popup. Lists the current
/// directory (dirs first) plus `../`; previews folders with `eza --tree` and files with
/// `bat`; descends into folders, ascends via `../`, and opens a chosen file in
/// `$CCMUX_EDITOR` (default nano — never vim), in-terminal. fzf quotes `{}` itself, so the
/// preview must NOT wrap `{}` in quotes.
const BROWSE_SH: &str = r#"#!/bin/sh
dir="${1:-$PWD}"
cd "$dir" 2>/dev/null || exit 0
editor="${CCMUX_EDITOR:-nano}"
while true; do
  sel=$( { printf '../\n'; eza -1 --group-directories-first 2>/dev/null || ls -1; } | \
    fzf --header="$(pwd)" --reverse \
        --preview 'if [ {} = ../ ]; then eza --tree --level=2 --color=always ..; \
                   elif [ -d {} ]; then eza --tree --level=2 --color=always -- {}; \
                   else bat --color=always --style=numbers -- {} 2>/dev/null || cat -- {}; fi' \
        --preview-window=right:65% ) || exit 0
  [ -z "$sel" ] && exit 0
  if [ "$sel" = "../" ]; then cd .. 2>/dev/null || exit 0
  elif [ -d "$sel" ]; then cd "$sel" 2>/dev/null || exit 0
  else "$editor" "$sel"; exit 0
  fi
done
"#;

/// Self-contained Neovim config for the `F` neo-tree popup. Bootstraps lazy.nvim + neo-tree
/// into whatever XDG_DATA_HOME the caller sets (ccmux isolates it under the private cache),
/// so it never touches the user's own Neovim config. Opens the tree once, after neo-tree
/// loads, rooted at Neovim's launch cwd.
const NEOTREE_INIT: &str = r#"-- ccmux neo-tree popup config (isolated; do not edit — regenerated on use).
local lazypath = vim.fn.stdpath("data") .. "/lazy/lazy.nvim"
if not (vim.uv or vim.loop).fs_stat(lazypath) then
  vim.fn.system({ "git", "clone", "--filter=blob:none",
    "https://github.com/folke/lazy.nvim.git", "--branch=stable", lazypath })
end
vim.opt.rtp:prepend(lazypath)

require("lazy").setup({
  {
    "nvim-neo-tree/neo-tree.nvim",
    branch = "v3.x",
    dependencies = {
      "nvim-lua/plenary.nvim",
      "nvim-tree/nvim-web-devicons",
      "MunifTanjim/nui.nvim",
    },
    config = function()
      require("neo-tree").setup({
        close_if_last_window = true,
        window = { position = "current" },
        filesystem = { hijack_netrw_behavior = "open_current" },
      })
      vim.schedule(function() pcall(vim.cmd, "Neotree position=current") end)
    end,
  },
  {
    "nvim-telescope/telescope.nvim",
    branch = "0.1.x",
    dependencies = { "nvim-lua/plenary.nvim" },
    config = function() require("telescope").setup({}) end,
  },
}, { ui = { border = "rounded" } })

-- Search + navigation keymaps (work from the tree and from any open file).
local map = vim.keymap.set
map("n", "<C-p>", "<cmd>Telescope find_files<cr>", { desc = "Find files (name)" })
map("n", "<C-g>", "<cmd>Telescope live_grep<cr>", { desc = "Search contents (grep)" })
map("n", "<C-e>", "<cmd>Neotree toggle position=current<cr>", { desc = "Toggle file tree" })
"#;
