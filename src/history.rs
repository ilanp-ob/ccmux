use std::path::PathBuf;
use std::time::SystemTime;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionEntry {
    pub id: String,
    pub title: String,
    pub cwd: String,
    pub branch: Option<String>,
    pub worktree_label: String,
    pub last_activity: SystemTime,
    pub msg_count: usize,
    pub file_path: PathBuf,
    pub worktree_alive: bool,
}

/// Parse one session `.jsonl` into a `SessionEntry`. Pure: no filesystem access.
/// `worktree_alive` is left `true`; the impure scanner sets it.
/// Returns `None` if the file has no user/assistant messages.
pub fn parse_session_meta(jsonl: &str, file_path: PathBuf, mtime: SystemTime) -> Option<SessionEntry> {
    let mut cwd: Option<String> = None;
    let mut branch: Option<String> = None;
    let mut ai_title: Option<String> = None;
    let mut first_user_text: Option<String> = None;
    let mut msg_count = 0usize;

    for line in jsonl.lines() {
        let line = line.trim();
        if line.is_empty() { continue; }
        let v: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if cwd.is_none() {
            if let Some(c) = v.get("cwd").and_then(|x| x.as_str()) { cwd = Some(c.to_string()); }
        }
        if branch.is_none() {
            if let Some(b) = v.get("gitBranch").and_then(|x| x.as_str()) {
                if !b.is_empty() { branch = Some(b.to_string()); }
            }
        }
        match v.get("type").and_then(|t| t.as_str()) {
            Some("ai-title") => {
                if let Some(t) = v.get("aiTitle").and_then(|x| x.as_str()) {
                    ai_title = Some(t.to_string());
                }
            }
            Some("user") => {
                msg_count += 1;
                if first_user_text.is_none() {
                    first_user_text = extract_user_text(&v);
                }
            }
            Some("assistant") => { msg_count += 1; }
            _ => {}
        }
    }

    if msg_count == 0 { return None; }

    let cwd = cwd.unwrap_or_default();
    let worktree_label = std::path::Path::new(&cwd)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| cwd.clone());
    let id = file_path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let title = ai_title
        .or(first_user_text)
        .map(|t| truncate_title(&t, 60))
        .unwrap_or_else(|| "(untitled)".to_string());

    Some(SessionEntry {
        id, title, cwd, branch, worktree_label,
        last_activity: mtime, msg_count, file_path, worktree_alive: true,
    })
}

/// Extract plain text from a `user` record's message content (string or block array).
fn extract_user_text(v: &serde_json::Value) -> Option<String> {
    let content = v.get("message")?.get("content")?;
    if let Some(s) = content.as_str() {
        return Some(s.trim().to_string());
    }
    if let Some(arr) = content.as_array() {
        let mut out = String::new();
        for block in arr {
            if block.get("type").and_then(|t| t.as_str()) == Some("text") {
                if let Some(t) = block.get("text").and_then(|x| x.as_str()) {
                    out.push_str(t);
                }
            }
        }
        let out = out.trim().to_string();
        if !out.is_empty() { return Some(out); }
    }
    None
}

/// Truncate to `max` chars, appending "…" if cut. Single-line (newlines → spaces).
pub fn truncate_title(s: &str, max: usize) -> String {
    let oneline: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let chars: Vec<char> = oneline.chars().collect();
    if chars.len() <= max {
        oneline
    } else {
        let cut: String = chars[..max.saturating_sub(1)].iter().collect();
        format!("{}…", cut)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{"type":"last-prompt","leafUuid":"x","sessionId":"abc"}
{"type":"attachment","cwd":"/Users/me/dev/proj","gitBranch":"main"}
{"type":"ai-title","aiTitle":"Fix the parser bug","sessionId":"abc"}
{"type":"user","message":{"role":"user","content":"hello there"}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"hi"}]}}
{"type":"user","message":{"role":"user","content":"again"}}"#;

    #[test]
    fn parses_title_cwd_branch_and_msg_count() {
        let e = parse_session_meta(SAMPLE, PathBuf::from("/p/abc.jsonl"), SystemTime::UNIX_EPOCH).unwrap();
        assert_eq!(e.id, "abc");
        assert_eq!(e.title, "Fix the parser bug");
        assert_eq!(e.cwd, "/Users/me/dev/proj");
        assert_eq!(e.branch, Some("main".to_string()));
        assert_eq!(e.worktree_label, "proj");
        assert_eq!(e.msg_count, 3); // 2 user + 1 assistant
        assert!(e.worktree_alive);
    }

    #[test]
    fn falls_back_to_first_user_text_when_no_ai_title() {
        let no_title = "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"first prompt here\"}}";
        let e = parse_session_meta(no_title, PathBuf::from("/p/zzz.jsonl"), SystemTime::UNIX_EPOCH).unwrap();
        assert_eq!(e.title, "first prompt here");
    }

    #[test]
    fn returns_none_for_no_messages() {
        let only_meta = "{\"type\":\"attachment\",\"cwd\":\"/x\"}";
        assert!(parse_session_meta(only_meta, PathBuf::from("/p/q.jsonl"), SystemTime::UNIX_EPOCH).is_none());
    }
}
