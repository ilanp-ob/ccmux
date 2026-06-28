use crate::session::ClaudeCodeStatus;

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
