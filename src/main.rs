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

#[derive(Parser)]
#[command(name = "ccmux", version, about = "TUI for managing Claude Code tmux sessions")]
struct Cli {
    #[arg(long)]
    server: Option<String>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

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
