# ccmux Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rename claude-tmux to ccmux and extend with config system, multi-server tmux support, ocli pane detection, and enhanced worktree workflow.

**Architecture:** Incremental extension of existing Rust TUI codebase. New features live in new modules (`config.rs`, `workflow/worktree.rs`). Existing modules (`tmux.rs`, `session.rs`, `detection.rs`, `app/mod.rs`, `input.rs`, `ui/`) are modified to thread new capabilities through.

**Tech Stack:** Rust, ratatui 0.29, crossterm 0.28, git2, serde + toml (new), clap (new)

---

## File Structure

### New files
- `src/config.rs` — Config loading, saving, first-run prompt, TOML serde structs
- `src/workflow/mod.rs` — Re-exports for workflow module
- `src/workflow/worktree.rs` — WorktreeFlow state machine, folder derivation, execution logic

### Modified files
- `Cargo.toml` — Rename package, update metadata, add `serde`, `toml`, `clap` deps
- `LICENSE` — Add fork copyright notice
- `README.md` — Update name, description, URLs
- `src/main.rs` — Add `config` and `workflow` modules, clap arg parsing, config loading
- `src/tmux.rs` — Add `server` param to all functions, add `discover_servers()`, add `new_window()` and `send_keys()` helpers
- `src/session.rs` — Add `PaneType` enum, add `pane_type` and `server` fields to `Session`
- `src/detection.rs` — Add `detect_ocli_status()` function
- `src/app/mod.rs` — Add `Config` field, integrate `WorktreeFlow` mode, update `Tmux::` calls with server, add ocli to `tick_status`
- `src/app/mode.rs` — Add `WorktreeFlow` mode with sub-states, add `WorktreeFlowField` enum
- `src/app/helpers.rs` — Add `derive_folder_name()` for smart Houston/generic folder naming
- `src/input.rs` — Add `handle_worktree_flow_mode()` handler
- `src/ui/mod.rs` — Rename title to "ccmux", add server tags, add ocli display, add `WorktreeFlow` modal dispatch, update footer hints
- `src/ui/dialogs.rs` — Add `render_worktree_flow_dialog()` for the multi-step workflow UI
- `src/ui/help.rs` — Add `w` keybinding to help text
- `src/git/worktree.rs` — Add `list_remote_branches()` to include remote branches in selection
- `src/completion.rs` — No changes needed (reuse existing branch/path completion)

---

### Task 1: Rename to ccmux + Add Dependencies

**Files:**
- Modify: `Cargo.toml`
- Modify: `LICENSE`
- Modify: `README.md`
- Modify: `src/ui/mod.rs:129-133` (header title)

- [ ] **Step 1: Update Cargo.toml**

```toml
[package]
name = "ccmux"
version = "0.5.0"
edition = "2021"
description = "TUI for managing Claude Code tmux sessions"
license = "AGPL-3.0-only"
authors = ["Ilan Peretz", "Niels Groeneveld (original author)"]
repository = "https://github.com/ilanp-ob/ccmux"
homepage = "https://github.com/ilanp-ob/ccmux"
readme = "README.md"
keywords = ["tmux", "cli", "tui", "claude", "terminal"]
categories = ["command-line-utilities", "development-tools"]
exclude = [".claude/", "docs/", ".github/"]

[dependencies]
ratatui = "0.29"
crossterm = "0.28"
anyhow = "1.0"
dirs = "5.0"
unicode-width = "0.2"
ansi-to-tui = "7.0"
git2 = "0.20"
serde = { version = "1.0", features = ["derive"] }
toml = "0.8"
clap = { version = "4", features = ["derive"] }
```

- [ ] **Step 2: Update LICENSE**

Add after the existing copyright line (`Copyright (C) 2026 Niels Groeneveld`):

```
Copyright (C) 2026 Ilan Peretz (fork — ccmux)
Based on claude-tmux by Niels Groeneveld (https://github.com/nielsgroen/claude-tmux)
```

- [ ] **Step 3: Update UI header title**

In `src/ui/mod.rs`, change the `render_header` function (line ~130):

```rust
let title = format!(
    "─ ccmux ─{:─>width$}",
    current,
    width = area.width as usize - 9
);
```

(Change `"claude-tmux"` to `"ccmux"` and adjust width from 15 to 9 for the shorter name.)

- [ ] **Step 4: Update README.md**

Replace the entire README with ccmux branding. Keep the same structure but update:
- Title: `# ccmux`
- Description: mention it's a fork, what's added
- Installation: `cargo install ccmux` and `~/.cargo/bin/ccmux`
- tmux.conf: `bind-key C-c display-popup -E -w 80 -h 30 "~/.cargo/bin/ccmux"`
- Repository URLs to `https://github.com/ilanp-ob/ccmux`
- Credit original: "Based on [claude-tmux](https://github.com/nielsgroen/claude-tmux) by Niels Groeneveld."

- [ ] **Step 5: Verify it compiles**

Run: `cargo build 2>&1 | tail -5`
Expected: `Compiling ccmux v0.5.0` with no errors.

- [ ] **Step 6: Run existing tests**

Run: `cargo test 2>&1 | tail -10`
Expected: All existing tests pass.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml LICENSE README.md src/ui/mod.rs
git commit -m "rename: claude-tmux → ccmux with updated authorship and deps"
```

---

### Task 2: Configuration System

**Files:**
- Create: `src/config.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: Create `src/config.rs`**

```rust
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_claude")]
    pub claude: ClaudeConfig,
    #[serde(default = "default_worktree")]
    pub worktree: WorktreeConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeConfig {
    #[serde(default = "default_alias")]
    pub alias: String,
    #[serde(default = "default_model")]
    pub default_model: String,
    #[serde(default = "default_effort")]
    pub default_effort: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeConfig {
    #[serde(default = "default_base_dir")]
    pub base_dir: String,
    #[serde(default = "default_houston_path")]
    pub houston_path: String,
    #[serde(default = "default_worktree_defaults")]
    pub defaults: WorktreeDefaults,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeDefaults {
    #[serde(default = "default_base_branch")]
    pub base_branch: String,
}

fn default_claude() -> ClaudeConfig {
    ClaudeConfig {
        alias: default_alias(),
        default_model: default_model(),
        default_effort: default_effort(),
    }
}

fn default_worktree() -> WorktreeConfig {
    WorktreeConfig {
        base_dir: default_base_dir(),
        houston_path: default_houston_path(),
        defaults: default_worktree_defaults(),
    }
}

fn default_worktree_defaults() -> WorktreeDefaults {
    WorktreeDefaults {
        base_branch: default_base_branch(),
    }
}

fn default_alias() -> String { "c".to_string() }
fn default_model() -> String { "claude-opus-4-6".to_string() }
fn default_effort() -> String { "high".to_string() }
fn default_base_dir() -> String { "~/dev".to_string() }
fn default_houston_path() -> String { "~/dev/houston".to_string() }
fn default_base_branch() -> String { "origin/master".to_string() }

pub const AVAILABLE_MODELS: &[&str] = &[
    "claude-opus-4-7",
    "claude-opus-4-6",
    "claude-sonnet-4-6",
    "claude-haiku-4-5",
];

pub const AVAILABLE_EFFORTS: &[&str] = &[
    "low",
    "medium",
    "high",
    "max",
    "auto",
];

impl Default for Config {
    fn default() -> Self {
        Config {
            claude: default_claude(),
            worktree: default_worktree(),
        }
    }
}

impl Config {
    pub fn config_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("~/.config"))
            .join("ccmux")
            .join("config.toml")
    }

    pub fn load() -> Result<Self> {
        let path = Self::config_path();
        if !path.exists() {
            return Ok(Config::default());
        }
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read config at {}", path.display()))?;
        let config: Config = toml::from_str(&content)
            .with_context(|| format!("Failed to parse config at {}", path.display()))?;
        Ok(config)
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create config directory {}", parent.display()))?;
        }
        let content = toml::to_string_pretty(self)
            .context("Failed to serialize config")?;
        std::fs::write(&path, content)
            .with_context(|| format!("Failed to write config to {}", path.display()))?;
        Ok(())
    }

    pub fn exists() -> bool {
        Self::config_path().exists()
    }
}
```

