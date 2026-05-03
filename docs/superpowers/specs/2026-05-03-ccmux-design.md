# ccmux — Design Spec

**Date:** 2026-05-03  
**Status:** Approved

---

## Overview

ccmux is a tmux sidebar plugin for managing Claude Code sessions. It runs as a split-pane sidebar inside any tmux window, auto-detects every Claude pane in the tmux session (not just ones it started), and sends macOS notifications + tmux alerts when any Claude agent is waiting for input.

It is distributed as a TPM-compatible git plugin. The `ccmux.tmux` script sets up keybindings on install; the binary is installed separately (`cargo install ccmux` or downloaded from GitHub releases).

---

## Goals

- Replace the old popup-based ccmux with a proper sidebar pane (treemux pattern)
- Detect any Claude pane regardless of how it was started
- Support multiple Claude panes per tmux window, multiple tmux windows, multiple tmux servers
- Notify user when Claude is waiting for input — even when the terminal is not focused
- Carry forward all worktree, PR, and git operations from the previous version
- Clean codebase with clear module boundaries from day one

---

## Architecture

Three independent processes, all sharing state through tmux variables:

### ① ccmux sidebar (main TUI)

A Ratatui TUI binary running inside a tmux split pane. Lifecycle:

1. `ccmux toggle` (called from keybinding) checks `@ccmux_sidebar_<session>_<window_id>` tmux variable
2. If a live sidebar pane exists for this window → `tmux kill-pane` (hide)
3. If not → `tmux split-window -hb -l <width> "ccmux sidebar"` (show), store new pane ID in variable
4. On first sidebar open in a session, spawn `ccmux notify-worker` in background and store its PID in `@ccmux_notify_pid`

The sidebar refreshes every 500ms. It scans all panes across all configured tmux servers, groups them by window, and renders the hybrid list.

### ② ccmux notify-worker (background process)

A minimal background process daemonised by the sidebar on first open. It:

- Polls all tmux panes every 2 seconds via `tmux list-panes -s`
- Tracks previous status per pane (in-memory map, keyed by pane ID)
- On transition to `WaitingInput` for a pane that is not currently focused:
  - Fires a macOS notification via `osascript`
  - Sets `@ccmux_alert_WINDOWID = 1` on the tmux window
- Clears `@ccmux_alert_WINDOWID` when the user focuses any pane in that window
- Self-exits cleanly when the tmux session ends (`tmux has-session` returns non-zero)

No socket or IPC — the worker queries tmux directly. Redundancy with the sidebar's own polling is acceptable; tmux queries are cheap.

### ③ ccmux status (one-shot subcommand)

Fast subcommand for `window-status-format` in tmux status bar:

```
set -g window-status-format "#(ccmux status --window #{window_id})"
```

Prints: `●` Working, `◐` WaitingInput, `○` Idle, `?` Unknown, `⚠` if `@ccmux_alert_WINDOWID` is set.

---

## Tmux State Variables

All runtime state stored in tmux variables — no pid files or temp files:

| Variable | Scope | Value |
|---|---|---|
| `@ccmux_sidebar_<session>_<window_id>` | global | pane ID of sidebar for this window |
| `@ccmux_notify_pid` | global | PID of running notify-worker |
| `@ccmux_alert_WINDOWID` | window | `1` when a pane in this window needs attention |
| `@ccmux_color` | window | tmux colour name (e.g. `colour135`) set at window creation |

---

## Sidebar UI

### Layout

**Hybrid:** compact one-line rows; the selected row expands to show full detail and inline actions.

```
┌ ccmux ─────────────────────────────────────────┐
│                                                 │
│  ▸ window: work                                 │
│    ● ops-15619   feat/fix           %1          │
│  ┌─────────────────────────────────────────── ◐ │
│  │ houston   main                   %2          │
│  │ ~/dev/houston                               │
│  │ ⚠ Waiting for input                         │
│  │ [Enter] focus  [K] kill  [r] rename  [w] wt │
│  └────────────────────────────────────────────  │
│    ○ ccmux      fix                 %3          │
│                                                 │
│  ▸ window: research  [server: work]             │
│    ○ learnings  main                %4          │
│                                                 │
└─────────────────────────────────────────────────┘
```

