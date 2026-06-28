mod config;
mod control;
mod detection;
mod git;
mod gitstatus;
mod history;
mod hookstate;
mod jobs;
mod notify;
mod session;
mod sidebar;
mod tmux;

use std::io::{self, stdout};
use anyhow::Result;
use clap::{Parser, Subcommand};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::prelude::*;

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
    /// Auto-open sidebar when a window with Claude sessions is focused (called from tmux hook)
    AutoOpen {
        #[arg(long)]
        window: Option<String>,
        #[arg(long)]
        server: Option<String>,
    },
    /// Close all ccmux sidebars in this tmux server
    Close {
        #[arg(long)]
        server: Option<String>,
    },
    /// Install tmux hooks and enable sticky mode so sidebars open automatically
    Setup {
        #[arg(long)]
        server: Option<String>,
        /// Remove ccmux's tmux + Claude Code hooks
        #[arg(long)]
        uninstall: bool,
    },
    /// Attach directly to a running daemon agent via its PTY socket
    #[command(name = "pty-attach", hide = true)]
    PtyAttach {
        /// Short session ID (8-char daemonShort from roster.json)
        session: String,
    },
    /// Record a Claude Code lifecycle hook event (called from settings.json hooks)
    #[command(name = "hook-event", hide = true)]
    HookEvent,
    /// List detected Claude sessions (control CLI)
    List {
        #[arg(long)]
        json: bool,
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
        Cmd::AutoOpen { window, server } => run_auto_open(window, server),
        Cmd::Close { server } => run_close(server),
        Cmd::Setup { server, uninstall } => run_setup(server, uninstall),
        Cmd::PtyAttach { session } => run_pty_attach(&session),
        Cmd::HookEvent => run_hook_event(),
        Cmd::List { json, server } => control::run_list(server, json),
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

    // Get the current window from the tmux client — reliable from run-shell, no TMUX_PANE needed.
    let current_window = tmux.cmd()
        .args(["display-message", "-p", "#{window_id}"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();

    if current_window.is_empty() {
        anyhow::bail!("Could not determine current tmux window");
    }

    // Per-window sidebar: each window manages its own sidebar independently.
    // This lets the user open a sidebar wherever they are without it pulling them elsewhere.
    let var_key = format!("@ccmux_sidebar_{}_{}", session, current_window);

    if let Some(pane_id) = tmux.get_var(&var_key) {
        // Session-wide search — avoids false negatives from window-scoped list-panes.
        let alive = tmux.cmd()
            .args(["list-panes", "-s", "-t", &session, "-F", "#{pane_id}"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).lines().any(|l| l.trim() == pane_id))
            .unwrap_or(false);

        if alive {
            // Since var_key is scoped to current_window, the sidebar IS in this window.
            // pane_active=1 means it's the focused pane here → close. Otherwise → focus it.
            let pane_active = tmux.cmd()
                .args(["display-message", "-t", &pane_id, "-p", "#{pane_active}"])
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "1")
                .unwrap_or(false);

            if pane_active {
                tmux.kill_pane(&pane_id)?;
                tmux.del_var(&var_key)?;
            } else {
                let _ = tmux.select_pane(&pane_id);
            }
            return Ok(());
        } else {
            tmux.del_var(&var_key)?;
        }
    }

    // No sidebar in this window — open one here.
    let config = config::Config::load().unwrap_or_default();

    let binary = std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "ccmux".to_string());

    let sidebar_cmd = match &server {
        Some(s) => format!("{} sidebar --server {}", binary, s),
        None => format!("{} sidebar", binary),
    };

    let pane_id = tmux.split_sidebar(&current_window, config.sidebar.width, &sidebar_cmd)?;
    tmux.set_var(&var_key, &pane_id)?;
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
        // Spawn with Stdio::null() so the notify-worker does NOT inherit the pipe
        // that tmux opened for this run-shell subprocess's stdout. Without this,
        // tmux waits for EOF on that pipe forever (notify-worker never closes it),
        // permanently freezing the tmux client in the triggering iTerm2 window.
        let mut cmd = std::process::Command::new(&binary);
        cmd.arg("notify-worker");
        if let Some(s) = &server { cmd.args(["--server", s]); }
        cmd.stdin(std::process::Stdio::null())
           .stdout(std::process::Stdio::null())
           .stderr(std::process::Stdio::null());
        if let Ok(child) = cmd.spawn() {
            let _ = tmux.set_var(notify_pid_key, &child.id().to_string());
        }
    }

    Ok(())
}

