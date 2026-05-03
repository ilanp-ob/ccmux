# tmux Setup

## Install via TPM

Add to `~/.tmux.conf`:

    set -g @plugin 'ilanp-ob/ccmux'
    set -g @ccmux-toggle-key C-c   # prefix+C-c to toggle sidebar

Then press `prefix+I` to install. Install the binary separately:

    cargo install ccmux

## Status Bar (optional)

    set -g window-status-format \
      "#{?#{@ccmux_color},#[fg=#{@ccmux_color}],}#W#[default] #(ccmux status --window #{window_id})"
    set -g window-status-current-format \
      "#{?#{@ccmux_color},#[fg=#{@ccmux_color},bold],#[bold]}#W#[default] #(ccmux status --window #{window_id})"
