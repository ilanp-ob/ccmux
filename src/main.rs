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
mod workflow;

use std::io::{self, stdout};

use anyhow::Result;
use clap::Parser;
use crossterm::{
    event::{self, Event},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::prelude::*;

use crate::app::App;
use crate::session::ClaudeCodeStatus;

#[derive(Parser)]
#[command(name = "ccmux", version, about = "TUI for managing Claude Code tmux sessions")]
struct Cli {
    #[arg(long)]
    server: Option<String>,

    #[command(subcommand)]
    command: Option<SubCommand>,
}

#[derive(clap::Subcommand)]
enum SubCommand {
    /// Print a status icon for a tmux window's Claude pane (for status-right)
    Status {
        /// Target window ID (e.g. @3). If omitted, uses the current window.
        #[arg(long)]
        window: Option<String>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    if let Some(SubCommand::Status { window }) = cli.command {
        return run_status(cli.server.as_deref(), window.as_deref());
    }

    // Set up terminal
    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend)?;

    // Run the app
    let result = run(&mut terminal, cli.server);

    // Restore terminal
    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;

    result
}

/// Print a single icon representing the Claude state in the target window.
/// Designed to be called from tmux `status-right` as `#(ccmux status --window #{window_id})`.
fn run_status(server: Option<&str>, window: Option<&str>) -> Result<()> {
    use crate::tmux::Tmux;
    use crate::detection::detect_status;

    // Resolve window ID: use provided, or fall back to the current window
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

    // Find the Claude pane in this window via list-panes targeting by window_id
    let output = std::process::Command::new("tmux")
        .args(["list-panes", "-t", &window_id, "-F",
               "#{pane_id}\t#{pane_current_command}"])
        .output()?;

    if !output.status.success() {
        print!("?");
        return Ok(());
    }

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

    let content = Tmux::capture_pane(server, &pane_id, 30, true).unwrap_or_default();
    let status = detect_status(&content);

    let icon = match status {
        ClaudeCodeStatus::Working => "●",
        ClaudeCodeStatus::WaitingInput => "◐",
        ClaudeCodeStatus::Idle => "○",
        ClaudeCodeStatus::Unknown => "?",
    };

    print!("{}", icon);
    Ok(())
}

fn run(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, server_filter: Option<String>) -> Result<()> {
    let config = config::Config::load()?;
    if !config::Config::exists() {
        config.save()?;
    }
    let mut app = App::new(server_filter, config)?;

    // Always draw on the first iteration
    let mut needs_redraw = true;

    loop {
        if needs_redraw {
            terminal.draw(|frame| ui::render(frame, &mut app))?;
            needs_redraw = false;
        }

        // Check if we should quit
        if app.should_quit {
            break;
        }

        // Handle events (100ms poll keeps the status tick responsive)
        if event::poll(std::time::Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                input::handle_key(&mut app, key);
                needs_redraw = true;
            }
        }

        // Refresh Claude status (self-throttled to 500ms); redraw if status changed
        if app.tick_status() {
            needs_redraw = true;
        }
    }

    Ok(())
}
