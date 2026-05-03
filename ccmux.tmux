#!/usr/bin/env bash
# ccmux TPM plugin entry point
# Installs keybindings and checks for the ccmux binary.

CURRENT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"

# Read user-configured options (with defaults)
toggle_key=$(tmux show-option -gv @ccmux-toggle-key 2>/dev/null || echo "C-c")
notifications=$(tmux show-option -gv @ccmux-notifications 2>/dev/null || echo "on")

# Bind toggle key (prefix + key)
tmux bind-key "$toggle_key" run-shell "ccmux toggle"

# Check binary is available
if ! command -v ccmux &>/dev/null; then
    tmux display-message "ccmux: binary not found. Run: cargo install ccmux"
fi
