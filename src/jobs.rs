use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobStatus {
    Working,
    Blocked,
    Completed,
    Failed,
    Idle,
}

impl JobStatus {
    pub fn icon(&self) -> &'static str {
        match self {
            Self::Working   => "●",
            Self::Blocked   => "◐",
            Self::Completed => "✓",
            Self::Failed    => "✗",
            Self::Idle      => "○",
        }
    }

    pub fn needs_attention(&self) -> bool {
        matches!(self, Self::Blocked)
    }
}

/// A background Claude Code daemon job read from ~/.claude/jobs/<id>/state.json
#[derive(Debug, Clone)]
pub struct JobEntry {
    /// 8-char short ID (daemonShort)
    pub id: String,
    /// Full UUID session ID for resume
    pub session_id: String,
    /// Human-readable name (from "name" field, else truncated intent)
    pub name: String,
    pub status: JobStatus,
    /// Current detail text (what the agent is doing / last message)
    pub detail: String,
    /// What the agent is blocked on (only populated when status == Blocked)
    pub needs: Option<String>,
    pub cwd: PathBuf,
    /// Unix epoch seconds of last update
    pub updated_at: i64,
    /// Path to timeline.jsonl for appending replies
    pub timeline_path: PathBuf,
    /// Sequential display number continuing after pane numbers
    pub display_num: usize,
}

impl JobEntry {
    pub fn age_secs(&self) -> i64 {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        (now - self.updated_at).max(0)
    }
}

/// Scan ~/.claude/jobs/*/state.json and return active jobs, excluding this process's
/// own job (matched via CLAUDE_JOB_DIR) and completed/failed jobs.
pub fn load_jobs() -> Vec<JobEntry> {
    let home = std::env::var("HOME").unwrap_or_default();
    let jobs_dir = PathBuf::from(&home).join(".claude").join("jobs");

    let own_id = std::env::var("CLAUDE_JOB_DIR").ok()
        .map(PathBuf::from)
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()));

    let mut entries = Vec::new();

    let Ok(dir) = std::fs::read_dir(&jobs_dir) else { return entries };

    for entry in dir.flatten() {
        let path = entry.path();
        if !path.is_dir() { continue; }

        let id = path.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        if own_id.as_deref() == Some(&id) { continue; }

        let state_path = path.join("state.json");
        let timeline_path = path.join("timeline.jsonl");

        let Ok(content) = std::fs::read_to_string(&state_path) else { continue };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) else { continue };

        let status = match v["state"].as_str().unwrap_or("unknown") {
            "working"            => JobStatus::Working,
            "blocked"            => JobStatus::Blocked,
            "completed" | "done" => JobStatus::Completed,
            "failed"             => JobStatus::Failed,
            _                    => JobStatus::Idle,
        };

        // Don't show completed/failed — they'd just accumulate clutter
        if matches!(status, JobStatus::Completed | JobStatus::Failed) { continue; }

        // Skip interactive terminal sessions — they already appear in the tmux pane list
        if !v["firstTerminalAt"].is_null() { continue; }

        let name = v["name"].as_str()
            .or_else(|| v["intent"].as_str())
            .unwrap_or(&id)
            .to_string();
        let name = truncate_str(&name, 40);

        let detail = v["detail"].as_str().unwrap_or("").to_string();
        let needs  = v["needs"].as_str().map(String::from);
        let session_id = v["sessionId"].as_str()
            .or_else(|| v["resumeSessionId"].as_str())
            .unwrap_or(&id)
            .to_string();
        let cwd = v["cwd"].as_str()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(&home));
        let updated_at = v["updatedAt"].as_str()
            .and_then(parse_iso8601_epoch)
            .unwrap_or(0);

        // The daemon only writes "blocked" to state.json when it detects `needs input:`;
        // it never writes "working" while the agent is mid-run. So state.json can be stuck
        // on "blocked" indefinitely. Override: if the last timeline entry is a user reply
        // (text == ""), the agent received the reply and is working.
        let status = if status == JobStatus::Blocked
            && last_timeline_entry_is_user_reply(&timeline_path)
        {
            JobStatus::Working
        } else {
            status
        };

        entries.push(JobEntry {
            id,
            session_id,
            name,
            status,
            detail,
            needs,
            cwd,
            updated_at,
            timeline_path,
            display_num: 0,
        });
    }

    // Blocked jobs float to top; within the same group, most-recently-updated first
    entries.sort_by(|a, b| {
        let a_blocked = a.status == JobStatus::Blocked;
        let b_blocked = b.status == JobStatus::Blocked;
        b_blocked.cmp(&a_blocked).then(b.updated_at.cmp(&a.updated_at))
    });

    entries
}

