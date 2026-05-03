mod config;
mod detection;
mod session;
mod sidebar;
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

fn run_sidebar(_server: Option<String>) -> Result<()> {
    todo!("implemented in Task 14")
}

fn run_focus(_n: usize, _server: Option<String>) -> Result<()> {
    todo!("implemented in Task 15")
}

fn run_notify_worker(_server: Option<String>) -> Result<()> {
    todo!("implemented in Plan 4")
}
