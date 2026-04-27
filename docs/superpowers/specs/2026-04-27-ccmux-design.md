# ccmux — Design Spec

Fork of [claude-tmux](https://github.com/nielsgroen/claude-tmux) by Niels Groeneveld.
Renamed to **ccmux** to avoid conflict with the original.

## Overview

ccmux extends the original TUI with:
1. **Rename** — `claude-tmux` → `ccmux` throughout
2. **Configuration system** — persistent TOML config at `~/.config/ccmux/config.toml`
3. **Multi-server tmux support** — auto-detect all `tmux -L` servers (iTerm2 profile-based isolation)
4. **ops-cli (ocli) detection** — detect and display ocli panes alongside Claude panes
5. **Enhanced worktree workflow** — multi-step dialog for creating git worktrees with Claude launch

Architecture approach: incremental extension of the existing codebase. New features live in new modules; existing patterns are preserved.

---

## 1. Rename to ccmux + Authorship

### Changes
- `Cargo.toml`: `name = "ccmux"`, binary name `ccmux`
- `Cargo.toml`: `authors = ["Ilan Peretz", "Niels Groeneveld (original author)"]`
- `Cargo.toml`: `repository` / `homepage` → `https://github.com/ilanp-ob/ccmux`
- `LICENSE`: Add `Copyright (C) 2026 Ilan Peretz (fork)` below original copyright. Add note: "Based on claude-tmux by Niels Groeneveld."
- `README.md`: Update title, description, installation instructions
- UI title bar (`src/ui/mod.rs`): Display "ccmux" instead of "claude-tmux"
- License remains AGPL-3.0 (copyleft, allows forking with attribution)

---

## 2. Configuration System

### New module: `src/config.rs`

### Config file: `~/.config/ccmux/config.toml`

```toml
[claude]
alias = "c"
default_model = "claude-opus-4-6"
default_effort = "high"

[worktree]
base_dir = "~/dev"
houston_path = "~/dev/houston"

[worktree.defaults]
base_branch = "origin/master"
```

### Behavior
- First run with no config: prompt user for claude alias (pre-filled "c"), write config with sensible defaults
- Config loaded at startup, accessible via `App` struct
- Model and effort are per-creation overridable; config sets defaults

### Available models
- `claude-opus-4-7`
- `claude-opus-4-6`
- `claude-sonnet-4-6`
- `claude-haiku-4-5`

### Available effort levels
- `low`, `medium`, `high`, `max`, `auto`

---

## 3. Multi-Server tmux Support

### Problem
User's zshrc creates separate tmux servers per iTerm2 profile via `tmux -L <session_name>`. Current code uses plain `tmux` commands (default server only).

### Auto-discovery
On startup and each refresh, scan `/tmp/tmux-$(id -u)/` for active server sockets. Each socket name corresponds to a tmux server.

### Changes to `src/tmux.rs`
Every tmux command gains a `server: Option<&str>` parameter. When `Some("work")`, prepends `-L work` to the command invocation.

Affected functions: `list_sessions`, `list_panes`, `capture_pane`, `new_session`, `kill_session`, `rename_session`, `switch_to_session`, `current_session`.

### UI
- Sessions tagged with server name: `[work] my-session`
- If only one server active, no tag shown
- New windows created on the same server the user is currently attached to

### CLI flag
`ccmux --server <name>` — filter to a single server. Default: show all.

---

## 4. ops-cli (ocli) Detection and Display

### Pane detection
Extend pane scanning in `src/tmux.rs` to match commands: `ops-cli`, `ocli`, `ops`.

### New type: `PaneType`
```rust
enum PaneType {
    Claude,
    Ocli,
}
```

Carried on each `Session` entry so UI and action logic can differentiate.

### Status detection for ocli (`src/detection.rs`)
Basic heuristic from pane content:
- **Idle**: Menu/prompt visible, waiting for input
- **Working**: Active operation (deployment, tailing, etc.)
- **Unknown**: Can't determine

Exact patterns to be refined by inspecting actual ocli output. Simpler than Claude's content-change detection.

### UI
- ocli panes show with `[ocli]` label prefix in the session list
- Different status symbols or color to distinguish from Claude
- Actions for ocli panes: only SwitchTo, Kill (no git/PR actions)

---

## 5. Enhanced Worktree Workflow

### Trigger
Keybinding `w` in Normal mode (or from action menu). Available when a session has a git repo detected.

### New module: `src/workflow/worktree.rs`
Houses the multi-step state machine, keeping `app/mod.rs` from growing further.

### New mode: `WorktreeFlow`
Sub-states: `Fetching` → `BranchSelect` → `FolderName` → `ClaudeOptions` → `Executing`

### Step 1: Git Fetch
Runs automatically when the dialog opens. Shows a spinner/status while fetching. Ensures branch list is up to date.

### Step 2: Branch Selection
Fuzzy-searchable list of all local + remote branches.

Three options:
1. **`[new branch]`** — prompts for name, then base branch selection (default: `origin/master`, also fuzzy-searchable)
2. **Existing local branch** — use directly
3. **Remote branch** — auto-create local tracking branch

User types to filter (fzf-style inline filtering within the TUI list).

### Step 3: Folder Name
Auto-derived from branch name, displayed in an editable field.

**Derivation logic**:
- Strip common prefixes: `feature/`, `fix/`, `bugfix/`, `hotfix/`, `chore/`, `refactor/`, etc.
- **Houston repo**: Extract Jira ticket pattern `OPS-\d+` → folder is `ops-<number>` (lowercase). No ticket → short slug with `ops-` prefix.
- **Other repos**: Short slug from remaining branch name. Lowercase, non-alphanumeric → hyphens, collapse consecutive hyphens, cap at ~30 chars.
- Examples:
  - Houston + `feature/OPS-12345-fix-scheduling` → `ops-12345`
  - Houston + `fix/remove-dead-code` → `ops-remove-dead-code`
  - Other + `feature/add-search-bar` → `add-search-bar`

User can edit the pre-filled value or press Enter to accept.

### Step 4: Claude Launch Options
Pre-filled from config defaults. Tab through fields:
- **Model**: Selectable list — `claude-opus-4-7`, `claude-opus-4-6`, `claude-sonnet-4-6`, `claude-haiku-4-5`
- **Effort**: Selectable list — `low`, `medium`, `high`, `max`, `auto`
- **Launch Claude**: Toggle, default ON. Set OFF to skip.

Enter to confirm all.

### Step 5: Execution
1. Create git worktree at `<base_dir>/<folder-name>` from selected branch
2. Create new tmux window (on current server) named `<folder-name>`
3. Send `cd <worktree-path>` to the new window
4. If launch enabled: send `<alias> --model <model> --effort <effort>` to the pane

---

## Module Layout (new/changed files)

```
src/
├── config.rs              [NEW]  Config loading, first-run prompt, TOML serde
├── tmux.rs                [MOD]  Add server parameter to all commands, auto-discovery
├── session.rs             [MOD]  Add PaneType field
├── detection.rs           [MOD]  Add ocli status detection
├── workflow/              [NEW]
│   ├── mod.rs                    Re-exports
│   └── worktree.rs               WorktreeFlow state machine, folder derivation, execution
├── app/
│   ├── mod.rs             [MOD]  Add config field, WorktreeFlow mode integration
│   └── mode.rs            [MOD]  Add WorktreeFlow mode with sub-states
├── input.rs               [MOD]  Handle WorktreeFlow key events
├── ui/
│   ├── mod.rs             [MOD]  Rename title, server tags
│   └── dialogs.rs         [MOD]  WorktreeFlow dialog rendering
├── completion.rs          [MOD]  Reuse branch completion for fuzzy search
└── main.rs                [MOD]  Config loading, CLI args for --server
```

## Dependencies (new)

- `toml` + `serde` — config file parsing
- `clap` — CLI argument parsing (for `--server` flag)