- [ ] **Step 2: Register the module in `src/main.rs`**

Add `mod config;` after the existing module declarations (line 1-8). The full module list becomes:

```rust
mod app;
mod completion;
mod config;
mod detection;
mod git;
mod input;
mod scroll_state;
mod session;
mod tmux;
mod ui;
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo build 2>&1 | tail -5`
Expected: Compiles with no errors (config module is loaded but not yet used by App).

- [ ] **Step 4: Commit**

```bash
git add src/config.rs src/main.rs
git commit -m "feat: add configuration system with TOML persistence"
```

---

### Task 3: Multi-Server tmux Support

**Files:**
- Modify: `src/tmux.rs`
- Modify: `src/session.rs`
- Modify: `src/app/mod.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: Add `server` field to `Session` in `src/session.rs`**

Add after the `git_context` field (line ~83):

```rust
    /// The tmux server name this session belongs to (None = default server)
    pub server: Option<String>,
```

- [ ] **Step 2: Refactor `src/tmux.rs` — add server param to all functions**

Replace the entire `Tmux` impl with server-aware versions. Key changes:

1. Add helper to build tmux `Command` with optional `-L`:

```rust
impl Tmux {
    fn cmd(server: Option<&str>) -> Command {
        let mut cmd = Command::new("tmux");
        if let Some(s) = server {
            cmd.args(["-L", s]);
        }
        cmd
    }
```

2. Change every function signature to accept `server: Option<&str>` as first parameter and use `Self::cmd(server)` instead of `Command::new("tmux")`.

3. `list_sessions(server)` — takes server, sets `session.server = server.map(|s| s.to_string())` on each created Session.

4. Add `discover_servers()`:

```rust
    pub fn discover_servers() -> Vec<String> {
        let uid = unsafe { libc::getuid() };
        let socket_dir = PathBuf::from(format!("/tmp/tmux-{}", uid));
        if !socket_dir.is_dir() {
            return vec![];
        }
        let mut servers = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&socket_dir) {
            for entry in entries.flatten() {
                if let Some(name) = entry.file_name().to_str() {
                    if !name.starts_with('.') {
                        servers.push(name.to_string());
                    }
                }
            }
        }
        servers.sort();
        servers
    }
```

Note: Add `libc = "0.2"` to `Cargo.toml` dependencies for `getuid()`.

5. Add `list_all_sessions()` that discovers servers and calls `list_sessions` for each:

```rust
    pub fn list_all_sessions(filter_server: Option<&str>) -> Result<Vec<Session>> {
        let servers = match filter_server {
            Some(s) => vec![s.to_string()],
            None => Self::discover_servers(),
        };

        // Also try the default server (no -L flag)
        let mut all_sessions = Vec::new();

        for server_name in &servers {
            if let Ok(sessions) = Self::list_sessions(Some(server_name)) {
                all_sessions.extend(sessions);
            }
        }

        // Sort: attached first, then by server, then by name
        all_sessions.sort_by(|a, b| {
            b.attached
                .cmp(&a.attached)
                .then_with(|| a.server.cmp(&b.server))
                .then_with(|| a.name.cmp(&b.name))
                .then_with(|| a.window_label.cmp(&b.window_label))
        });

        Ok(all_sessions)
    }
```

6. Add `new_window()` and `send_keys()` for the worktree workflow:

```rust
    pub fn new_window(
        server: Option<&str>,
        session: &str,
        window_name: &str,
        path: &std::path::Path,
    ) -> Result<()> {
        let path_str = path.to_string_lossy();
        let status = Self::cmd(server)
            .args(["new-window", "-t", session, "-n", window_name, "-c", &path_str])
            .status()
            .context("Failed to create new window")?;
        if !status.success() {
            anyhow::bail!("Failed to create window '{}' in session '{}'", window_name, session);
        }
        Ok(())
    }

    pub fn send_keys(
        server: Option<&str>,
        target: &str,
        keys: &str,
    ) -> Result<()> {
        let status = Self::cmd(server)
            .args(["send-keys", "-t", target, keys, "Enter"])
            .status()
            .context("Failed to send keys")?;
        if !status.success() {
            anyhow::bail!("Failed to send keys to '{}'", target);
        }
        Ok(())
    }
```

7. Update `current_session` to take `server: Option<&str>`:

```rust
    pub fn current_session(server: Option<&str>) -> Result<Option<String>> {
        let output = Self::cmd(server)
            .args(["display-message", "-p", "#{session_name}"])
            .output()
            .context("Failed to get current session")?;
        // ... rest unchanged
    }
```

8. Similarly update: `switch_to_session`, `new_session`, `kill_session`, `rename_session`, `capture_pane`, `list_panes`.

- [ ] **Step 3: Add `libc` dependency to `Cargo.toml`**

Add to `[dependencies]`:
```toml
libc = "0.2"
```

- [ ] **Step 4: Update `src/app/mod.rs` to use multi-server**

1. Add `server_filter: Option<String>` field to `App` struct.
2. Change `App::new()` to accept `server_filter: Option<String>`:

```rust
pub fn new(server_filter: Option<String>) -> Result<Self> {
    let sessions = Tmux::list_all_sessions(server_filter.as_deref())?;
    // Try to detect current session from any server
    let current_session = {
        let servers = Tmux::discover_servers();
        let mut found = None;
        for s in &servers {
            if let Ok(Some(name)) = Tmux::current_session(Some(s)) {
                found = Some(name);
                break;
            }
        }
        found
    };
    // ... rest similar, store server_filter
```

3. Update `refresh_sessions()` to use `Tmux::list_all_sessions(self.server_filter.as_deref())`.

4. Update all `Tmux::` calls to pass the session's server:
   - `switch_to_session`: `Tmux::switch_to_session(session.server.as_deref(), &target)`
   - `kill_session`: `Tmux::kill_session(session.server.as_deref(), &session_name)`
   - `rename_session`: similarly
   - `capture_pane`: `Tmux::capture_pane(session.server.as_deref(), &pane_id, ...)`
   - `new_session`: `Tmux::new_session(session.server.as_deref(), ...)`

5. In `tick_status()`, collect `(session_index, pane_id, server)` tuples and pass server to `capture_pane`.

- [ ] **Step 5: Add CLI args in `src/main.rs`**

```rust
use clap::Parser;

#[derive(Parser)]
#[command(name = "ccmux", version, about = "TUI for managing Claude Code tmux sessions")]
struct Cli {
    /// Filter to a specific tmux server (e.g., --server work)
    #[arg(long)]
    server: Option<String>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend)?;
    let result = run(&mut terminal, cli.server);
    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;
    result
}

