mod config;
mod detection;
mod session;
mod tmux;

use anyhow::Result;
use clap::{Parser, Subcommand};
use session::ClaudeCodeStatus;

#[derive(Parser)]
#[command(name = "ccmux", version, about = "tmux sidebar for Claude Code sessions")]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Toggle the sidebar in the current tmux window
    Toggle {
        #[arg(long)]
        server: Option<String>,
    },
    /// Run the sidebar TUI (called internally by toggle)
    Sidebar {
        #[arg(long)]
        server: Option<String>,
    },
    /// Print a status icon for tmux window-status-format
    Status {
        #[arg(long)]
        window: Option<String>,
        #[arg(long)]
        server: Option<String>,
    },
    /// Focus session number N in the sidebar list
    Focus {
        n: usize,
        #[arg(long)]
        server: Option<String>,
    },
    /// Run background notification worker (called internally)
    NotifyWorker {
        #[arg(long)]
        server: Option<String>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Cmd::Status { window, server } => run_status(server.as_deref(), window.as_deref()),
        Cmd::Toggle { server } => run_toggle(server),
        Cmd::Sidebar { server } => run_sidebar(server),
        Cmd::Focus { n, server } => run_focus(n, server),
        Cmd::NotifyWorker { server } => run_notify_worker(server),
    }
}

fn run_status(server: Option<&str>, window: Option<&str>) -> Result<()> {
    let tmux = tmux::Tmux::new(server.map(|s| s.to_string()));

    let window_id = match window {
        Some(id) => id.to_string(),
        None => {
            let pane_id = std::env::var("TMUX_PANE").unwrap_or_default();
            if pane_id.is_empty() {
                print!("?");
                return Ok(());
            }
            let output = std::process::Command::new("tmux")
                .args(["display-message", "-t", &pane_id, "-p", "#{window_id}"])
                .output()?;
            String::from_utf8_lossy(&output.stdout).trim().to_string()
        }
    };

    if window_id.is_empty() {
        print!("?");
        return Ok(());
    }

    // Check for alert first
    if tmux.get_window_var(&window_id, "@ccmux_alert").as_deref() == Some("1") {
        print!("⚠");
        return Ok(());
    }

    // Find Claude pane in this window
    let output = std::process::Command::new("tmux")
        .args(["list-panes", "-t", &window_id, "-F",
               "#{pane_id}\t#{pane_current_command}"])
        .output()?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let claude_pane = stdout.lines().find_map(|line| {
        let mut parts = line.splitn(2, '\t');
        let pane_id = parts.next()?;
        let cmd = parts.next().unwrap_or("");
        if cmd == "claude" || cmd.contains("claude") {
            Some(pane_id.to_string())
        } else {
            None
        }
    });

    let Some(pane_id) = claude_pane else {
        print!(" ");
        return Ok(());
    };

    let content = tmux.capture_pane(&pane_id, 30, true).unwrap_or_default();
    let status = detection::detect_status(&content);
    print!("{}", status.icon());
    Ok(())
}

fn run_toggle(_server: Option<String>) -> Result<()> {
    todo!("implemented in Task 9")
}

fn run_sidebar(_server: Option<String>) -> Result<()> {
    todo!("implemented in Task 14")
}

fn run_focus(_n: usize, _server: Option<String>) -> Result<()> {
    todo!("implemented in Task 15")
}

fn run_notify_worker(_server: Option<String>) -> Result<()> {
    todo!("implemented in Plan 4")
}
