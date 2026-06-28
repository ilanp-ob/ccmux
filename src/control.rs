use crate::session::ClaudeCodeStatus;
use crate::tmux::Tmux;

pub struct ResolvedWindow {
    pub window_id: String,
    pub claude_pane_id: String,
    pub cwd: String,
}

/// Resolve a --window selector (@id or window name) to its Claude pane + cwd.
pub fn resolve_window(tmux: &Tmux, session: &str, selector: &str, commands: &[String]) -> Option<ResolvedWindow> {
    let window_id = if selector_is_id(selector) {
        selector.to_string()
    } else {
        let out = tmux.cmd()
            .args(["list-windows", "-t", session, "-F", "#{window_id}\t#{window_name}"])
            .output().ok()?;
        String::from_utf8_lossy(&out.stdout).lines().find_map(|l| {
            let mut p = l.splitn(2, '\t');
            let id = p.next()?;
            let name = p.next().unwrap_or("");
            if name == selector { Some(id.to_string()) } else { None }
        })?
    };
    let out = tmux.cmd()
        .args(["list-panes", "-t", &window_id, "-F",
               "#{pane_id}\t#{pane_current_command}\t#{pane_current_path}"])
        .output().ok()?;
    String::from_utf8_lossy(&out.stdout).lines().find_map(|l| {
        let mut p = l.splitn(3, '\t');
        let pane = p.next()?;
        let cmd = p.next().unwrap_or("");
        let path = p.next().unwrap_or("");
        let is_claude = cmd.contains("claude") || commands.iter().any(|c| cmd.contains(c.as_str()));
        if is_claude {
            Some(ResolvedWindow { window_id: window_id.clone(), claude_pane_id: pane.to_string(), cwd: path.to_string() })
        } else { None }
    })
}

/// List every detected Claude session with authoritative status.
fn resolve_or_exit(tmux: &Tmux, selector: &str) -> ResolvedWindow {
    let commands = crate::config::Config::load().map(|c| c.detection.commands).unwrap_or_default();
    let session = tmux.current_session().ok().flatten().unwrap_or_default();
    match resolve_window(tmux, &session, selector, &commands) {
        Some(r) => r,
        None => {
            eprintln!("ccmux: no Claude session in window '{}'", selector);
            std::process::exit(1);
        }
    }
}

pub fn run_send(server: Option<String>, window: String, text: String) -> anyhow::Result<()> {
    let tmux = Tmux::new(server);
    let r = resolve_or_exit(&tmux, &window);
    tmux.send_keys(&r.claude_pane_id, &text)?;
    Ok(())
}

pub fn run_read(server: Option<String>, window: String, lines: Option<usize>) -> anyhow::Result<()> {
    let tmux = Tmux::new(server);
    let r = resolve_or_exit(&tmux, &window);
    let content = tmux.capture_pane(&r.claude_pane_id, lines.unwrap_or(50), true)?;
    print!("{}", content);
    Ok(())
}