fn run_sidebar(server: Option<String>) -> Result<()> {
    let config = config::Config::load()?;
    let mut app = sidebar::App::new(server, config)?;

    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    stdout().execute(EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend)?;

    let result = run_sidebar_loop(&mut terminal, &mut app);

    disable_raw_mode()?;
    stdout().execute(DisableMouseCapture)?;
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
            match event::read()? {
                Event::Key(key) => {
                    sidebar::input::handle_key(app, key);
                    needs_redraw = true;
                }
                Event::Mouse(mouse) => {
                    sidebar::input::handle_mouse(app, mouse);
                    needs_redraw = true;
                }
                Event::Resize(_, _) => {
                    // Re-enforce the configured sidebar width after a terminal resize
                    // (e.g. monitor attach/detach causes tmux to reshape all panes).
                    if let Some(ref pane_id) = app.own_pane_id.clone() {
                        let width = app.config.sidebar.width.to_string();
                        let tmux = tmux::Tmux::new(app.managed_server.clone());
                        let _ = tmux.cmd()
                            .args(["resize-pane", "-t", pane_id, "-x", &width])
                            .status();
                    }
                    needs_redraw = true;
                }
                _ => {}
            }
        }

        if app.refresh() {
            needs_redraw = true;
        }

        if app.tick_status() {
            needs_redraw = true;
        }

        if app.tick_jobs() {
            needs_redraw = true;
        }

        if app.tick_focus() {
            needs_redraw = true;
        }

        if app.tick_worktree() {
            needs_redraw = true;
        }

        if app.tick_folder_pick() {
            needs_redraw = true;
        }

        if app.tick_history() {
            needs_redraw = true;
        }

        if app.tick_gitstatus() {
            needs_redraw = true;
        }

        if app.tick_nav_hint() {
            needs_redraw = true;
        }

        if app.tick_global_info() {
            needs_redraw = true;
        }

        if app.tick_alerts() {
            needs_redraw = true;
        }

        if app.tick_message() {
            needs_redraw = true;
        }

        if app.tick_own_metrics() {
            needs_redraw = true;
        }

        if app.tick_blink() {
            needs_redraw = true;
        }

        if app.tick_thinking() {
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

    let own_pane_id = std::env::var("TMUX_PANE").ok();
    let groups = tmux.list_groups(
        &session,
        own_pane_id.as_deref(),
        &config.detection.commands,
    )?;

    let flat: Vec<_> = groups.iter().flat_map(|g| g.panes.iter()).collect();
    let Some(pane) = flat.iter().find(|p| p.display_num == n) else {
        // No matching session — show a brief status-bar hint and exit cleanly so
        // tmux doesn't display the ugly "'ccmux focus N' returned 1" toast.
        let _ = tmux.cmd()
            .args(["display-message", &format!("ccmux: no session #{}", n)])
            .status();
        return Ok(());
    };

    let window_id = pane.window_id.clone();
    let pane_id = pane.pane_id.clone();
    tmux.select_window(&window_id)?;
    tmux.select_pane(&pane_id)?;

    // Always open (or reuse) a sidebar in the target window when jumping via shortcut,
    // regardless of sticky setting.  Skip if sidebar already alive.
    let var_key = format!("@ccmux_sidebar_{}_{}", session, window_id);
    let sidebar_alive = tmux.get_var(&var_key)
        .map(|pid| tmux.pane_exists(&pid))
        .unwrap_or(false);

    if !sidebar_alive {
        let binary = std::env::current_exe()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| "ccmux".to_string());
        let sidebar_cmd = match &server {
            Some(s) => format!("{} sidebar --server {}", binary, s),
            None    => format!("{} sidebar", binary),
        };
        if let Ok(new_pane) = tmux.split_sidebar(&window_id, config.sidebar.width, &sidebar_cmd) {
            let _ = tmux.set_var(&var_key, &new_pane);
        }
        // Return focus to the Claude pane.
        let _ = tmux.select_pane(&pane_id);
    }

    Ok(())
}

fn run_notify_worker(server: Option<String>) -> Result<()> {
    notify::run(server);
    Ok(())
}