fn run(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, server_filter: Option<String>) -> Result<()> {
    let mut app = App::new(server_filter)?;
    // ... rest unchanged
}
```

- [ ] **Step 6: Update UI to show server tags**

In `src/ui/mod.rs`, in `render_session_list`, when building the session display line:

After the marker span, add server tag if there are multiple servers:

```rust
// Determine if we should show server tags (more than 1 unique server)
let show_server_tags = {
    let mut servers: Vec<_> = app.sessions.iter().filter_map(|s| s.server.as_deref()).collect();
    servers.dedup();
    servers.len() > 1
};

// ... inside the loop, before the name span:
if show_server_tags {
    if let Some(ref server) = session.server {
        line_spans.push(Span::styled(
            format!("[{}] ", server),
            Style::default().fg(Color::Magenta),
        ));
    }
}
```

- [ ] **Step 7: Verify it compiles and run tests**

Run: `cargo build 2>&1 | tail -5`
Run: `cargo test 2>&1 | tail -10`
Expected: Compiles and tests pass.

- [ ] **Step 8: Commit**

```bash
git add -A
git commit -m "feat: multi-server tmux support with auto-discovery"
```

---

### Task 4: ocli (ops-cli) Detection and Display

**Files:**
- Modify: `src/session.rs`
- Modify: `src/tmux.rs`
- Modify: `src/detection.rs`
- Modify: `src/app/mod.rs`
- Modify: `src/app/mode.rs`
- Modify: `src/ui/mod.rs`

- [ ] **Step 1: Add `PaneType` to `src/session.rs`**

Add after `ClaudeCodeStatus` enum:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PaneType {
    #[default]
    Claude,
    Ocli,
}

impl PaneType {
    pub fn label(&self) -> &'static str {
        match self {
            PaneType::Claude => "claude",
            PaneType::Ocli => "ocli",
        }
    }
}
```

Add to `Session` struct after `claude_code_pane`:

```rust
    /// What type of tool is running in the detected pane
    pub pane_type: PaneType,
```

- [ ] **Step 2: Add ocli detection helper in `src/tmux.rs`**

Add a helper to classify pane commands:

```rust
use crate::session::PaneType;

fn classify_pane_command(command: &str) -> Option<PaneType> {
    if command == "claude" || command.contains("claude") {
        Some(PaneType::Claude)
    } else if command == "ops-cli" || command == "ocli" || command == "ops" || command.contains("ops-cli") {
        Some(PaneType::Ocli)
    } else {
        None
    }
}
```

Update `list_sessions` to use this classifier instead of only matching "claude". For sessions with detected panes, set `pane_type` on the `Session`.

Where the current code does:
```rust
let claude_panes: Vec<&Pane> = panes.iter()
    .filter(|p| p.current_command == "claude" || p.current_command.contains("claude"))
    .collect();
```

Replace with:
```rust
let detected_panes: Vec<(&Pane, PaneType)> = panes.iter()
    .filter_map(|p| classify_pane_command(&p.current_command).map(|t| (p, t)))
    .collect();
```

Then iterate over `detected_panes` instead of `claude_panes`, setting `session.pane_type` accordingly. The field `claude_code_pane` still stores the pane ID (rename is optional — keeping it avoids a huge refactor).

- [ ] **Step 3: Add ocli status detection in `src/detection.rs`**

```rust
pub fn detect_ocli_status(content: &str) -> ClaudeCodeStatus {
    // ocli uses Ink (React-based TUI). Look for interactive prompt indicators.
    // When idle, it shows a menu with selectable items or a command prompt.
    // When working, it shows progress indicators or streaming output.

    if content.contains("Select") || content.contains("❯") || content.contains("Choose") {
        return ClaudeCodeStatus::Idle;
    }

    if content.contains("Deploying") || content.contains("Tailing") || content.contains("Loading") || content.contains("Fetching") {
        return ClaudeCodeStatus::Working;
    }

    if content.contains("[y/n]") || content.contains("[Y/n]") || content.contains("Confirm") {
        return ClaudeCodeStatus::WaitingInput;
    }

    ClaudeCodeStatus::Unknown
}
```

- [ ] **Step 4: Update `tick_status` in `src/app/mod.rs` to handle ocli panes**

Change the target collection to include pane_type:

```rust
let targets: Vec<(usize, String, PaneType)> = self
    .sessions
    .iter()
    .enumerate()
    .filter_map(|(i, s)| {
        s.claude_code_pane.as_ref().map(|id| (i, id.clone(), s.pane_type))
    })
    .collect();
```

In the status detection loop, branch on pane type:

```rust
let status = match pane_type {
    PaneType::Ocli => detect_ocli_status(&content),
    PaneType::Claude => match self.pane_content_cache.get(&pane_id) {
        Some(prev) if prev != &content => ClaudeCodeStatus::Working,
        Some(_) => detect_static_status(&content),
        None => detect_status(&content),
    },
};
```

- [ ] **Step 5: Filter actions for ocli in `compute_actions`**

In `src/app/mod.rs`, in `compute_actions()`, check the pane type of the selected session. If it's `Ocli`, only offer `SwitchTo`, `Rename`, and `Kill` (no git/PR actions):

```rust
let pane_type = self.selected_session().map(|s| s.pane_type).unwrap_or_default();

if pane_type == PaneType::Ocli {
    let mut actions = vec![SessionAction::SwitchTo, SessionAction::Rename, SessionAction::Kill];
    self.available_actions = actions;
    self.selected_action = 0;
    return;
}
```

Add this check at the beginning of `compute_actions`, before the git logic.

- [ ] **Step 6: Update UI to show ocli label**

In `src/ui/mod.rs`, in the session list rendering, add a type indicator before the status symbol when the pane is ocli:

```rust
if session.pane_type == PaneType::Ocli {
    line_spans.push(Span::styled(
        "[ocli] ",
        Style::default().fg(Color::Blue),
    ));
}
```

- [ ] **Step 7: Verify and commit**

Run: `cargo build 2>&1 | tail -5`
Run: `cargo test 2>&1 | tail -10`
Expected: Compiles and tests pass.