pub fn run_list(server: Option<String>, json: bool) -> anyhow::Result<()> {
    let cfg = crate::config::Config::load()?;
    let tmux = Tmux::new(server.clone());
    let session = tmux.current_session()?.unwrap_or_default();
    let groups = tmux.list_groups(&session, None, &cfg.detection.commands).unwrap_or_default();
    let states = crate::hookstate::load_states();
    let now = crate::hookstate::now_secs();

    let mut infos = Vec::new();
    for g in &groups {
        for pane in &g.panes {
            let cwd = pane.current_path.to_string_lossy().to_string();
            let status = crate::hookstate::resolve_status(&cwd, &states, pane.status.clone(), false, now);
            let session_id = states.iter()
                .filter(|s| s.cwd == cwd && s.status != crate::hookstate::HookStatus::Ended)
                .max_by_key(|s| s.updated_at)
                .map(|s| s.session_id.clone())
                .unwrap_or_default();
            infos.push(SessionInfo {
                window_id: g.window_id.clone(),
                name: g.window_name.clone(),
                cwd,
                status,
                session_id,
            });
        }
    }

    if json {
        println!("{}", to_json(&infos));
    } else {
        for i in &infos {
            println!("{}\t{}\t{}\t{}", i.window_id, i.name, status_label(i.status.clone()), i.cwd);
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitTarget { Idle, Waiting, Settled }

pub fn parse_until(s: &str) -> Option<WaitTarget> {
    match s {
        "idle" => Some(WaitTarget::Idle),
        "waiting" => Some(WaitTarget::Waiting),
        "settled" => Some(WaitTarget::Settled),
        _ => None,
    }
}

pub fn status_matches(s: ClaudeCodeStatus, t: WaitTarget) -> bool {
    use ClaudeCodeStatus as S;
    match t {
        WaitTarget::Idle => s == S::Idle,
        WaitTarget::Waiting => s == S::WaitingInput,
        WaitTarget::Settled => matches!(s, S::Idle | S::WaitingInput),
    }
}

pub fn selector_is_id(s: &str) -> bool { s.starts_with('@') }

/// Quote a string for safe inclusion in a single shell command.
fn sh_quote(s: &str) -> String { format!("'{}'", s.replace('\'', "'\\''")) }

pub fn run_spawn(
    server: Option<String>, dir: String, name: String,
    prompt: Option<String>, model: Option<String>, effort: Option<String>,
) -> anyhow::Result<()> {
    let cfg = crate::config::Config::load()?;
    let model = model.unwrap_or(cfg.claude.default_model);
    let effort = effort.unwrap_or(cfg.claude.default_effort);
    let tmux = Tmux::new(server);
    let session = tmux.current_session()?.unwrap_or_default();
    let path = std::path::PathBuf::from(&dir);
    if !path.is_dir() {
        eprintln!("ccmux: not a directory: {}", dir);
        std::process::exit(1);
    }
    let window_id = tmux.new_window(&session, &name, &path)?;
    // Initial prompt is passed as claude's trailing arg (no typing race).
    let prompt_arg = match &prompt {
        Some(p) if !p.is_empty() => format!(" {}", sh_quote(p)),
        _ => String::new(),
    };
    let launch = format!("claude --model {} --effort {} --name {}{}",
        model, effort, sh_quote(&name), prompt_arg);
    tmux.send_keys(&window_id, &launch)?;
    println!("{}", serde_json::json!({ "window_id": window_id, "name": name }));
    Ok(())
}

pub const WAIT_POLL_MS: u64 = 500;

pub fn run_wait(server: Option<String>, window: String, until: String, timeout: u64) -> anyhow::Result<()> {
    let target = match parse_until(&until) {
        Some(t) => t,
        None => { eprintln!("ccmux: invalid --until '{}' (use idle|waiting|settled)", until); std::process::exit(1); }
    };
    let tmux = Tmux::new(server);
    let r = resolve_or_exit(&tmux, &window);

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout);
    loop {
        let states = crate::hookstate::load_states();
        let now = crate::hookstate::now_secs();
        // hook-state authoritative; scraped fallback = Unknown (wait relies on hooks).
        let status = crate::hookstate::resolve_status(&r.cwd, &states, crate::session::ClaudeCodeStatus::Unknown, false, now);
        if status_matches(status.clone(), target) {
            println!("{}", serde_json::json!({ "window": window, "status": status_label(status) }));
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            println!("{}", serde_json::json!({ "window": window, "status": status_label(status), "timeout": true }));
            std::process::exit(2);
        }
        std::thread::sleep(std::time::Duration::from_millis(WAIT_POLL_MS));
    }
}

pub fn status_label(s: ClaudeCodeStatus) -> &'static str {
    use ClaudeCodeStatus as S;
    match s {
        S::Working => "working",
        S::Thinking => "thinking",
        S::WaitingInput => "waiting",
        S::Idle => "idle",
        S::Unknown => "unknown",
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionInfo {
    pub window_id: String,
    pub name: String,
    pub cwd: String,
    pub status: ClaudeCodeStatus,
    pub session_id: String,
}

pub fn to_json(sessions: &[SessionInfo]) -> String {
    let arr: Vec<serde_json::Value> = sessions.iter().map(|s| serde_json::json!({
        "window_id": s.window_id,
        "name": s.name,
        "cwd": s.cwd,
        "status": status_label(s.status.clone()),
        "session_id": s.session_id,
    })).collect();
    serde_json::to_string(&serde_json::Value::Array(arr)).unwrap_or_else(|_| "[]".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::ClaudeCodeStatus as S;

    #[test]
    fn parse_until_variants() {
        assert_eq!(parse_until("idle"), Some(WaitTarget::Idle));
        assert_eq!(parse_until("waiting"), Some(WaitTarget::Waiting));
        assert_eq!(parse_until("settled"), Some(WaitTarget::Settled));
        assert_eq!(parse_until("bogus"), None);
    }

    #[test]
    fn status_matches_matrix() {
        assert!(status_matches(S::Idle, WaitTarget::Idle));
        assert!(!status_matches(S::WaitingInput, WaitTarget::Idle));
        assert!(status_matches(S::WaitingInput, WaitTarget::Waiting));
        assert!(!status_matches(S::Idle, WaitTarget::Waiting));
        assert!(status_matches(S::Idle, WaitTarget::Settled));
        assert!(status_matches(S::WaitingInput, WaitTarget::Settled));
        assert!(!status_matches(S::Working, WaitTarget::Settled));
        assert!(!status_matches(S::Thinking, WaitTarget::Settled));
        assert!(!status_matches(S::Unknown, WaitTarget::Settled));
    }

    #[test]
    fn selector_is_id_detects_at_prefix() {
        assert!(selector_is_id("@7"));
        assert!(!selector_is_id("fix-bug"));
    }

    #[test]
    fn status_label_strings() {
        assert_eq!(status_label(S::Working), "working");
        assert_eq!(status_label(S::Idle), "idle");
        assert_eq!(status_label(S::WaitingInput), "waiting");
    }

    #[test]
    fn to_json_shape() {
        let s = SessionInfo { window_id: "@7".into(), name: "fix".into(), cwd: "/r".into(), status: S::Idle, session_id: "sid".into() };
        let out = to_json(&[s]);
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v[0]["window_id"], "@7");
        assert_eq!(v[0]["name"], "fix");
        assert_eq!(v[0]["status"], "idle");
        assert_eq!(v[0]["session_id"], "sid");
    }
}
