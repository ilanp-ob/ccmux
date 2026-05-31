//! Host terminal app (iTerm2, Terminal, …) memory sampling for the ccmux footer.
//!
//! All process-spawning lives in the `sample_*` / `detect_*` wrappers at the
//! bottom; the logic above them is pure and unit-tested.

/// The terminal application hosting this ccmux session.
#[derive(Debug, Clone, PartialEq)]
pub struct HostApp {
    pub name: String,
    pub pid: u32,
}

/// Extract a display name from an executable path that lives inside a macOS
/// `.app` bundle. Returns `None` for non-app executables (shells, `login`, …).
pub fn friendly_name(exec_path: &str) -> Option<String> {
    if !exec_path.contains(".app/Contents/MacOS/") {
        return None;
    }
    let bundle = exec_path
        .split('/')
        .find(|seg| seg.ends_with(".app"))?
        .strip_suffix(".app")?;
    let friendly = match bundle {
        "iTerm" => "iTerm2",
        other => other,
    };
    Some(friendly.to_string())
}

/// Climb the parent-process chain from `start_pid`, returning the first ancestor
/// that is a macOS `.app` executable. `lookup(pid)` returns `(ppid, comm)` where
/// `comm` is the executable path. Capped at 12 hops as a safety bound.
pub fn walk_to_app<F>(start_pid: u32, lookup: F) -> Option<HostApp>
where
    F: Fn(u32) -> Option<(u32, String)>,
{
    let mut pid = start_pid;
    for _ in 0..12 {
        let (ppid, comm) = lookup(pid)?;
        if let Some(name) = friendly_name(&comm) {
            return Some(HostApp { name, pid });
        }
        if ppid <= 1 {
            break;
        }
        pid = ppid;
    }
    None
}

/// Sum RSS (returned as MB) over the subtree rooted at `root`.
/// `table` rows are `(pid, ppid, rss_kb)`. The root's own RSS is included.
pub fn subtree_rss_mb(root: u32, table: &[(u32, u32, u64)]) -> f32 {
    let mut total_kb = 0u64;
    let mut stack = vec![root];
    let mut seen = std::collections::HashSet::new();
    while let Some(cur) = stack.pop() {
        if !seen.insert(cur) {
            continue;
        }
        for &(pid, ppid, rss) in table {
            if pid == cur {
                total_kb += rss;
            }
            if ppid == cur && pid != cur {
                stack.push(pid);
            }
        }
    }
    total_kb as f32 / 1024.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn friendly_name_iterm() {
        assert_eq!(friendly_name("/Applications/iTerm.app/Contents/MacOS/iTerm2").as_deref(), Some("iTerm2"));
    }
    #[test]
    fn friendly_name_terminal() {
        assert_eq!(friendly_name("/System/Applications/Utilities/Terminal.app/Contents/MacOS/Terminal").as_deref(), Some("Terminal"));
    }
    #[test]
    fn friendly_name_ghostty_keeps_bundle_name() {
        assert_eq!(friendly_name("/Applications/Ghostty.app/Contents/MacOS/ghostty").as_deref(), Some("Ghostty"));
    }
    #[test]
    fn friendly_name_non_app_is_none() {
        assert_eq!(friendly_name("/bin/zsh"), None);
        assert_eq!(friendly_name("login"), None);
    }

    #[test]
    fn walk_reaches_iterm_through_itermserver() {
        let lookup = |pid: u32| -> Option<(u32, String)> {
            match pid {
                5057 => Some((4855, "tmux".into())),
                4855 => Some((4854, "-zsh".into())),
                4854 => Some((88138, "login".into())),
                88138 => Some((88124, "/Users/x/Library/Application Support/iTerm2/iTermServer-3.6.10".into())),
                88124 => Some((1, "/Applications/iTerm.app/Contents/MacOS/iTerm2".into())),
                _ => None,
            }
        };
        assert_eq!(walk_to_app(5057, lookup), Some(HostApp { name: "iTerm2".into(), pid: 88124 }));
    }
    #[test]
    fn walk_returns_none_when_no_app_ancestor() {
        let lookup = |pid: u32| -> Option<(u32, String)> {
            match pid { 10 => Some((1, "/usr/bin/sshd".into())), _ => None }
        };
        assert_eq!(walk_to_app(10, lookup), None);
    }

    #[test]
    fn subtree_sums_descendants_excludes_daemon() {
        let table = [
            (100u32, 1u32, 1024u64),
            (101, 100, 2048),
            (102, 101, 512),
            (200, 1, 9_999_999),
        ];
        assert_eq!(subtree_rss_mb(100, &table), 3.5);
    }
    #[test]
    fn subtree_unknown_root_is_zero() {
        let table = [(1u32, 0u32, 4096u64)];
        assert_eq!(subtree_rss_mb(999, &table), 0.0);
    }
}
