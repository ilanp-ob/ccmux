#!/usr/bin/env bash
# ccmux TPM plugin entry point
# Installs keybindings and checks for the ccmux binary.

CURRENT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"

# Read user-configured options (with defaults)
toggle_key=$(tmux show-option -gv @ccmux-toggle-key 2>/dev/null || echo "C-c")
close_key=$(tmux show-option -gv @ccmux-close-key 2>/dev/null || echo "M-c")
notifications=$(tmux show-option -gv @ccmux-notifications 2>/dev/null || echo "on")

# Claude window icon — Nerd Fonts robot  (U+F544) by default.
# Override in ~/.tmux.conf:  set -g @ccmux-claude-icon "✳"   (plain-font fallback)
_ccmux_icon=$(tmux show-option -gv @ccmux-claude-icon 2>/dev/null)
if [ -z "$_ccmux_icon" ]; then
    _ccmux_icon=$''   # nf-fa-robot
fi
# Store in a tmux option so the hook can read it without quoting the glyph inline.
tmux set-option -g @_ccmux_claude_icon "$_ccmux_icon"

# Bind toggle key (prefix + key) — -b so the client is never blocked
tmux bind-key "$toggle_key" run-shell -b "ccmux toggle"

# Bind close-all key (prefix + M-c by default; override with @ccmux-close-key)
tmux bind-key "$close_key" run-shell -b "ccmux close"

# after-select-window and after-new-window (set in ~/.tmux.conf or via ccmux setup)
# cover the auto-open use case. window-focus-in fires on every iTerm2 click/focus
# event and would block tmux even with run-shell, so we intentionally omit it here.

# Replace "cc:" prefix with the Claude icon in window names set by Claude Code.
# The hook fires on every window-renamed event; the case guard prevents an infinite loop.
tmux set-hook -g window-renamed \
  'run-shell -b "icon=$(tmux show-option -gv @_ccmux_claude_icon 2>/dev/null); n=#{window_name}; case $n in cc:*) tmux rename-window -t #{window_id} \"${icon:-✳} ${n#cc:}\";; esac"'

# Rename any already-running cc: windows right now (e.g. after plugin reload).
tmux list-windows -a -F "#{window_id}\t#{window_name}" 2>/dev/null | \
  while IFS="$(printf '\t')" read -r wid wname; do
    case "$wname" in
      cc:*) tmux rename-window -t "$wid" "${_ccmux_icon} ${wname#cc:}" ;;
    esac
  done

# Check binary is available
if ! command -v ccmux &>/dev/null; then
    tmux display-message "ccmux: binary not found. Run: cargo install ccmux"
fi