/// Append a user reply entry to a job's timeline.jsonl so the daemon picks it up.
pub fn reply_to_job(job: &JobEntry, text: &str) -> anyhow::Result<()> {
    use std::io::Write;
    let entry = serde_json::json!({
        "at": utc_now_iso8601(),
        "state": "blocked",
        "detail": text,
        "text": ""
    });
    let line = serde_json::to_string(&entry)? + "\n";
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&job.timeline_path)?;
    f.write_all(line.as_bytes())?;
    Ok(())
}

/// Read the last line of timeline.jsonl and return true if it's a user reply
/// (agent output has non-empty `text`; user replies have `text: ""`).
fn last_timeline_entry_is_user_reply(timeline_path: &std::path::Path) -> bool {
    let Ok(content) = std::fs::read_to_string(timeline_path) else { return false };
    let last = content.lines().rev().find(|l| !l.trim().is_empty());
    let Some(line) = last else { return false };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { return false };
    v["text"].as_str() == Some("")
}

/// Look up the PTY socket path for a session from the daemon roster.
pub fn pty_sock_for_session(session_short: &str) -> Option<String> {
    let home = std::env::var("HOME").unwrap_or_default();
    let roster = std::path::PathBuf::from(&home)
        .join(".claude").join("daemon").join("roster.json");
    let content = std::fs::read_to_string(roster).ok()?;
    let v: serde_json::Value = serde_json::from_str(&content).ok()?;
    v["workers"][session_short]["ptySock"].as_str().map(String::from)
}


fn truncate_str(s: &str, max: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        s.to_string()
    } else {
        chars[..max.saturating_sub(1)].iter().collect::<String>() + "…"
    }
}

fn parse_iso8601_epoch(ts: &str) -> Option<i64> {
    let b = ts.as_bytes();
    if b.len() < 19 { return None; }
    let s = std::str::from_utf8(&b[..19]).ok()?;
    let y: i64  = s[0..4].parse().ok()?;
    let mo: i64 = s[5..7].parse().ok()?;
    let d: i64  = s[8..10].parse().ok()?;
    let h: i64  = s[11..13].parse().ok()?;
    let mi: i64 = s[14..16].parse().ok()?;
    let sec: i64 = s[17..19].parse().ok()?;
    let (y, mo) = if mo <= 2 { (y - 1, mo + 12) } else { (y, mo) };
    let a = y / 100;
    let jdn = (365.25_f64 * (y + 4716) as f64) as i64
            + (30.6001_f64 * (mo + 1) as f64) as i64
            + d + (2 - a + a / 4) - 1524;
    Some((jdn - 2_440_588) * 86400 + h * 3600 + mi * 60 + sec)
}

pub fn utc_now_iso8601() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let z = secs / 86400 + 2440588;
    let h_rem = ((secs % 86400) + 86400) % 86400;
    let a = z + 32044;
    let b = (4 * a + 3) / 146097;
    let c = a - (146097 * b) / 4;
    let d = (4 * c + 3) / 1461;
    let e = c - (1461 * d) / 4;
    let m = (5 * e + 2) / 153;
    let day   = e - (153 * m + 2) / 5 + 1;
    let month = m + 3 - 12 * (m / 10);
    let year  = 100 * b + d - 4800 + m / 10;
    let hour  = h_rem / 3600;
    let min   = (h_rem % 3600) / 60;
    let sec   = h_rem % 60;
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.000Z", year, month, day, hour, min, sec)
}
