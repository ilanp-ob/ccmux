use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Resolve a path's git common dir (shared across all worktrees of a repo).
/// Returns an absolute, canonicalized path, or None if `path` isn't in a repo.
pub fn git_common_dir(path: &Path) -> Option<PathBuf> {
    if !path.is_dir() { return None; }
    let out = std::process::Command::new("git")
        .args(["-C", &path.to_string_lossy(), "rev-parse", "--git-common-dir"])
        .output().ok()?;
    if !out.status.success() { return None; }
    let raw = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if raw.is_empty() { return None; }
    let p = PathBuf::from(&raw);
    // git may return a relative ".git"; resolve against the queried path.
    let abs = if p.is_absolute() { p } else { path.join(p) };
    abs.canonicalize().ok().or(Some(abs))
}

/// The `~/.claude/projects` directory.
fn projects_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "~".to_string());
    PathBuf::from(home).join(".claude").join("projects")
}

/// Gather every session `.jsonl` belonging to `repo_root`'s repo, across all worktrees.
/// `current_cwd` is the selected pane's path, used to sort its sessions first.
pub fn scan_repo_sessions(repo_root: &Path, current_cwd: &str) -> Vec<SessionEntry> {
    let Some(target) = git_common_dir(repo_root) else { return Vec::new() };
    let projects = projects_dir();

    let mut entries: Vec<SessionEntry> = Vec::new();
    let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();

    let Ok(dirs) = std::fs::read_dir(&projects) else { return Vec::new() };
    for dir in dirs.flatten() {
        // Skip symlinked project dirs to avoid double-counting (git-aware-history consolidation).
        match dir.file_type() {
            Ok(ft) if ft.is_symlink() => continue,
            _ => {}
        }
        let dpath = dir.path();
        if !dpath.is_dir() { continue; }
        let Ok(files) = std::fs::read_dir(&dpath) else { continue };
        for f in files.flatten() {
            let fpath = f.path();
            if fpath.extension().and_then(|e| e.to_str()) != Some("jsonl") { continue; }
            let canon = fpath.canonicalize().unwrap_or_else(|_| fpath.clone());
            if !seen.insert(canon) { continue; }
            let Ok(text) = std::fs::read_to_string(&fpath) else { continue };
            let mtime = std::fs::metadata(&fpath)
                .and_then(|m| m.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            if let Some(mut e) = parse_session_meta(&text, fpath.clone(), mtime) {
                e.worktree_alive = Path::new(&e.cwd).is_dir();
                entries.push(e);
            }
        }
    }

    group_by_repo(entries, |cwd| git_common_dir(Path::new(cwd)), &target, repo_root, current_cwd)
}

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

/// Human-readable "time ago". Buckets: just now / Nm / Nh / Nd ago.
pub fn relative_time(then: std::time::SystemTime, now: std::time::SystemTime) -> String {
    let secs = now.duration_since(then).map(|d| d.as_secs()).unwrap_or(0);
    if secs < 60 { "just now".to_string() }
    else if secs < 3600 { format!("{}m ago", secs / 60) }
    else if secs < 86400 { format!("{}h ago", secs / 3600) }
    else { format!("{}d ago", secs / 86400) }
}

/// Filter `entries` down to those belonging to the repo identified by `target_common_dir`,
/// then sort current-worktree-first, newest-first within groups.
pub fn group_by_repo(
    entries: Vec<SessionEntry>,
    resolve_common_dir: impl Fn(&str) -> Option<std::path::PathBuf>,
    target_common_dir: &std::path::Path,
    repo_root: &std::path::Path,
    current_cwd: &str,
) -> Vec<SessionEntry> {
    let repo_root_str = repo_root.to_string_lossy().to_string();
    let under = format!("{}/", repo_root_str);
    let sibling = format!("{}-", repo_root_str);

    let mut kept: Vec<SessionEntry> = entries.into_iter().filter(|e| {
        match resolve_common_dir(&e.cwd) {
            Some(cd) => cd == target_common_dir,
            None => {
                // Dead worktree — match by ccmux's path convention.
                e.cwd == repo_root_str
                    || e.cwd.starts_with(&under)
                    || e.cwd.starts_with(&sibling)
            }
        }
    }).collect();

    kept.sort_by(|a, b| {
        let a_cur = a.cwd == current_cwd;
        let b_cur = b.cwd == current_cwd;
        // current worktree first (true sorts before false), then newest first
        b_cur.cmp(&a_cur).then(b.last_activity.cmp(&a.last_activity))
    });
    kept
}

/// Render a session `.jsonl` into readable transcript text for a pager.
pub fn render_transcript(jsonl: &str, max_turns: usize) -> String {
    let mut turns: Vec<String> = Vec::new();
    for line in jsonl.lines() {
        let line = line.trim();
        if line.is_empty() { continue; }
        let v: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        match v.get("type").and_then(|t| t.as_str()) {
            Some("user") => {
                if let Some(text) = render_user_turn(&v) {
                    turns.push(format!("▶ You\n{}", text));
                }
            }
            Some("assistant") => {
                let body = render_assistant_turn(&v);
                if !body.trim().is_empty() {
                    turns.push(format!("✻ Claude\n{}", body));
                }
            }
            _ => {} // skip bookkeeping types
        }
    }

    let mut prefix = String::new();
    let start = if turns.len() > max_turns {
        prefix = format!("… (earlier turns omitted — showing last {} of {}) …\n\n", max_turns, turns.len());
        turns.len() - max_turns
    } else { 0 };

    format!("{}{}", prefix, turns[start..].join("\n\n"))
}

fn render_user_turn(v: &serde_json::Value) -> Option<String> {
    let content = v.get("message")?.get("content")?;
    if let Some(s) = content.as_str() {
        let s = s.trim();
        return if s.is_empty() { None } else { Some(s.to_string()) };
    }
    if let Some(arr) = content.as_array() {
        let mut out = String::new();
        for block in arr {
            match block.get("type").and_then(|t| t.as_str()) {
                Some("text") => {
                    if let Some(t) = block.get("text").and_then(|x| x.as_str()) {
                        out.push_str(t);
                        out.push('\n');
                    }
                }
                Some("image") => out.push_str("[image]\n"),
                Some("tool_result") => {} // drop
                _ => {}
            }
        }
        let out = out.trim().to_string();
        return if out.is_empty() { None } else { Some(out) };
    }
    None
}

fn render_assistant_turn(v: &serde_json::Value) -> String {
    let mut out = String::new();
    let content = match v.get("message").and_then(|m| m.get("content")).and_then(|c| c.as_array()) {
        Some(a) => a,
        None => return out,
    };
    for block in content {
        match block.get("type").and_then(|t| t.as_str()) {
            Some("text") => {
                if let Some(t) = block.get("text").and_then(|x| x.as_str()) {
                    out.push_str(t);
                    out.push('\n');
                }
            }
            Some("thinking") => {
                let words = block.get("thinking").and_then(|x| x.as_str())
                    .map(|t| t.split_whitespace().count()).unwrap_or(0);
                out.push_str(&format!("  ✻ thinking… ({} words)\n", words));
            }
            Some("tool_use") => {
                let name = block.get("name").and_then(|x| x.as_str()).unwrap_or("tool");
                let arg = tool_key_arg(name, block.get("input"));
                match arg {
                    Some(a) if name == "Bash" => out.push_str(&format!("  ⚙ {}: {}\n", name, a)),
                    Some(a) => out.push_str(&format!("  ⚙ {} {}\n", name, a)),
                    None => out.push_str(&format!("  ⚙ {}\n", name)),
                }
            }
            _ => {}
        }
    }
    out
}

/// Pick a one-line summary arg for a tool call. Bash uses `: cmd`; file tools show the path.
fn tool_key_arg(name: &str, input: Option<&serde_json::Value>) -> Option<String> {
    let input = input?;
    let key = match name {
        "Bash" => "command",
        "Edit" | "Write" | "Read" | "NotebookEdit" => "file_path",
        "Grep" | "Glob" => "pattern",
        _ => return None,
    };
    input.get(key).and_then(|x| x.as_str()).map(|s| truncate_title(s, 60))
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

    fn mk(id: &str, cwd: &str, secs: u64) -> SessionEntry {
        SessionEntry {
            id: id.into(), title: id.into(), cwd: cwd.into(), branch: None,
            worktree_label: "w".into(),
            last_activity: SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(secs),
            msg_count: 1, file_path: PathBuf::from(format!("/p/{}.jsonl", id)), worktree_alive: true,
        }
    }

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

    #[test]
    fn truncate_title_collapses_and_cuts() {
        assert_eq!(truncate_title("short", 60), "short");
        assert_eq!(truncate_title("a\n  b\tc", 60), "a b c");
        assert_eq!(truncate_title("abcdefghij", 5), "abcd…");
    }

    #[test]
    fn relative_time_buckets() {
        use std::time::Duration;
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        assert_eq!(relative_time(now, now), "just now");
        assert_eq!(relative_time(now - Duration::from_secs(120), now), "2m ago");
        assert_eq!(relative_time(now - Duration::from_secs(7200), now), "2h ago");
        assert_eq!(relative_time(now - Duration::from_secs(3 * 86400), now), "3d ago");
    }

    #[test]
    fn groups_includes_repo_and_orders_current_first() {
        let target = PathBuf::from("/repo/.git");
        let repo_root = PathBuf::from("/dev/proj");
        let entries = vec![
            mk("a", "/dev/proj", 100),           // resolves to target
            mk("b", "/dev/proj-feature", 300),   // resolves to target (live worktree)
            mk("c", "/dev/other", 200),          // resolves elsewhere — excluded
            mk("d", "/dev/proj-dead", 400),      // resolver None, path-convention match — included
            mk("e", "/dev/unrelated-dead", 500), // resolver None, no path match — excluded
        ];
        let resolver = |cwd: &str| -> Option<PathBuf> {
            match cwd {
                "/dev/proj" | "/dev/proj-feature" => Some(PathBuf::from("/repo/.git")),
                "/dev/other" => Some(PathBuf::from("/repo2/.git")),
                _ => None, // dead worktrees don't resolve
            }
        };
        let out = group_by_repo(entries, resolver, &target, &repo_root, "/dev/proj");
        let ids: Vec<&str> = out.iter().map(|e| e.id.as_str()).collect();
        // current worktree ("/dev/proj" → a) first; then others by recency desc: d(400), b(300)
        assert_eq!(ids, vec!["a", "d", "b"]);
    }

    #[test]
    fn renders_turns_collapsing_tools_and_thinking() {
        let jsonl = r#"{"type":"attachment","cwd":"/x"}
{"type":"ai-title","aiTitle":"t"}
{"type":"user","message":{"role":"user","content":"do the thing"}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"thinking","thinking":"one two three"},{"type":"text","text":"on it"},{"type":"tool_use","name":"Bash","input":{"command":"cargo build"}},{"type":"tool_use","name":"Edit","input":{"file_path":"src/lib.rs"}}]}}
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"ignored"}]}}"#;
        let out = render_transcript(jsonl, 200);
        assert!(out.contains("▶ You"));
        assert!(out.contains("do the thing"));
        assert!(out.contains("✻ Claude"));
        assert!(out.contains("on it"));
        assert!(out.contains("thinking… (3 words)"));
        assert!(out.contains("⚙ Bash: cargo build"));
        assert!(out.contains("⚙ Edit src/lib.rs"));
        assert!(!out.contains("ignored")); // tool_result dropped
        assert!(!out.contains("ai-title") && !out.contains("\"type\""));
    }

    #[test]
    fn caps_to_last_max_turns() {
        let mut lines = Vec::new();
        for i in 0..10 {
            lines.push(format!("{{\"type\":\"user\",\"message\":{{\"role\":\"user\",\"content\":\"msg{}\"}}}}", i));
        }
        let out = render_transcript(&lines.join("\n"), 3);
        assert!(out.contains("earlier turns omitted"));
        assert!(out.contains("msg9"));
        assert!(!out.contains("msg0"));
    }
}
