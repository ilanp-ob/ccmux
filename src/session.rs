use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaudeCodeStatus {
    Working,
    WaitingInput,
    Idle,
    Unknown,
}

impl ClaudeCodeStatus {
    pub fn icon(&self) -> &'static str {
        match self {
            Self::Working => "●",
            Self::WaitingInput => "◐",
            Self::Idle => "○",
            Self::Unknown => "?",
        }
    }

    pub fn needs_attention(&self) -> bool {
        matches!(self, Self::WaitingInput)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaneType {
    Claude,
    Ocli,
    Other(String),
}

/// A single tmux pane running a detected command (claude, ocli, etc.)
#[derive(Debug, Clone)]
pub struct DetectedPane {
    /// tmux pane ID, e.g. "%12"
    pub pane_id: String,
    /// tmux window ID, e.g. "@5"
    pub window_id: String,
    pub window_name: String,
    /// tmux window index (for targeting)
    pub window_index: String,
    /// true if this pane currently has keyboard focus
    pub pane_active: bool,
    pub current_command: String,
    pub current_path: PathBuf,
    pub pane_type: PaneType,
    pub status: ClaudeCodeStatus,
    /// None = default tmux server; Some(name) = tmux -L name
    pub server: Option<String>,
    /// sequential display number assigned at render time (1-based)
    pub display_num: usize,
}

impl DetectedPane {
    pub fn git_branch(&self) -> Option<String> {
        let repo = git2::Repository::discover(&self.current_path).ok()?;
        let head = repo.head().ok()?;
        head.shorthand().map(|s| s.to_string())
    }
}

/// All detected panes in a single tmux window, from one server
#[derive(Debug, Clone)]
pub struct WindowGroup {
    pub window_id: String,
    pub window_name: String,
    /// None = default server
    pub server: Option<String>,
    pub panes: Vec<DetectedPane>,
    /// Number of non-Claude, non-sidebar panes in this window (e.g. zsh, cargo, nvim).
    pub extra_pane_count: usize,
}
