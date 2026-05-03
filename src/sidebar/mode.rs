/// All UI modes the sidebar can be in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    /// Normal browsing — navigating the pane list
    Normal,
    /// Showing inline action hints for selected pane (expand → action row)
    ActionHints,
    /// Confirming a destructive action
    Confirm(ConfirmKind),
    /// Help overlay
    Help,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmKind {
    KillWindow { window_id: String, window_name: String },
    KillAndDeleteWorktree { window_id: String, path: String },
}
