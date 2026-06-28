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
