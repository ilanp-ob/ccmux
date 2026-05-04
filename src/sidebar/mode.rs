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
    Compose { text: String },
    /// Renaming the selected window
    Rename { text: String },
    /// Creating a new plain tmux window
    NewWindow { name: String, color_idx: usize, launch_claude: bool, field: u8 },
    /// Multi-step worktree creation flow
    WorktreeFlow(WorktreeStep),
    /// Action menu for the selected pane (PR ops, delete worktree, …)
    ActionMenu { items: Vec<ActionItem>, cursor: usize },
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
        /// Cursor position in the filtered list
        cursor: usize,
        /// If true, user has selected "[New branch]" and is typing the name
        entering_new: bool,
        new_branch_text: String,
    },
    /// Edit the folder name (derived from branch, full path shown as hint)
    FolderName { repo_root: String, branch: String, folder: String },
    /// Choose launch options
    Options { repo_root: String, branch: String, folder: String, opts: WorktreeOpts },
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
    /// Which field has focus: 0=model 1=effort 2=launch_claude 3=color 4=vscode
    pub field: u8,
}

impl Default for WorktreeOpts {
    fn default() -> Self {
        Self {
            model_idx: 2,      // claude-sonnet-4-6
            effort_idx: 2,     // high
            launch_claude: true,
            color_idx: 0,
            open_vscode: false,
            field: 0,
        }
    }
}

// ─── Action menu ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionItem {
    CreatePR,
    ViewPR,
    MergePR,
    ClosePR,
    DeleteWorktree { worktree_path: String, repo_root: String },
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
