#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookStatus {
    Working,
    Idle,
    Waiting,
    Ended,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionState {
    pub session_id: String,
    pub cwd: String,
    pub status: HookStatus,
    pub updated_at: i64,
}

/// Parse a Claude Code hook JSON payload into a SessionState. Pure.
/// Returns None for events we don't track or missing required fields.
pub fn parse_event(json: &str, now: i64) -> Option<SessionState> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let session_id = v.get("session_id")?.as_str()?.to_string();
    let cwd = v.get("cwd")?.as_str()?.to_string();
    let event = v.get("hook_event_name")?.as_str()?;
    let status = match event {
        "UserPromptSubmit" => HookStatus::Working,
        "Stop" | "SessionStart" => HookStatus::Idle,
        "SessionEnd" => HookStatus::Ended,
        "Notification" => {
            if v.get("notification_type").and_then(|x| x.as_str()) == Some("idle_prompt") {
                HookStatus::Idle
            } else {
                HookStatus::Waiting
            }
        }
        _ => return None,
    };
    Some(SessionState {
        session_id,
        cwd,
        status,
        updated_at: now,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(event: &str, notif: Option<&str>) -> String {
        let n = notif
            .map(|x| format!(r#","notification_type":"{}""#, x))
            .unwrap_or_default();
        format!(
            r#"{{"session_id":"s1","cwd":"/repo","transcript_path":"/t","hook_event_name":"{}"{}}}"#,
            event, n
        )
    }

    #[test]
    fn maps_each_event_to_status() {
        assert_eq!(
            parse_event(&ev("UserPromptSubmit", None), 100)
                .unwrap()
                .status,
            HookStatus::Working
        );
        assert_eq!(
            parse_event(&ev("Stop", None), 100).unwrap().status,
            HookStatus::Idle
        );
        assert_eq!(
            parse_event(&ev("SessionStart", None), 100).unwrap().status,
            HookStatus::Idle
        );
        assert_eq!(
            parse_event(&ev("SessionEnd", None), 100).unwrap().status,
            HookStatus::Ended
        );
        assert_eq!(
            parse_event(&ev("Notification", Some("idle_prompt")), 100)
                .unwrap()
                .status,
            HookStatus::Idle
        );
        assert_eq!(
            parse_event(&ev("Notification", Some("permission_prompt")), 100)
                .unwrap()
                .status,
            HookStatus::Waiting
        );
        assert_eq!(
            parse_event(&ev("Notification", None), 100).unwrap().status,
            HookStatus::Waiting
        );
    }

    #[test]
    fn fills_fields_and_now() {
        let s = parse_event(&ev("Stop", None), 4242).unwrap();
        assert_eq!(s.session_id, "s1");
        assert_eq!(s.cwd, "/repo");
        assert_eq!(s.updated_at, 4242);
    }

    #[test]
    fn untracked_event_and_missing_fields_return_none() {
        assert!(parse_event(&ev("PreToolUse", None), 1).is_none());
        assert!(parse_event(r#"{"hook_event_name":"Stop"}"#, 1).is_none()); // no session_id/cwd
        assert!(parse_event("not json", 1).is_none());
    }
}