```bash
git add -A
git commit -m "feat: detect and display ops-cli (ocli) panes"
```

---

### Task 5: Smart Folder Name Derivation

**Files:**
- Modify: `src/app/helpers.rs`

- [ ] **Step 1: Add `derive_folder_name` to `src/app/helpers.rs`**

```rust
use std::path::Path;

/// Derive a short folder name from a branch name for worktree creation.
///
/// For Houston repos: extract OPS-XXXXX Jira ticket → `ops-xxxxx`.
/// If no ticket, use `ops-` prefix + slug from branch.
/// For other repos: short slug from branch name.
pub fn derive_folder_name(branch: &str, is_houston: bool) -> String {
    let stripped = strip_branch_prefix(branch);

    if is_houston {
        // Try to extract Jira ticket pattern OPS-\d+
        if let Some(ticket) = extract_jira_ticket(branch) {
            return ticket.to_lowercase();
        }
        // No ticket — use ops- prefix + slug
        let slug = slugify(stripped, 25);
        return format!("ops-{}", slug);
    }

    // Non-houston: just a slug
    slugify(stripped, 30)
}

fn strip_branch_prefix(branch: &str) -> &str {
    let prefixes = [
        "feature/", "fix/", "bugfix/", "hotfix/",
        "chore/", "refactor/", "docs/", "test/",
        "ci/", "build/", "perf/", "style/",
    ];
    for prefix in &prefixes {
        if let Some(rest) = branch.strip_prefix(prefix) {
            return rest;
        }
    }
    // Also strip origin/ prefix for remote branches
    if let Some(rest) = branch.strip_prefix("origin/") {
        return strip_branch_prefix(rest);
    }
    branch
}

fn extract_jira_ticket(branch: &str) -> Option<String> {
    // Match OPS-\d+ anywhere in the branch name
    let upper = branch.to_uppercase();
    let mut start = 0;
    while let Some(pos) = upper[start..].find("OPS-") {
        let abs_pos = start + pos;
        let rest = &upper[abs_pos + 4..];
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if !digits.is_empty() {
            return Some(format!("ops-{}", digits));
        }
        start = abs_pos + 4;
    }
    None
}

fn slugify(s: &str, max_len: usize) -> String {
    let slug: String = s
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    // Collapse consecutive hyphens
    let mut result = String::new();
    let mut prev_hyphen = false;
    for c in slug.chars() {
        if c == '-' {
            if !prev_hyphen && !result.is_empty() {
                result.push('-');
            }
            prev_hyphen = true;
        } else {
            result.push(c);
            prev_hyphen = false;
        }
    }
    // Trim trailing hyphen
    let trimmed = result.trim_end_matches('-');
    if trimmed.len() > max_len {
        // Try to cut at a hyphen boundary
        let cut = &trimmed[..max_len];
        cut.rfind('-')
            .map(|i| cut[..i].to_string())
            .unwrap_or_else(|| cut.to_string())
    } else {
        trimmed.to_string()
    }
}

/// Check if a repo path is the houston repo by comparing resolved paths.
pub fn is_houston_repo(repo_path: &Path, houston_config_path: &str) -> bool {
    let expanded_houston = expand_path(houston_config_path);
    // Compare canonical paths to handle symlinks / different representations
    let repo_canon = std::fs::canonicalize(repo_path).unwrap_or_else(|_| repo_path.to_path_buf());
    let houston_canon = std::fs::canonicalize(&expanded_houston).unwrap_or(expanded_houston);
    repo_canon == houston_canon
}
```

