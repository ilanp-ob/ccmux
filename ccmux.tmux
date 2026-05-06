#!/usr/bin/env bash
# ccmux TPM plugin entry point
# Installs keybindings and checks for the ccmux binary.

CURRENT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"

# Read user-configured options (with defaults)
toggle_key=$(tmux show-option -gv @ccmux-toggle-key 2>/dev/null || echo "C-c")
close_key=$(tmux show-option -gv @ccmux-close-key 2>/dev/null || echo "M-c")
notifications=$(tmux show-option -gv @ccmux-notifications 2>/dev/null || echo "on")

# Bind toggle key (prefix + key) — -b so the client is never blocked
tmux bind-key "$toggle_key" run-shell -b "ccmux toggle"

# Bind close-all key (prefix + M-c by default; override with @ccmux-close-key)
tmux bind-key "$close_key" run-shell -b "ccmux close"

# after-select-window and after-new-window (set in ~/.tmux.conf or via ccmux setup)
# cover the auto-open use case. window-focus-in fires on every iTerm2 click/focus
# event and would block tmux even with run-shell, so we intentionally omit it here.

# Replace "cc:" prefix with Claude symbol (✳) in window names set by Claude Code.
# The hook fires on every window-renamed event; the case guard prevents an infinite loop.
tmux set-hook -g window-renamed \
  'run-shell -b "n=#{window_name}; case $n in cc:*) tmux rename-window -t #{window_id} \"✳ ${n#cc:}\";; esac"'

# Rename any already-running cc: windows right now (e.g. after plugin reload).
tmux list-windows -a -F "#{window_id}\t#{window_name}" 2>/dev/null | \
  while IFS="$(printf '\t')" read -r wid wname; do
    case "$wname" in
      cc:*) tmux rename-window -t "$wid" "✳ ${wname#cc:}" ;;
    esac
  done

# Check binary is available
if ! command -v ccmux &>/dev/null; then
    tmux display-message "ccmux: binary not found. Run: cargo install ccmux"
fi
