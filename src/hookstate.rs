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

use crate::session::ClaudeCodeStatus;
use std::path::PathBuf;

pub const STATE_TTL_DAYS: i64 = 7;
pub const STALE_WORKING_SECS: i64 = 90;

/// `~/.cache/ccmux/sessions/` (created 0700 via the shared cache helper).
pub fn sessions_dir() -> Option<PathBuf> {
    let dir = crate::sidebar::private_cache_dir()?.join("sessions");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

fn status_str(s: HookStatus) -> &'static str {
    match s {
        HookStatus::Working => "working",
        HookStatus::Idle => "idle",
        HookStatus::Waiting => "waiting",
        HookStatus::Ended => "ended",
    }
}

fn status_from_str(s: &str) -> Option<HookStatus> {
    match s {
        "working" => Some(HookStatus::Working),
        "idle" => Some(HookStatus::Idle),
        "waiting" => Some(HookStatus::Waiting),
        "ended" => Some(HookStatus::Ended),
        _ => None,
    }
}

/// Write/overwrite a session's state file. Errors are swallowed (best-effort).
pub fn write_state(s: &SessionState) {
    let Some(dir) = sessions_dir() else { return };
    // session_id is a UUID; still sanitize to be safe against path traversal.
    let safe: String = s.session_id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
    if safe.is_empty() { return; }
    let json = serde_json::json!({
        "session_id": s.session_id,
        "cwd": s.cwd,
        "status": status_str(s.status),
        "updated_at": s.updated_at,
    });
    let _ = std::fs::write(dir.join(format!("{}.json", safe)), json.to_string());
}

/// Load all session states, GC'ing ended/expired ones.
pub fn load_states() -> Vec<SessionState> {
    let Some(dir) = sessions_dir() else { return Vec::new() };
    let now = now_secs();
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(&dir) else { return out };
    for e in rd.flatten() {
        let path = e.path();
        if path.extension().and_then(|x| x.to_str()) != Some("json") { continue; }
        let Ok(txt) = std::fs::read_to_string(&path) else { continue };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&txt) else { continue };
        let session_id = v["session_id"].as_str().unwrap_or("").to_string();
        let cwd = v["cwd"].as_str().unwrap_or("").to_string();
        let updated_at = v["updated_at"].as_i64().unwrap_or(0);
        let status = v["status"].as_str().and_then(status_from_str);
        let Some(status) = status else { continue };
        // GC: ended, or older than TTL.
        if status == HookStatus::Ended || (now - updated_at) > STATE_TTL_DAYS * 86400 {
            let _ = std::fs::remove_file(&path);
            continue;
        }
        if session_id.is_empty() || cwd.is_empty() { continue; }
        out.push(SessionState { session_id, cwd, status, updated_at });
    }
    out
}

pub fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub fn resolve_status(
    pane_cwd: &str,
    states: &[SessionState],
    scraped: ClaudeCodeStatus,
    content_changed: bool,
    now: i64,
) -> ClaudeCodeStatus {
    let fresh = states.iter()
        .filter(|s| s.cwd == pane_cwd && s.status != HookStatus::Ended)
        .max_by_key(|s| s.updated_at);
    let Some(s) = fresh else { return scraped; };

    if s.status == HookStatus::Working
        && (now - s.updated_at) > STALE_WORKING_SECS
        && !content_changed
    {
        return scraped; // missed Stop — defer to the screen scraper
    }

    match s.status {
        HookStatus::Working => ClaudeCodeStatus::Working,
        HookStatus::Idle => ClaudeCodeStatus::Idle,
        HookStatus::Waiting => ClaudeCodeStatus::WaitingInput,
        HookStatus::Ended => scraped, // unreachable (filtered above)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::ClaudeCodeStatus;

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

    fn st(cwd: &str, status: HookStatus, updated_at: i64) -> SessionState {
        SessionState { session_id: format!("{}-{}", cwd, updated_at), cwd: cwd.into(), status, updated_at }
    }

    #[test]
    fn no_state_falls_back_to_scraped() {
        let got = resolve_status("/repo", &[], ClaudeCodeStatus::Working, false, 100);
        assert_eq!(got, ClaudeCodeStatus::Working);
    }

    #[test]
    fn authoritative_state_wins_over_scraped() {
        let states = [st("/repo", HookStatus::Waiting, 100)];
        let got = resolve_status("/repo", &states, ClaudeCodeStatus::Idle, false, 100);
        assert_eq!(got, ClaudeCodeStatus::WaitingInput);
    }

    #[test]
    fn freshest_same_cwd_wins() {
        let states = [st("/repo", HookStatus::Working, 100), st("/repo", HookStatus::Idle, 200)];
        let got = resolve_status("/repo", &states, ClaudeCodeStatus::Unknown, false, 200);
        assert_eq!(got, ClaudeCodeStatus::Idle);
    }

    #[test]
    fn stale_working_unchanged_defers_to_scraped() {
        let states = [st("/repo", HookStatus::Working, 0)];
        // now far past STALE_WORKING_SECS, content unchanged → defer to scraped
        let got = resolve_status("/repo", &states, ClaudeCodeStatus::Idle, false, 1000);
        assert_eq!(got, ClaudeCodeStatus::Idle);
    }

    #[test]
    fn stale_working_but_content_changing_stays_working() {
        let states = [st("/repo", HookStatus::Working, 0)];
        let got = resolve_status("/repo", &states, ClaudeCodeStatus::Idle, true, 1000);
        assert_eq!(got, ClaudeCodeStatus::Working);
    }

    #[test]
    fn ended_state_ignored() {
        let states = [st("/repo", HookStatus::Ended, 200)];
        let got = resolve_status("/repo", &states, ClaudeCodeStatus::Idle, false, 200);
        assert_eq!(got, ClaudeCodeStatus::Idle); // ended skipped → fall back to scraped
    }

}
