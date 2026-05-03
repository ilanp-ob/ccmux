mod config;
mod detection;
mod session;
mod sidebar;
mod tmux;

use std::io::{self, stdout};
use anyhow::Result;
use clap::{Parser, Subcommand};
use crossterm::{
    event::{self, Event},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::prelude::*;
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
            let output = tmux.cmd()
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
    let output = tmux.cmd()
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

fn run_toggle(server: Option<String>) -> Result<()> {
    let tmux = tmux::Tmux::new(server.clone());

    let session = tmux.current_session()?.unwrap_or_default();
    if session.is_empty() {
        anyhow::bail!("Not inside a tmux session");
    }

    let window_id = tmux.own_window_id().unwrap_or_default();
    let var_key = format!("@ccmux_sidebar_{}_{}", session, window_id);

    // Check if a sidebar pane already exists
    if let Some(pane_id) = tmux.get_var(&var_key) {
        // Verify it's still alive (pane might have died)
        let alive = tmux.cmd()
            .args(["list-panes", "-F", "#{pane_id}"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).contains(&pane_id))
            .unwrap_or(false);

        if alive {
            tmux.kill_pane(&pane_id)?;
            tmux.del_var(&var_key)?;
            return Ok(());
        } else {
            tmux.del_var(&var_key)?;
        }
    }

    // Spawn sidebar
    let config = config::Config::load().unwrap_or_default();

    let binary = std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "ccmux".to_string());

    let sidebar_cmd = match &server {
        Some(s) => format!("{} sidebar --server {}", binary, s),
        None => format!("{} sidebar", binary),
    };

    let pane_id = tmux.split_sidebar(&session, config.sidebar.width, &sidebar_cmd)?;
    tmux.set_var(&var_key, &pane_id)?;
    // Explicitly focus the sidebar pane — run-shell doesn't transfer focus automatically
    let _ = tmux.select_pane(&pane_id);

    // Spawn notify-worker if not already running
    let notify_pid_key = "@ccmux_notify_pid";
    let worker_running = tmux.get_var(notify_pid_key)
        .and_then(|pid| pid.parse::<u32>().ok())
        .map(|pid| {
            std::process::Command::new("kill")
                .args(["-0", &pid.to_string()])
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        })
        .unwrap_or(false);

    if !worker_running {
        let notify_cmd = match &server {
            Some(s) => format!("{} notify-worker --server {}", binary, s),
            None => format!("{} notify-worker", binary),
        };
        let child = std::process::Command::new("sh")
            .args(["-c", &format!("{} &", notify_cmd)])
            .spawn()?;
        tmux.set_var(notify_pid_key, &child.id().to_string())?;
    }

    Ok(())
}

fn run_sidebar(server: Option<String>) -> Result<()> {
    let config = config::Config::load()?;
    let mut app = sidebar::App::new(server, config)?;

    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend)?;

    let result = run_sidebar_loop(&mut terminal, &mut app);

    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;

    result
}

fn run_sidebar_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut sidebar::App,
) -> Result<()> {
    let mut needs_redraw = true;

    loop {
        if needs_redraw {
            terminal.draw(|frame| sidebar::render::render(frame, app))?;
            needs_redraw = false;
        }

        if app.should_quit {
            break;
        }

        if event::poll(std::time::Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                sidebar::input::handle_key(app, key);
                needs_redraw = true;
            }
        }

        if app.refresh() {
            needs_redraw = true;
        }

        if app.tick_status() {
            needs_redraw = true;
        }
    }

    Ok(())
}

fn run_focus(n: usize, server: Option<String>) -> Result<()> {
    let config = config::Config::load().unwrap_or_default();
    let tmux = tmux::Tmux::new(server.clone());

    let session = tmux.current_session()?.unwrap_or_default();
    if session.is_empty() {
        anyhow::bail!("Not inside a tmux session");
    }

    let own_window_id = tmux.own_window_id();
    let groups = tmux.list_groups(
        &session,
        own_window_id.as_deref(),
        &config.detection.commands,
    )?;

    let flat: Vec<_> = groups.iter().flat_map(|g| g.panes.iter()).collect();
    let pane = flat.iter().find(|p| p.display_num == n)
        .ok_or_else(|| anyhow::anyhow!("No session with number {}", n))?;

    tmux.select_window(&pane.window_id)?;
    tmux.select_pane(&pane.pane_id)?;
    Ok(())
}

fn run_notify_worker(_server: Option<String>) -> Result<()> {
    todo!("implemented in Plan 4")
}
