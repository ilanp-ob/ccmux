#!/usr/bin/env bash
# ccmux TPM plugin entry point
# Installs keybindings and checks for the ccmux binary.

CURRENT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"

# Read user-configured options (with defaults)
toggle_key=$(tmux show-option -gv @ccmux-toggle-key 2>/dev/null || echo "C-c")
close_key=$(tmux show-option -gv @ccmux-close-key 2>/dev/null || echo "M-c")
notifications=$(tmux show-option -gv @ccmux-notifications 2>/dev/null || echo "on")

# Bind toggle key (prefix + key)
tmux bind-key "$toggle_key" run-shell "ccmux toggle"

# Bind close-all key (prefix + M-c by default; override with @ccmux-close-key)
tmux bind-key "$close_key" run-shell "ccmux close"

# Install window-focus-in hook for sticky sidebar auto-open
tmux set-hook -g window-focus-in "run-shell 'ccmux auto-open --window \#{window_id}'"

# Check binary is available
if ! command -v ccmux &>/dev/null; then
    tmux display-message "ccmux: binary not found. Run: cargo install ccmux"
fi