fn run_close(server: Option<String>) -> Result<()> {
    let tmux = tmux::Tmux::new(server);

    // Scan all global tmux options for @ccmux_sidebar_* vars (one per window that has a sidebar).
    let output = tmux.cmd()
        .args(["show-options", "-g"])
        .output()?;

    let text = String::from_utf8_lossy(&output.stdout);
    let entries: Vec<(String, String)> = text.lines()
        .filter_map(|line| {
            let mut parts = line.splitn(2, ' ');
            let key = parts.next()?.trim().to_string();
            let val = parts.next()?.trim().trim_matches('"').to_string();
            if key.starts_with("@ccmux_sidebar_") && !val.is_empty() {
                Some((key, val))
            } else {
                None
            }
        })
        .collect();

    for (key, pane_id) in entries {
        let _ = tmux.kill_pane(&pane_id);
        let _ = tmux.del_var(&key);
    }

    Ok(())
}

fn run_auto_open(window: Option<String>, server: Option<String>) -> Result<()> {
    let tmux = tmux::Tmux::new(server.clone());

    // Only proceed if sticky mode is enabled
    if tmux.get_var("@ccmux_sticky").as_deref() != Some("1") {
        return Ok(());
    }

    let window_id = match window {
        Some(id) => id,
        None => return Ok(()),
    };

    // Derive session from window
    let output = tmux.cmd()
        .args(["display-message", "-t", &window_id, "-p", "#{session_name}"])
        .output()?;
    let session = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if session.is_empty() {
        return Ok(());
    }

    // Skip if a sidebar is already alive in this window.
    let var_key = format!("@ccmux_sidebar_{}_{}", session, window_id);
    if let Some(pane_id) = tmux.get_var(&var_key) {
        if tmux.pane_exists(&pane_id) {
            return Ok(());
        }
        tmux.del_var(&var_key)?;
    }

    // Quick Claude check using pane_current_command — no ps scan needed.
    // The notify-worker handles the versioned-binary edge case via process tree walk.
    let panes_out = tmux.cmd()
        .args(["list-panes", "-t", &window_id, "-F", "#{pane_current_command}"])
        .output()?;
    let has_claude = String::from_utf8_lossy(&panes_out.stdout)
        .lines()
        .any(|cmd| cmd.contains("claude") || cmd.contains("ocli"));
    if !has_claude {
        return Ok(());
    }

    // Open the sidebar in this window without stealing focus.
    let config = config::Config::load().unwrap_or_default();
    let binary = std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "ccmux".to_string());
    let sidebar_cmd = match &server {
        Some(s) => format!("{} sidebar --server {}", binary, s),
        None => format!("{} sidebar", binary),
    };

    let pane_id = tmux.split_sidebar(&window_id, config.sidebar.width, &sidebar_cmd)?;
    if !pane_id.is_empty() {
        tmux.set_var(&var_key, &pane_id)?;
    }

    Ok(())
}

/// Additively install (or remove) ccmux's Claude Code hooks in ~/.claude/settings.json.
/// Backs up first; aborts (no write) if existing settings JSON is unparseable.
fn install_claude_hooks(uninstall: bool) -> Result<()> {
    let home = std::env::var("HOME").unwrap_or_default();
    let path = std::path::Path::new(&home).join(".claude").join("settings.json");
    let existing = std::fs::read_to_string(&path).unwrap_or_default();

    let binary = std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "ccmux".to_string());

    let updated = if uninstall {
        if existing.trim().is_empty() { return Ok(()); }
        hookstate::unmerge_hooks_from_settings(&existing)
    } else {
        hookstate::merge_hooks_into_settings(&existing, &binary)
    };

    let Some(updated) = updated else {
        anyhow::bail!("~/.claude/settings.json is not valid JSON — not modifying it. Fix it and re-run.");
    };

    if let Some(parent) = path.parent() { let _ = std::fs::create_dir_all(parent); }
    if !existing.trim().is_empty() {
        let _ = std::fs::write(path.with_extension("json.ccmux-bak"), &existing);
    }
    std::fs::write(&path, updated)?;
    println!("✓ {} Claude Code hooks in {}", if uninstall { "Removed" } else { "Installed" }, path.display());
    Ok(())
}

