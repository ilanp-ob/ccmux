use crate::git::BranchEntry;

/// All UI modes the sidebar can be in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    /// Normal browsing — navigating the pane list
    Normal,
    /// Showing inline action hints for selected pane
    ActionHints,
    /// Confirming a destructive action
    Confirm(ConfirmKind),
    /// Help overlay
    Help,
    /// Composing a free-text message to send to the selected Claude session
    Compose { text: String, cursor: usize },
    /// Creating a new plain tmux window
    NewWindow { name: String, color_idx: usize, launch_claude: bool, field: u8, name_cursor: usize },
    /// Editing an existing window's name and color
    EditWindow { window_id: String, name: String, color_idx: usize, field: u8, name_cursor: usize },
    /// Multi-step worktree creation flow
    WorktreeFlow(WorktreeStep),
    /// Action menu for the selected pane (PR ops, delete worktree, …)
    ActionMenu { items: Vec<ActionItem>, cursor: usize },
    /// Folder picker for creating a new session in a chosen directory
    FolderPick(FolderPickStep),
    /// Browsing Claude session history for the selected pane's repo.
    History(HistoryStep),
}

// ─── Worktree flow ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorktreeStep {
    /// Background fetch in progress (thread stored in App.fetch_handle)
    Fetching,
    /// Branch picker: searchable list + "new branch" option
    BranchSelect {
        repo_root: String,
        branches: Vec<BranchEntry>,
        /// Live filter string typed by the user
        filter: String,
        /// Text cursor within `filter`
        filter_cursor: usize,
        /// Cursor position in the filtered list
        cursor: usize,
        /// If true, user has selected "[New branch]" and is typing the name
        entering_new: bool,
        new_branch_text: String,
        /// Text cursor within `new_branch_text`
        new_branch_cursor: usize,
    },
    /// Edit the full worktree path (pre-filled alongside main repo; user can change it freely)
    FolderName { repo_root: String, branch: String, folder: String, cursor: usize, base_branch: Option<String> },
    /// Choose launch options.
    /// `existing_wt_path` is set when the worktree already exists — skips `git worktree add`.
    Options {
        repo_root: String,
        branch: String,
        folder: String,
        opts: WorktreeOpts,
        existing_wt_path: Option<String>,
    },
    /// Execution in progress
    Executing { status: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeOpts {
    pub model_idx: usize,
    pub effort_idx: usize,
    pub launch_claude: bool,
    pub color_idx: usize,
    pub open_vscode: bool,
    /// Which field has focus: 0=model 1=effort 2=launch_claude 3=color 4=vscode [5=base_branch if new]
    pub field: u8,
    /// Set when creating a brand-new branch — the ref to fork from (e.g. "origin/main").
    /// None for existing/remote-tracking branches.
    pub base_branch: Option<String>,
    pub base_branch_cursor: usize,
}

impl Default for WorktreeOpts {
    fn default() -> Self {
        Self {
            model_idx: 3,      // claude-sonnet-4-6
            effort_idx: 2,     // high
            launch_claude: true,
            color_idx: 0,
            open_vscode: false,
            field: 0,
            base_branch: None,
            base_branch_cursor: 0,
        }
    }
}

// ─── Folder pick flow ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FolderPickStep {
    /// Background directory scan in progress
    Scanning,
    /// Directory list ready; user is filtering
    Picking {
        root: std::path::PathBuf,
        dirs: Vec<std::path::PathBuf>,
        filter: String,
        /// Text cursor within `filter`
        filter_cursor: usize,
        /// List selection cursor
        cursor: usize,
    },
    /// Confirm launch options before creating the window
    Options {
        path: std::path::PathBuf,
        /// Whether the directory needs to be created (typed name that doesn't exist yet)
        is_new: bool,
        opts: WorktreeOpts,
    },
}

// ─── History flow ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryStep {
    /// Background scan of ~/.claude/projects in progress.
    Loading,
    /// Sessions loaded; user is filtering/navigating.
    List {
        entries: Vec<crate::history::SessionEntry>,
        /// Repo main root — used as the resume fallback dir for dead worktrees.
        repo_root: String,
        filter: String,
        filter_cursor: usize,
        cursor: usize,
    },
}

// ─── Action menu ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionItem {
    CreatePR,
    ViewPR,
    MergePR,
    ClosePR,
    DeleteWorktree { worktree_path: String, repo_root: String },
    /// Send arbitrary text to the selected pane or job (used for smart choice menus)
    SendText { label: String, text: String },
}

impl ActionItem {
    pub fn label(&self) -> String {
        match self {
            Self::CreatePR    => "Create PR  (gh pr create --fill)".into(),
            Self::ViewPR      => "View PR    (gh pr view)".into(),
            Self::MergePR     => "Merge PR   (gh pr merge --auto)".into(),
            Self::ClosePR     => "Close PR   (gh pr close)".into(),
            Self::DeleteWorktree { worktree_path, .. } =>
                format!("Delete worktree  ({})", worktree_path),
            Self::SendText { label, .. } => label.clone(),
        }
    }
}

// ─── Confirm ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmKind {
    KillWindow { window_id: String, window_name: String },
    KillAndDeleteWorktree { window_id: String, path: String },
    DeleteWorktree { repo_root: String, worktree_path: String },
}