- Window group headers show server label only when multiple servers are configured
- `%N` numbers on the right enable `ccmux focus N` / `prefix+N` jump shortcuts
- WaitingInput rows get a left border highlight (`◐` colour) even when collapsed

### Navigation

| Key | Action |
|---|---|
| `j` / `↓` | Select next session |
| `k` / `↑` | Select previous session |
| `Enter` | Focus selected pane. If in same window: sidebar stays open, focus moves to the pane. If in a different window: switch to that window (sidebar state there is independent). |
| `l` / `→` | Open action menu for selected session |
| `n` | New window flow |
| `w` | New window from worktree flow |
| `K` | Kill selected window (confirm prompt) |
| `r` | Rename selected window |
| `q` / `Esc` | Close sidebar |
| `?` | Help overlay |
| `1`–`9` | Jump to session N directly (works while sidebar is focused; outside the sidebar, bind `ccmux focus N` to any key you choose) |

### Session Detection

Scans via `tmux list-panes -s -F "#{pane_id} #{pane_current_command} #{window_id} #{window_name} #{pane_active} #{pane_current_path} ..."`.

A pane is shown in the sidebar if its `pane_current_command` matches any entry in `detection.commands` config (default: `["claude", "ocli", "ops-cli"]`). Detection is case-insensitive substring match.

Sessions not started by ccmux are shown identically to ones that were — the source doesn't matter.

### Multi-server

Config lists additional tmux servers to query. The sidebar fans out `tmux list-panes` calls to each server, aggregates results, and groups window headers with a `[server: name]` label when more than one server has results.

---

## Worktree Creation Flow

Five-step modal flow (same as previous ccmux, cleaned up):

1. **Fetching** — git fetch in background
2. **BranchSelect** — filter existing branches or type a new name; choose base branch
3. **FolderName** — derived from branch name, editable
4. **ClaudeOptions** — model, effort, launch Claude toggle, color picker, VS Code toggle
5. **Execute** — creates worktree, opens tmux window, sets `@ccmux_color`, optionally opens VS Code with matching title bar color

The worktree is created as a sibling of the main repo (`source_repo.parent().join(folder)`).

---

## Window Color

Color is chosen during worktree/new-window creation from a fixed palette:

| Name | Hex | tmux colour |
|---|---|---|
| none | — | — |
| red | #E06C75 | colour167 |
| orange | #E5C07B | colour179 |
| green | #98C379 | colour114 |
| blue | #61AFEF | colour75 |
| purple | #C678DD | colour135 |
| pink | #FF79C6 | colour212 |
| cyan | #56B6C2 | colour73 |

- tmux variable `@ccmux_color` set on the window via `set-window-option`
- VS Code: writes `workbench.colorCustomizations.titleBar.activeBackground` into `.vscode/settings.json` (merges, does not overwrite)
- Status bar format (user adds to tmux.conf):

```
set -g window-status-format \
  "#{?#{@ccmux_color},#[fg=#{@ccmux_color}],}#W#[default] #(ccmux status --window #{window_id})"
```

---

## Notifications

On `WaitingInput` transition for a non-focused pane:

**macOS notification** (via `osascript`):
```
display notification "houston is waiting for input" with title "ccmux" subtitle "main • feat/fix"
```

**tmux alert**: `set-window-option -t <window_id> @ccmux_alert 1`  
`ccmux status` reads this and outputs `⚠` instead of the normal icon.  
Alert clears when user focuses any pane in that window.

Notification fires once per state transition. `repeat_secs = 0` (default). Set `> 0` in config to re-fire periodically.

---

## Configuration

`~/.config/ccmux/config.toml` — created with defaults on first run:

```toml
[sidebar]
width = 50
position = "left"       # or "right"
refresh_ms = 500

[claude]
alias = "claude"
default_model = "claude-sonnet-4-6"
default_effort = "high"

[detection]
commands = ["claude", "ocli", "ops-cli"]

[notifications]
enabled = true
macos = true
tmux_bell = true
repeat_secs = 0

[worktree]
base_dir = "~/dev"
houston_path = "~/dev/houston"

[worktree.defaults]
base_branch = "origin/master"

[servers]
# extra = ["other-server"]   # tmux -L other-server
```

TPM options in `tmux.conf` override config for common settings:

```
set -g @ccmux-width 50
set -g @ccmux-toggle-key C-c      # used as prefix+C-c
set -g @ccmux-notifications on
```

---

## TPM Plugin

Repository root contains `ccmux.tmux` which TPM sources on install/reload. It:

1. Reads `@ccmux-toggle-key` option (default `C-c`)
2. Binds `prefix+<key>` to `run-shell "ccmux toggle"`
3. Does NOT bind number keys outside the sidebar (avoids conflicting with tmux window-switch bindings). Users who want external jump shortcuts add them manually: `bind-key 1 run-shell "ccmux focus 1"`
4. Checks for `ccmux` binary in PATH; prints a one-time warning if missing with install instructions

User install:
```tmux
set -g @plugin 'ilanp-ob/ccmux'
```
Then `prefix+I` to install. Binary installed separately:
```bash
cargo install ccmux
# or download from https://github.com/ilanp-ob/ccmux/releases
```

---

## Module Structure

```
src/
  main.rs              CLI entrypoint (clap): toggle, sidebar, notify-worker, status, focus
  config.rs            Config struct, TOML load/save, WINDOW_COLORS, AVAILABLE_MODELS/EFFORTS

  tmux/
    mod.rs             Tmux command builder (-L <server> wrapping)
    detect.rs          list-panes scan → Vec<DetectedPane>, classify_command()
    state.rs           get/set @ccmux_* tmux variables
    windows.rs         new_window, kill_window, rename_window, split_window, set_window_color

  session.rs           Session, Pane, ClaudeCodeStatus, PaneType data types
  detection.rs         detect_status(content) — Working/WaitingInput/Idle/Unknown
  git.rs               GitContext, list_branches, create_worktree, PR operations
  completion.rs        Path tab-completion

  sidebar/
    mod.rs             App state + lifecycle (new, tick, refresh_sessions)
    render.rs          Ratatui frame rendering (read-only access to state)
    input.rs           Key event → state mutation
    mode.rs            Mode enum + all flow states

  notify.rs            notify-worker main loop

  workflow/
    worktree.rs        Worktree creation flow
    new_window.rs      New window flow
    pr.rs              PR creation flow

ccmux.tmux             TPM plugin entry point (shell script)
```

Target: no file exceeds ~300 lines. `sidebar/render.rs` and `sidebar/input.rs` are pure functions over the state defined in `sidebar/mod.rs`.

---

## Feature List (v1)

| Feature | |
|---|---|
| Sidebar panel (split pane, per-window toggle) | ✓ |
| Session-wide display, grouped by window + server | ✓ |
| Hybrid row layout (compact + expand on select) | ✓ |
| Auto-detect any Claude/ocli pane | ✓ |
| Multi-server support | ✓ |
| Worktree creation flow (branch→folder→options) | ✓ |
| New window flow | ✓ |
| macOS notification on WaitingInput | ✓ |
| tmux `@ccmux_alert` + `ccmux status` flash | ✓ |
| Window color picker → tmux var + VS Code title bar | ✓ |
| `prefix+1..9` jump to session by number | ✓ |
| TPM plugin (`ccmux.tmux`) | ✓ |
| Kill window / kill + delete worktree | ✓ |
| Rename window | ✓ |
| PR create / view / merge / close | ✓ |
| Git: stage, commit, push, pull, fetch | ✓ |
| Auto-focus last-active window on sidebar open | ✓ |
| Configurable via `~/.config/ccmux/config.toml` | ✓ |