- [ ] **Step 2: Add unit tests for folder derivation**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_houston_with_jira_ticket() {
        assert_eq!(derive_folder_name("feature/OPS-12345-fix-scheduling", true), "ops-12345");
        assert_eq!(derive_folder_name("OPS-999-quick-fix", true), "ops-999");
        assert_eq!(derive_folder_name("hotfix/OPS-42-urgent", true), "ops-42");
    }

    #[test]
    fn test_houston_without_jira_ticket() {
        assert_eq!(derive_folder_name("fix/remove-dead-code", true), "ops-remove-dead-code");
        assert_eq!(derive_folder_name("feature/add-logging", true), "ops-add-logging");
    }

    #[test]
    fn test_non_houston() {
        assert_eq!(derive_folder_name("feature/add-search-bar", false), "add-search-bar");
        assert_eq!(derive_folder_name("fix/remove-dead-code", false), "remove-dead-code");
        assert_eq!(derive_folder_name("main", false), "main");
    }

    #[test]
    fn test_remote_branch_prefix() {
        assert_eq!(derive_folder_name("origin/feature/OPS-123-foo", true), "ops-123");
        assert_eq!(derive_folder_name("origin/feature/add-bar", false), "add-bar");
    }

    #[test]
    fn test_slugify_truncation() {
        let long_name = "this-is-a-very-long-branch-name-that-exceeds-the-limit";
        let result = derive_folder_name(&format!("feature/{}", long_name), false);
        assert!(result.len() <= 30);
        assert!(!result.ends_with('-'));
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test app::helpers 2>&1 | tail -15`
Expected: All new tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/app/helpers.rs
git commit -m "feat: smart folder name derivation with Jira ticket extraction"
```

---

### Task 6: Remote Branch Listing

**Files:**
- Modify: `src/git/worktree.rs`

- [ ] **Step 1: Add `list_all_branches` to `src/git/worktree.rs`**

Add a new function that includes remote branches:

```rust
    /// List all branches (local + remote), with remote branches prefixed by their remote name.
    pub fn list_all_branches(repo_path: &Path) -> Result<Vec<String>> {
        let repo = Repository::discover(repo_path).context("Failed to open repository")?;
        let mut branches = Vec::new();

        // Local branches
        for branch_result in repo.branches(Some(git2::BranchType::Local))? {
            let (branch, _) = branch_result?;
            if let Ok(Some(name)) = branch.name() {
                branches.push(name.to_string());
            }
        }

        // Remote branches (skip HEAD references)
        for branch_result in repo.branches(Some(git2::BranchType::Remote))? {
            let (branch, _) = branch_result?;
            if let Ok(Some(name)) = branch.name() {
                // Skip origin/HEAD
                if name.ends_with("/HEAD") {
                    continue;
                }
                branches.push(name.to_string());
            }
        }

        // Sort: main/master first, then local branches, then remote branches
        branches.sort_by(|a, b| {
            let a_is_main = a == "main" || a == "master";
            let b_is_main = b == "main" || b == "master";
            let a_is_remote = a.contains('/');
            let b_is_remote = b.contains('/');
            match (a_is_main, b_is_main) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => match (a_is_remote, b_is_remote) {
                    (false, true) => std::cmp::Ordering::Less,
                    (true, false) => std::cmp::Ordering::Greater,
                    _ => a.cmp(b),
                },
            }
        });

        Ok(branches)
    }
```

- [ ] **Step 2: Commit**

```bash
git add src/git/worktree.rs
git commit -m "feat: list local + remote branches for worktree workflow"
```

---

### Task 7: WorktreeFlow Mode and State Machine

**Files:**
- Modify: `src/app/mode.rs`
- Create: `src/workflow/mod.rs`
- Create: `src/workflow/worktree.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: Add `WorktreeFlow` mode to `src/app/mode.rs`**

Add the new mode variant to the `Mode` enum:

```rust
    /// Enhanced worktree creation workflow
    WorktreeFlow {
        state: WorktreeFlowState,
    },
```

Add the state and field enums:

```rust
/// State machine for the enhanced worktree workflow
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorktreeFlowState {
    /// Fetching remote refs (spinner shown)
    Fetching,
    /// Selecting a branch (fzf-style filter)
    BranchSelect {
        /// All branches (local + remote), populated after fetch
        all_branches: Vec<String>,
        /// Current filter input
        filter_input: String,
        /// Selected index in filtered list
        selected: Option<usize>,
        /// Whether to create a new branch
        create_new: bool,
        /// If creating new: the base branch
        base_branch: String,
        /// Which sub-field is active
        field: BranchSelectField,
    },
    /// Editing the folder name
    FolderName {
        branch: String,
        is_new_branch: bool,
        base_branch: String,
        folder: String,
    },
    /// Configuring Claude launch options
    ClaudeOptions {
        branch: String,
        is_new_branch: bool,
        base_branch: String,
        folder: String,
        model_index: usize,
        effort_index: usize,
        launch_claude: bool,
        /// Which field is active (0=model, 1=effort, 2=launch toggle)
        field: usize,
    },
    /// Executing the worktree creation
    Executing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BranchSelectField {
    Filter,
    BaseBranch,
}
```

Update the `Mode` match arms and re-exports at the top of `app/mod.rs` to include the new types:

```rust
pub use mode::{
    BranchSelectField, CreatePullRequestField, Mode, NewSessionField, NewWorktreeField,
    SessionAction, WorktreeFlowState,
};
```

- [ ] **Step 2: Create `src/workflow/mod.rs`**

```rust
pub mod worktree;
```

- [ ] **Step 3: Create `src/workflow/worktree.rs`**

This contains the flow logic methods that App will call:

```rust
use std::path::PathBuf;

use anyhow::Result;

use crate::app::helpers::{derive_folder_name, expand_path, is_houston_repo};
use crate::app::mode::{BranchSelectField, Mode, WorktreeFlowState};
use crate::app::App;
use crate::config::{Config, AVAILABLE_EFFORTS, AVAILABLE_MODELS};
use crate::git::GitContext;
use crate::tmux::Tmux;

impl App {
    /// Start the enhanced worktree workflow
    pub fn start_worktree_flow(&mut self) {
        self.clear_messages();
        let Some(session) = self.selected_session() else { return };
        let Some(ref git) = session.git_context else {
            self.error = Some("Not a git repository".to_string());
            return;
        };

        let source_repo = if git.is_worktree {
            git.main_repo_path.clone().unwrap_or_else(|| session.working_directory.clone())
        } else {
            session.working_directory.clone()
        };

        // Store source repo for later use
        self.worktree_flow_source_repo = Some(source_repo.clone());
        self.worktree_flow_server = session.server.clone();

        // Start with fetching
        self.mode = Mode::WorktreeFlow {
            state: WorktreeFlowState::Fetching,
        };

        // Do the fetch synchronously (it's usually fast)
        let _ = GitContext::fetch(&source_repo);

        // After fetch, load branches and transition to BranchSelect
        let all_branches = GitContext::list_all_branches(&source_repo)
            .unwrap_or_default();

        let base_branch = self.config.worktree.defaults.base_branch.clone();

        self.mode = Mode::WorktreeFlow {
            state: WorktreeFlowState::BranchSelect {
                all_branches,
                filter_input: String::new(),
                selected: None,
                create_new: false,
                base_branch,
                field: BranchSelectField::Filter,
            },
        };
    }

    /// Get filtered branches for the current WorktreeFlow BranchSelect state
    pub fn worktree_flow_filtered_branches(&self) -> Vec<&str> {
        if let Mode::WorktreeFlow {
            state: WorktreeFlowState::BranchSelect {
                ref all_branches,
                ref filter_input,
                ..
            },
        } = self.mode
        {
            if filter_input.is_empty() {
                all_branches.iter().map(|s| s.as_str()).collect()
            } else {
                let input_lower = filter_input.to_lowercase();
                all_branches
                    .iter()
                    .filter(|b| b.to_lowercase().contains(&input_lower))
                    .map(|s| s.as_str())
                    .collect()
            }
        } else {
            vec![]
        }
    }

    /// Transition from BranchSelect to FolderName
    pub fn worktree_flow_confirm_branch(&mut self) {
        let (branch, is_new, base_branch) = if let Mode::WorktreeFlow {
            state: WorktreeFlowState::BranchSelect {
                ref all_branches,
                ref filter_input,
                selected,
                create_new,
                ref base_branch,
                ..
            },
        } = self.mode
        {
            let filtered = self.worktree_flow_filtered_branches();

            if create_new {
                if filter_input.is_empty() {
                    self.error = Some("Branch name cannot be empty".to_string());
                    return;
                }
                (filter_input.clone(), true, base_branch.clone())
            } else if let Some(idx) = selected {
                let branch = filtered.get(idx).unwrap_or(&filter_input.as_str()).to_string();
                (branch, false, base_branch.clone())
            } else if let Some(first) = filtered.first() {
                (first.to_string(), false, base_branch.clone())
            } else {
                // No match — treat as new branch
                (filter_input.clone(), true, base_branch.clone())
            }
        } else {
            return;
        };

        // Derive folder name
        let source_repo = self.worktree_flow_source_repo.as_ref().cloned().unwrap_or_default();
        let is_houston = is_houston_repo(&source_repo, &self.config.worktree.houston_path);
        let folder = derive_folder_name(&branch, is_houston);

        self.mode = Mode::WorktreeFlow {
            state: WorktreeFlowState::FolderName {
                branch,
                is_new_branch: is_new,
                base_branch,
                folder,
            },
        };
    }

    /// Transition from FolderName to ClaudeOptions
    pub fn worktree_flow_confirm_folder(&mut self) {
        let (branch, is_new, base_branch, folder) = if let Mode::WorktreeFlow {
            state: WorktreeFlowState::FolderName {
                ref branch,
                is_new_branch,
                ref base_branch,
                ref folder,
            },
        } = self.mode
        {
            if folder.is_empty() {
                self.error = Some("Folder name cannot be empty".to_string());
                return;
            }
            (branch.clone(), is_new_branch, base_branch.clone(), folder.clone())
        } else {
            return;
        };

        // Find default indices for model and effort
        let model_index = AVAILABLE_MODELS
            .iter()
            .position(|m| *m == self.config.claude.default_model)
            .unwrap_or(1); // default to claude-opus-4-6

        let effort_index = AVAILABLE_EFFORTS
            .iter()
            .position(|e| *e == self.config.claude.default_effort)
            .unwrap_or(2); // default to "high"

        self.mode = Mode::WorktreeFlow {
            state: WorktreeFlowState::ClaudeOptions {
                branch,
                is_new_branch: is_new,
                base_branch,
                folder,
                model_index,
                effort_index,
                launch_claude: true,
                field: 0,
            },
        };
    }

    /// Execute the worktree creation
    pub fn worktree_flow_execute(&mut self) {
        let (branch, is_new_branch, base_branch, folder, model_index, effort_index, launch_claude) =
            if let Mode::WorktreeFlow {
                state: WorktreeFlowState::ClaudeOptions {
                    ref branch,
                    is_new_branch,
                    ref base_branch,
                    ref folder,
                    model_index,
                    effort_index,
                    launch_claude,
                    ..
                },
            } = self.mode
            {
                (
                    branch.clone(),
                    is_new_branch,
                    base_branch.clone(),
                    folder.clone(),
                    model_index,
                    effort_index,
                    launch_claude,
                )
            } else {
                return;
            };

        let source_repo = self.worktree_flow_source_repo.take().unwrap_or_default();
        let server = self.worktree_flow_server.take();
        let base_dir = expand_path(&self.config.worktree.base_dir);
        let worktree_path = base_dir.join(&folder);

        // Resolve the actual branch name for worktree creation
        // If it's a remote branch like "origin/feature/foo", we need to create a local tracking branch
        let (local_branch, actually_new) = if is_new_branch {
            (branch.clone(), true)
        } else if branch.contains('/') {
            // Remote branch — strip remote prefix for local name
            let local_name = branch.split('/').skip(1).collect::<Vec<_>>().join("/");
            (local_name, true) // create_worktree will create the local branch
        } else {
            (branch.clone(), false)
        };

        // Create the worktree
        match GitContext::create_worktree(&source_repo, &worktree_path, &local_branch, actually_new) {
            Ok(_) => {
                // Determine session name from tmux current session
                let current_session = Tmux::current_session(server.as_deref())
                    .ok()
                    .flatten()
                    .unwrap_or_else(|| "default".to_string());

                // Create new window in current session
                match Tmux::new_window(server.as_deref(), &current_session, &folder, &worktree_path) {
                    Ok(_) => {
                        let target = format!("{}:{}", current_session, folder);

                        if launch_claude {
                            let model = AVAILABLE_MODELS[model_index];
                            let effort = AVAILABLE_EFFORTS[effort_index];
                            let alias = &self.config.claude.alias;
                            let cmd = format!("{} --model {} --effort {}", alias, model, effort);
                            let _ = Tmux::send_keys(server.as_deref(), &target, &cmd);
                        }

                        self.refresh_sessions();
                        self.message = Some(format!(
                            "Created worktree '{}' in window '{}'",
                            local_branch, folder
                        ));
                    }
                    Err(e) => {
                        self.error = Some(format!(
                            "Worktree created but failed to create window: {}",
                            e
                        ));
                    }
                }
            }
            Err(e) => {
                self.error = Some(format!("Failed to create worktree: {}", e));
            }
        }

        self.mode = Mode::Normal;
    }
}
```

- [ ] **Step 4: Add workflow fields to `App` struct in `src/app/mod.rs`**

Add these fields to `App`:

```rust
    /// Config
    pub config: Config,
    /// Source repo for active worktree flow
    pub worktree_flow_source_repo: Option<PathBuf>,
    /// Server for active worktree flow
    pub worktree_flow_server: Option<String>,
```

Update `App::new()` to initialize them:

```rust
pub fn new(server_filter: Option<String>, config: Config) -> Result<Self> {
    // ... existing code ...
    let mut app = Self {
        // ... existing fields ...
        config,
        worktree_flow_source_repo: None,
        worktree_flow_server: None,
    };
    // ...
```

Update `main.rs` to pass config:

```rust
fn run(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, server_filter: Option<String>) -> Result<()> {
    let config = config::Config::load()?;
    if !config::Config::exists() {
        config.save()?;
    }
    let mut app = App::new(server_filter, config)?;
    // ...
```

- [ ] **Step 5: Register workflow module in `src/main.rs`**

Add `mod workflow;` to module declarations.

- [ ] **Step 6: Verify it compiles**

Run: `cargo build 2>&1 | tail -5`
Expected: Compiles (new modes not yet wired to input/UI).

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "feat: worktree flow state machine and execution logic"
```

---

### Task 8: WorktreeFlow Input Handling

**Files:**
- Modify: `src/input.rs`

- [ ] **Step 1: Add handler for WorktreeFlow mode**

Add to the match in `handle_key`:

```rust
Mode::WorktreeFlow { .. } => handle_worktree_flow_mode(app, key),
```

Add the handler function:

```rust
fn handle_worktree_flow_mode(app: &mut App, key: KeyEvent) {
    use crate::app::mode::{BranchSelectField, WorktreeFlowState};
    use crate::config::{AVAILABLE_EFFORTS, AVAILABLE_MODELS};

    let state = if let Mode::WorktreeFlow { ref state } = app.mode {
        state.clone()
    } else {
        return;
    };

    match state {
        WorktreeFlowState::Fetching => {
            // Only allow escape during fetch
            if key.code == KeyCode::Esc {
                app.cancel();
            }
        }

        WorktreeFlowState::BranchSelect { field, .. } => match field {
            BranchSelectField::Filter => match key.code {
                KeyCode::Esc => app.cancel(),
                KeyCode::Enter => app.worktree_flow_confirm_branch(),
                KeyCode::Tab => {
                    // Toggle create_new or switch to base branch field
                    if let Mode::WorktreeFlow {
                        state: WorktreeFlowState::BranchSelect {
                            ref mut create_new,
                            ref mut field,
                            ..
                        },
                    } = app.mode
                    {
                        if *create_new {
                            *field = BranchSelectField::BaseBranch;
                        } else {
                            *create_new = true;
                        }
                    }
                }
                KeyCode::Down => {
                    let count = app.worktree_flow_filtered_branches().len();
                    if count > 0 {
                        if let Mode::WorktreeFlow {
                            state: WorktreeFlowState::BranchSelect {
                                ref mut selected, ..
                            },
                        } = app.mode
                        {
                            *selected = Some(selected.map(|i| (i + 1) % count).unwrap_or(0));
                        }
                    }
                }
                KeyCode::Up => {
                    let count = app.worktree_flow_filtered_branches().len();
                    if count > 0 {
                        if let Mode::WorktreeFlow {
                            state: WorktreeFlowState::BranchSelect {
                                ref mut selected, ..
                            },
                        } = app.mode
                        {
                            *selected = Some(
                                selected
                                    .map(|i| if i == 0 { count - 1 } else { i - 1 })
                                    .unwrap_or(count - 1),
                            );
                        }
                    }
                }
                KeyCode::Backspace => {
                    if let Mode::WorktreeFlow {
                        state: WorktreeFlowState::BranchSelect {
                            ref mut filter_input,
                            ref mut selected,
                            ref mut create_new,
                            ..
                        },
                    } = app.mode
                    {
                        filter_input.pop();
                        *selected = None;
                        *create_new = false;
                    }
                }
                KeyCode::Char(c) => {
                    if let Mode::WorktreeFlow {
                        state: WorktreeFlowState::BranchSelect {
                            ref mut filter_input,
                            ref mut selected,
                            ref mut create_new,
                            ..
                        },
                    } = app.mode
                    {
                        filter_input.push(c);
                        *selected = None;
                        *create_new = false;
                    }
                }
                _ => {}
            },
            BranchSelectField::BaseBranch => match key.code {
                KeyCode::Esc => app.cancel(),
                KeyCode::Enter => app.worktree_flow_confirm_branch(),
                KeyCode::Tab | KeyCode::BackTab => {
                    if let Mode::WorktreeFlow {
                        state: WorktreeFlowState::BranchSelect { ref mut field, .. },
                    } = app.mode
                    {
                        *field = BranchSelectField::Filter;
                    }
                }
                KeyCode::Backspace => {
                    if let Mode::WorktreeFlow {
                        state: WorktreeFlowState::BranchSelect {
                            ref mut base_branch,
                            ..
                        },
                    } = app.mode
                    {
                        base_branch.pop();
                    }
                }
                KeyCode::Char(c) => {
                    if c.is_alphanumeric() || c == '-' || c == '_' || c == '/' {
                        if let Mode::WorktreeFlow {
                            state: WorktreeFlowState::BranchSelect {
                                ref mut base_branch,
                                ..
                            },
                        } = app.mode
                        {
                            base_branch.push(c);
                        }
                    }
                }
                _ => {}
            },
        },

        WorktreeFlowState::FolderName { .. } => match key.code {
            KeyCode::Esc => app.cancel(),
            KeyCode::Enter => app.worktree_flow_confirm_folder(),
            KeyCode::Backspace => {
                if let Mode::WorktreeFlow {
                    state: WorktreeFlowState::FolderName { ref mut folder, .. },
                } = app.mode
                {
                    folder.pop();
                }
            }
            KeyCode::Char(c) => {
                if c.is_alphanumeric() || c == '-' || c == '_' {
                    if let Mode::WorktreeFlow {
                        state: WorktreeFlowState::FolderName { ref mut folder, .. },
                    } = app.mode
                    {
                        folder.push(c);
                    }
                }
            }
            _ => {}
        },

        WorktreeFlowState::ClaudeOptions {
            model_index,
            effort_index,
            launch_claude,
            field,
            ..
        } => match key.code {
            KeyCode::Esc => app.cancel(),
            KeyCode::Enter => app.worktree_flow_execute(),
            KeyCode::Tab => {
                if let Mode::WorktreeFlow {
                    state: WorktreeFlowState::ClaudeOptions { ref mut field, .. },
                } = app.mode
                {
                    *field = (*field + 1) % 3;
                }
            }
            KeyCode::BackTab => {
                if let Mode::WorktreeFlow {
                    state: WorktreeFlowState::ClaudeOptions { ref mut field, .. },
                } = app.mode
                {
                    *field = if *field == 0 { 2 } else { *field - 1 };
                }
            }
            KeyCode::Left | KeyCode::Char('h') => {
                if let Mode::WorktreeFlow {
                    state: WorktreeFlowState::ClaudeOptions {
                        ref mut model_index,
                        ref mut effort_index,
                        ref mut launch_claude,
                        field,
                        ..
                    },
                } = app.mode
                {
                    match field {
                        0 => {
                            if *model_index > 0 {
                                *model_index -= 1;
                            }
                        }
                        1 => {
                            if *effort_index > 0 {
                                *effort_index -= 1;
                            }
                        }
                        2 => *launch_claude = !*launch_claude,
                        _ => {}
                    }
                }
            }
            KeyCode::Right | KeyCode::Char('l') => {
                if let Mode::WorktreeFlow {
                    state: WorktreeFlowState::ClaudeOptions {
                        ref mut model_index,
                        ref mut effort_index,
                        ref mut launch_claude,
                        field,
                        ..
                    },
                } = app.mode
                {
                    match field {
                        0 => {
                            if *model_index < AVAILABLE_MODELS.len() - 1 {
                                *model_index += 1;
                            }
                        }
                        1 => {
                            if *effort_index < AVAILABLE_EFFORTS.len() - 1 {
                                *effort_index += 1;
                            }
                        }
                        2 => *launch_claude = !*launch_claude,
                        _ => {}
                    }
                }
            }
            _ => {}
        },

        WorktreeFlowState::Executing => {
            // No input during execution
        }
    }
}
```

- [ ] **Step 2: Add `w` keybinding to normal mode**

In `handle_normal_mode`, add:

```rust
// Worktree workflow
KeyCode::Char('w') => {
    app.start_worktree_flow();
}
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo build 2>&1 | tail -5`
Expected: Compiles.

- [ ] **Step 4: Commit**

```bash
git add src/input.rs
git commit -m "feat: worktree flow input handling with all sub-states"
```

---

### Task 9: WorktreeFlow UI Rendering

**Files:**
- Modify: `src/ui/mod.rs`
- Modify: `src/ui/dialogs.rs`
- Modify: `src/ui/help.rs`

- [ ] **Step 1: Add WorktreeFlow modal dispatch in `src/ui/mod.rs`**

In the `render` function, add a match arm for `WorktreeFlow`:

```rust
Mode::WorktreeFlow { ref state } => {
    dialogs::render_worktree_flow_dialog(frame, app, state);
}
```

Update footer hints to include `WorktreeFlow`:

```rust
Mode::WorktreeFlow { .. } => "  ⏎ confirm  tab next  ←→ change  esc cancel",
```

- [ ] **Step 2: Add `render_worktree_flow_dialog` in `src/ui/dialogs.rs`**

```rust
use crate::app::mode::{BranchSelectField, WorktreeFlowState};
use crate::config::{AVAILABLE_EFFORTS, AVAILABLE_MODELS};

pub fn render_worktree_flow_dialog(
    frame: &mut Frame,
    app: &App,
    state: &WorktreeFlowState,
) {
    match state {
        WorktreeFlowState::Fetching => {
            let area = centered_rect(40, 5, frame.area());
            let block = Block::default()
                .title(" Worktree ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan));
            let text = Paragraph::new("  Fetching remote branches...")
                .block(block);
            frame.render_widget(Clear, area);
            frame.render_widget(text, area);
        }

        WorktreeFlowState::BranchSelect {
            filter_input,
            selected,
            create_new,
            base_branch,
            field,
            ..
        } => {
            let filtered = app.worktree_flow_filtered_branches();
            let branches_to_show = filtered.len().min(10);
            let extra_lines = if *create_new { 3 } else { 0 }; // base branch field
            let dialog_height = 8 + branches_to_show as u16 + extra_lines as u16;
            let area = centered_rect(70, dialog_height, frame.area());

            let block = Block::default()
                .title(" Select Branch ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan));

            let mut lines = Vec::new();

            // Filter input
            let filter_style = if *field == BranchSelectField::Filter {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };

            let new_indicator = if *create_new {
                Span::styled(" (new branch)", Style::default().fg(Color::Green))
            } else {
                Span::raw("")
            };

            lines.push(Line::from(vec![
                Span::styled("Search: ", filter_style),
                Span::styled(filter_input, Style::default().fg(Color::Yellow)),
                if *field == BranchSelectField::Filter {
                    Span::raw("_")
                } else {
                    Span::raw("")
                },
                new_indicator,
            ]));

            // Branch list
            if !filtered.is_empty() {
                lines.push(Line::styled(
                    "        ─────────────────────────────────────────",
                    Style::default().fg(Color::DarkGray),
                ));

                for (i, branch) in filtered.iter().take(10).enumerate() {
                    let is_sel = *selected == Some(i);
                    let prefix = if is_sel { "      > " } else { "        " };
                    let style = if is_sel {
                        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::DarkGray)
                    };
                    lines.push(Line::styled(format!("{}{}", prefix, branch), style));
                }

                if filtered.len() > 10 {
                    lines.push(Line::styled(
                        format!("        ... and {} more", filtered.len() - 10),
                        Style::default().fg(Color::DarkGray),
                    ));
                }

                lines.push(Line::styled(
                    "        ─────────────────────────────────────────",
                    Style::default().fg(Color::DarkGray),
                ));
            }

            // Base branch (if creating new)
            if *create_new {
                lines.push(Line::raw(""));
                let base_style = if *field == BranchSelectField::BaseBranch {
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                lines.push(Line::from(vec![
                    Span::styled("Base:   ", base_style),
                    Span::styled(base_branch, Style::default().fg(Color::Cyan)),
                    if *field == BranchSelectField::BaseBranch {
                        Span::raw("_")
                    } else {
                        Span::raw("")
                    },
                ]));
            }

            lines.push(Line::raw(""));
            lines.push(Line::styled(
                "↑↓ select  Tab new branch  Enter confirm  Esc cancel",
                Style::default().fg(Color::DarkGray),
            ));

            let paragraph = Paragraph::new(Text::from(lines))
                .block(block)
                .wrap(Wrap { trim: false });
            frame.render_widget(Clear, area);
            frame.render_widget(paragraph, area);
        }

        WorktreeFlowState::FolderName {
            branch, folder, ..
        } => {
            let area = centered_rect(60, 8, frame.area());
            let block = Block::default()
                .title(" Folder Name ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan));

            let lines = vec![
                Line::from(vec![
                    Span::styled("Branch: ", Style::default().fg(Color::DarkGray)),
                    Span::styled(branch, Style::default().fg(Color::Cyan)),
                ]),
                Line::raw(""),
                Line::from(vec![
                    Span::styled("Folder: ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                    Span::styled(folder, Style::default().fg(Color::Yellow)),
                    Span::raw("_"),
                ]),
                Line::raw(""),
                Line::styled(
                    "Edit or press Enter to confirm  Esc cancel",
                    Style::default().fg(Color::DarkGray),
                ),
            ];

            let paragraph = Paragraph::new(Text::from(lines))
                .block(block)
                .wrap(Wrap { trim: false });
            frame.render_widget(Clear, area);
            frame.render_widget(paragraph, area);
        }

        WorktreeFlowState::ClaudeOptions {
            branch,
            folder,
            model_index,
            effort_index,
            launch_claude,
            field,
            ..
        } => {
            let area = centered_rect(60, 12, frame.area());
            let block = Block::default()
                .title(" Claude Options ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan));

            let model_style = if *field == 0 {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let effort_style = if *field == 1 {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let launch_style = if *field == 2 {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };

            let model = AVAILABLE_MODELS[*model_index];
            let effort = AVAILABLE_EFFORTS[*effort_index];
            let launch_str = if *launch_claude { "YES" } else { "NO" };
            let launch_color = if *launch_claude { Color::Green } else { Color::Red };

            let lines = vec![
                Line::from(vec![
                    Span::styled("Branch: ", Style::default().fg(Color::DarkGray)),
                    Span::styled(branch, Style::default().fg(Color::Cyan)),
                    Span::raw("  →  "),
                    Span::styled(folder, Style::default().fg(Color::Magenta)),
                ]),
                Line::raw(""),
                Line::from(vec![
                    Span::styled("Model:  ", model_style),
                    Span::styled(format!("◀ {} ▶", model), Style::default().fg(Color::Yellow)),
                ]),
                Line::raw(""),
                Line::from(vec![
                    Span::styled("Effort: ", effort_style),
                    Span::styled(format!("◀ {} ▶", effort), Style::default().fg(Color::Yellow)),
                ]),
                Line::raw(""),
                Line::from(vec![
                    Span::styled("Launch: ", launch_style),
                    Span::styled(launch_str, Style::default().fg(launch_color)),
                ]),
                Line::raw(""),
                Line::styled(
                    "Tab next  ←→ change  Enter create  Esc cancel",
                    Style::default().fg(Color::DarkGray),
                ),
            ];

            let paragraph = Paragraph::new(Text::from(lines))
                .block(block)
                .wrap(Wrap { trim: false });
            frame.render_widget(Clear, area);
            frame.render_widget(paragraph, area);
        }

        WorktreeFlowState::Executing => {
            let area = centered_rect(40, 5, frame.area());
            let block = Block::default()
                .title(" Worktree ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Green));
            let text = Paragraph::new("  Creating worktree...")
                .block(block);
            frame.render_widget(Clear, area);
            frame.render_widget(text, area);
        }
    }
}
```

- [ ] **Step 3: Update help screen in `src/ui/help.rs`**

Add `w` keybinding under "Actions":

```rust
Line::raw("  w           Worktree workflow"),
```

Increase the dialog height from 21 to 22 to accommodate the new line.

- [ ] **Step 4: Verify it compiles**

Run: `cargo build 2>&1 | tail -5`
Expected: Compiles with no errors.

- [ ] **Step 5: Run all tests**

Run: `cargo test 2>&1 | tail -15`
Expected: All tests pass.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat: worktree flow UI rendering with branch select, folder name, and claude options dialogs"
```

---

### Task 10: Integration Testing and Polish

**Files:**
- Various minor fixes across all modified files

- [ ] **Step 1: Full build and test**

Run: `cargo build --release 2>&1 | tail -5`
Run: `cargo test 2>&1`
Fix any compilation errors or test failures.

- [ ] **Step 2: Test the binary launches**

Run: `cargo run -- --help`
Expected: Shows ccmux help with `--server` option.

- [ ] **Step 3: Check for unused imports/warnings**

Run: `cargo build 2>&1 | grep "warning"`
Fix any warnings.

- [ ] **Step 4: Commit final polish**

```bash
git add -A
git commit -m "chore: fix warnings and polish"
```

- [ ] **Step 5: Push to remote**

```bash
git push origin main
```