fn run_setup(server: Option<String>, uninstall: bool) -> Result<()> {
    install_claude_hooks(uninstall)?;

    if uninstall {
        let tmux = tmux::Tmux::new(server.clone());
        for hook in ["after-select-window", "after-new-window"] {
            let _ = tmux.cmd().args(["set-hook", "-gu", hook]).status();
        }
        let _ = tmux.set_var("@ccmux_sticky", "0");
        println!("✓ Removed tmux hooks + sticky");
        return Ok(());
    }

    let tmux = tmux::Tmux::new(server.clone());

    let binary = std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "ccmux".to_string());

    let server_flag = match &server {
        Some(s) => format!(" --server {}", s),
        None => String::new(),
    };

    // The auto-open command called by each hook.
    // #{window_id} is a tmux format string expanded by tmux at hook-fire time.
    let auto_open_cmd = format!(
        "{} auto-open --window #{{window_id}}{}",
        binary, server_flag
    );

    // Hooks that cover all cases: switching windows, creating new windows.
    let hooks = ["after-select-window", "after-new-window"];

    for hook in hooks {
        tmux.cmd()
            .args(["set-hook", "-g", hook,
                   &format!("run-shell -b '{}'", auto_open_cmd)])
            .status()?;
        println!("✓ Installed hook: {}", hook);
    }

    // Enable sticky mode so auto-open actually runs its logic.
    tmux.set_var("@ccmux_sticky", "1")?;
    println!("✓ Sticky mode enabled");

    // Bind PREFIX+Ctrl+1..9 (and Ctrl+0 for session 10) to focus by number.
    // Pass -b and the shell command as separate args — no shell quoting needed.
    for n in 1..=9usize {
        let focus_cmd = format!("{} focus {}{}", binary, n, server_flag);
        tmux.cmd()
            .args(["bind-key", &format!("C-{}", n), "run-shell", "-b", &focus_cmd])
            .status()?;
    }
    // Ctrl+0 → session 10
    let focus10_cmd = format!("{} focus 10{}", binary, server_flag);
    tmux.cmd()
        .args(["bind-key", "C-0", "run-shell", "-b", &focus10_cmd])
        .status()?;
    println!("✓ Installed key bindings: PREFIX+Ctrl+1..9 (and Ctrl+0 for session 10)");

    println!();
    println!("Sidebars will now open automatically whenever Claude is running in a window.");
    println!("Jump to any Claude session with PREFIX+Ctrl+NUMBER from anywhere in tmux.");
    println!();
    println!("To persist across tmux restarts, add to ~/.tmux.conf:");
    println!();
    for hook in hooks {
        println!("  set-hook -g {} \"run-shell -b '{}'\"", hook, auto_open_cmd);
    }
    for n in 1..=9usize {
        let focus_cmd = format!("{} focus {}{}", binary, n, server_flag);
        println!("  bind-key C-{} run-shell -b '{}'", n, focus_cmd);
    }
    println!("  bind-key C-0 run-shell -b '{}'", focus10_cmd);
    println!();
    println!("To disable:  ccmux close  (closes all sidebars)");
    println!("             tmux set-option -g @ccmux_sticky 0");

    Ok(())
}

/// Read a Claude Code hook JSON payload from stdin and record the session's status.
/// Best-effort and fast: never fails a Claude turn (always returns Ok).
fn run_hook_event() -> Result<()> {
    use std::io::Read;
    let mut buf = String::new();
    if std::io::stdin().read_to_string(&mut buf).is_ok() {
        if let Some(state) = hookstate::parse_event(&buf, hookstate::now_secs()) {
            hookstate::write_state(&state);
        }
    }
    Ok(())
}

fn run_pty_attach(session_short: &str) -> Result<()> {
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;
    use crossterm::terminal::{enable_raw_mode, disable_raw_mode};

    let sock_path = jobs::pty_sock_for_session(session_short)
        .filter(|p| std::path::Path::new(p).exists())
        .ok_or_else(|| anyhow::anyhow!(
            "No live PTY socket for session {}. Use `claude agents` to attach.",
            session_short
        ))?;

    let mut sock = UnixStream::connect(&sock_path)?;
    let mut sock_write = sock.try_clone()?;

    enable_raw_mode()?;

    // Thread: socket → stdout
    let reader = std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        let stdout = std::io::stdout();
        let mut out = stdout.lock();
        loop {
            match sock.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => { let _ = out.write_all(&buf[..n]); let _ = out.flush(); }
            }
        }
    });

    // Main thread: stdin → socket (until reader thread exits)
    let stdin = std::io::stdin();
    let mut buf = [0u8; 256];
    loop {
        if reader.is_finished() { break; }
        use std::io::BufRead;
        // Non-blocking: just try to read with a short timeout via select on raw fd
        match stdin.lock().read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => { if sock_write.write_all(&buf[..n]).is_err() { break; } }
        }
    }

    disable_raw_mode()?;
    println!("\r\n[detached from {}]", session_short);
    Ok(())
}
