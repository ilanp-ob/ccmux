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

pub const HOOK_EVENTS: [&str; 5] =
    ["UserPromptSubmit", "Stop", "Notification", "SessionStart", "SessionEnd"];

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

fn entry_is_ours(entry: &serde_json::Value) -> bool {
    entry.get("hooks").and_then(|h| h.as_array()).map(|hs| {
        hs.iter().any(|h| h.get("command").and_then(|c| c.as_str())
            .map(|c| c.contains("hook-event")).unwrap_or(false))
    }).unwrap_or(false)
}

/// Additively merge ccmux's hook entries into a settings.json string.
/// Empty input → `{}`. Returns None only if non-empty input is unparseable.
pub fn merge_hooks_into_settings(existing: &str, bin_path: &str) -> Option<String> {
    let mut root: serde_json::Value = if existing.trim().is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_str(existing).ok()?
    };
    if !root.is_object() { return None; }

    let our_cmd = format!("{} hook-event", bin_path);
    let our_entry = serde_json::json!({
        "matcher": "",
        "hooks": [ { "type": "command", "command": our_cmd } ]
    });

    let hooks = root.as_object_mut().unwrap()
        .entry("hooks").or_insert_with(|| serde_json::json!({}));
    let hooks = hooks.as_object_mut()?;

    for ev in HOOK_EVENTS {
        let arr = hooks.entry(ev).or_insert_with(|| serde_json::json!([]));
        let arr = arr.as_array_mut()?;
        // Idempotent: update an existing ours-entry's command, else append.
        if let Some(existing_ours) = arr.iter_mut().find(|e| entry_is_ours(e)) {
            *existing_ours = our_entry.clone();
        } else {
            arr.push(our_entry.clone());
        }
    }
    serde_json::to_string_pretty(&root).ok()
}

/// Remove ccmux's hook entries; drop now-empty event arrays. Other hooks kept.
pub fn unmerge_hooks_from_settings(existing: &str) -> Option<String> {
    let mut root: serde_json::Value = serde_json::from_str(existing).ok()?;
    if let Some(hooks) = root.get_mut("hooks").and_then(|h| h.as_object_mut()) {
        let keys: Vec<String> = hooks.keys().cloned().collect();
        for k in keys {
            if let Some(arr) = hooks.get_mut(&k).and_then(|a| a.as_array_mut()) {
                arr.retain(|e| !entry_is_ours(e));
                let empty = arr.is_empty();
                if empty { hooks.remove(&k); }
            }
        }
    }
    serde_json::to_string_pretty(&root).ok()
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

    fn cmd_count(json: &str) -> usize {
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        let mut n = 0;
        if let Some(hooks) = v.get("hooks").and_then(|h| h.as_object()) {
            for (_ev, arr) in hooks {
                for entry in arr.as_array().unwrap_or(&vec![]) {
                    for h in entry.get("hooks").and_then(|x| x.as_array()).unwrap_or(&vec![]) {
                        if h.get("command").and_then(|c| c.as_str()).unwrap_or("").contains("hook-event") { n += 1; }
                    }
                }
            }
        }
        n
    }

    #[test]
    fn merge_adds_all_events_into_empty_settings() {
        let out = merge_hooks_into_settings("", "/bin/ccmux").unwrap();
        assert_eq!(cmd_count(&out), 5); // one per event
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert!(v["hooks"]["Stop"].is_array());
    }

    #[test]
    fn merge_preserves_existing_hooks() {
        let existing = r#"{"hooks":{"Stop":[{"matcher":"","hooks":[{"type":"command","command":"my-existing-hook"}]}]},"model":"x"}"#;
        let out = merge_hooks_into_settings(existing, "/bin/ccmux").unwrap();
        assert!(out.contains("my-existing-hook"));      // existing preserved
        assert!(out.contains("/bin/ccmux hook-event")); // ours added
        assert_eq!(serde_json::from_str::<serde_json::Value>(&out).unwrap()["model"], "x");
    }

    #[test]
    fn merge_is_idempotent() {
        let once = merge_hooks_into_settings("", "/bin/ccmux").unwrap();
        let twice = merge_hooks_into_settings(&once, "/bin/ccmux").unwrap();
        assert_eq!(cmd_count(&twice), 5); // not 10
    }

    #[test]
    fn unmerge_removes_only_ours() {
        let existing = r#"{"hooks":{"Stop":[{"matcher":"","hooks":[{"type":"command","command":"my-existing-hook"}]}]}}"#;
        let merged = merge_hooks_into_settings(existing, "/bin/ccmux").unwrap();
        let cleaned = unmerge_hooks_from_settings(&merged).unwrap();
        assert!(cleaned.contains("my-existing-hook"));
        assert_eq!(cmd_count(&cleaned), 0);
    }

    #[test]
    fn unparseable_returns_none() {
        assert!(merge_hooks_into_settings("{not json", "/bin/ccmux").is_none());
    }
}
