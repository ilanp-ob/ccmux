#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GitStatus {
    pub branch: String,
    pub ahead: u32,
    pub behind: u32,
    pub staged: u32,
    pub unstaged: u32,
    pub untracked: u32,
    pub files: Vec<FileEntry>,
    pub has_upstream: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    pub code: String,
    pub path: String,
}

/// Parse `git status --porcelain=v2 --branch -z` output. Pure.
pub fn parse_status_v2(out: &str) -> GitStatus {
    let mut s = GitStatus::default();
    let mut oid: Option<String> = None;

    // NUL-separated tokens; we need index control to skip rename origPath tokens.
    let tokens: Vec<&str> = out.split('\0').filter(|t| !t.is_empty()).collect();
    let mut i = 0;
    while i < tokens.len() {
        let t = tokens[i];
        if let Some(rest) = t.strip_prefix("# ") {
            if let Some(v) = rest.strip_prefix("branch.oid ") {
                oid = Some(v.to_string());
            } else if let Some(v) = rest.strip_prefix("branch.head ") {
                s.branch = v.to_string();
            } else if rest.starts_with("branch.upstream ") {
                s.has_upstream = true;
            } else if let Some(v) = rest.strip_prefix("branch.ab ") {
                s.has_upstream = true;
                for part in v.split_whitespace() {
                    if let Some(a) = part.strip_prefix('+') { s.ahead = a.parse().unwrap_or(0); }
                    if let Some(b) = part.strip_prefix('-') { s.behind = b.parse().unwrap_or(0); }
                }
            }
            i += 1;
            continue;
        }

        // Entry records.
        if let Some(rest) = t.strip_prefix("? ") {
            s.untracked += 1;
            s.files.push(FileEntry { code: "??".into(), path: rest.to_string() });
        } else if t.starts_with("1 ") || t.starts_with("u ") {
            // `1 <xy> <sub> <mH> <mI> <mW> <hH> <hI> <path>` (8 meta fields, then path)
            let xy = xy_of(t);
            tally(&mut s, &xy);
            let path = field_after(t, 8);
            s.files.push(FileEntry { code: xy, path });
        } else if t.starts_with("2 ") {
            // `2 <xy> <sub> <mH> <mI> <mW> <hH> <hI> <Xscore> <path>` (9 meta fields, then path)
            let xy = xy_of(t);
            tally(&mut s, &xy);
            let path = field_after(t, 9);
            s.files.push(FileEntry { code: xy, path });
            i += 1; // skip the next token: the rename/copy origPath
        }
        i += 1;
    }

    if s.branch == "(detached)" {
        if let Some(o) = oid {
            let short: String = o.chars().take(7).collect();
            s.branch = format!("({})", short);
        }
    }
    s
}

/// The two-char XY status field of an entry token (`<type> <xy> ...`).
fn xy_of(t: &str) -> String {
    t.split(' ').nth(1).unwrap_or("..").to_string()
}

/// The path field: everything after the first `n` space-separated fields.
fn field_after(t: &str, n: usize) -> String {
    t.splitn(n + 1, ' ').nth(n).unwrap_or("").to_string()
}

/// Tally a changed entry's XY into staged/unstaged. `.` means unmodified in that column.
fn tally(s: &mut GitStatus, xy: &str) {
    let mut c = xy.chars();
    let x = c.next().unwrap_or('.');
    let y = c.next().unwrap_or('.');
    if x != '.' { s.staged += 1; }
    if y != '.' { s.unstaged += 1; }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegKind { Branch, Ahead, Behind, Staged, Unstaged, Untracked, Clean }

/// Build the summary line as typed segments, omitting zero counts.
/// A clean repo (no changes) yields Branch + Clean("✓").
pub fn summary_segments(s: &GitStatus) -> Vec<(SegKind, String)> {
    let mut v = vec![(SegKind::Branch, s.branch.clone())];
    if s.has_upstream {
        if s.ahead > 0 { v.push((SegKind::Ahead, format!("↑{}", s.ahead))); }
        if s.behind > 0 { v.push((SegKind::Behind, format!("↓{}", s.behind))); }
    }
    if s.staged > 0 { v.push((SegKind::Staged, format!("●{}", s.staged))); }
    if s.unstaged > 0 { v.push((SegKind::Unstaged, format!("+{}", s.unstaged))); }
    if s.untracked > 0 { v.push((SegKind::Untracked, format!("?{}", s.untracked))); }
    if s.staged == 0 && s.unstaged == 0 && s.untracked == 0 {
        v.push((SegKind::Clean, "✓".to_string()));
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_branch_ahead_behind_and_counts() {
        // staged modified (M.), unstaged modified (.M), untracked (?)
        let out = "# branch.oid abc123\0# branch.head main\0# branch.upstream origin/main\0# branch.ab +2 -1\0\
1 M. N... 100644 100644 100644 aaa bbb src/staged.rs\0\
1 .M N... 100644 100644 100644 ccc ddd src/unstaged.rs\0\
? notes.md\0";
        let s = parse_status_v2(out);
        assert_eq!(s.branch, "main");
        assert!(s.has_upstream);
        assert_eq!((s.ahead, s.behind), (2, 1));
        assert_eq!((s.staged, s.unstaged, s.untracked), (1, 1, 1));
        assert_eq!(s.files.len(), 3);
        assert_eq!(s.files[0], FileEntry { code: "M.".into(), path: "src/staged.rs".into() });
        assert_eq!(s.files[2], FileEntry { code: "??".into(), path: "notes.md".into() });
    }

    #[test]
    fn no_upstream_omits_ahead_behind() {
        let out = "# branch.oid abc\0# branch.head feature\0? x.txt\0";
        let s = parse_status_v2(out);
        assert_eq!(s.branch, "feature");
        assert!(!s.has_upstream);
        assert_eq!((s.ahead, s.behind), (0, 0));
        assert_eq!(s.untracked, 1);
    }

    #[test]
    fn detached_head_uses_short_oid() {
        let out = "# branch.oid 4bdb0a2a48aa1d4e863171294a898237445f5742\0# branch.head (detached)\0";
        let s = parse_status_v2(out);
        assert_eq!(s.branch, "(4bdb0a2)"); // 7-char short oid in parens
        assert_eq!(s.files.len(), 0);
    }

    #[test]
    fn rename_entry_skips_next_origpath_token() {
        // type-2 rename: path is in the `2` record; origPath is the NEXT NUL token and must be skipped.
        let out = "# branch.oid a\0# branch.head main\0\
2 R. N... 100644 100644 100644 aaa bbb R100 new/name.rs\0old/name.rs\0\
? after.txt\0";
        let s = parse_status_v2(out);
        // the origPath token must NOT become a phantom file
        assert_eq!(s.files.len(), 2);
        assert_eq!(s.files[0], FileEntry { code: "R.".into(), path: "new/name.rs".into() });
        assert_eq!(s.files[1].path, "after.txt");
        assert_eq!(s.staged, 1); // R. → index column set
    }

    #[test]
    fn clean_repo_is_empty() {
        let out = "# branch.oid a\0# branch.head main\0# branch.upstream origin/main\0# branch.ab +0 -0\0";
        let s = parse_status_v2(out);
        assert_eq!((s.staged, s.unstaged, s.untracked), (0, 0, 0));
        assert!(s.files.is_empty());
        assert!(s.has_upstream);
    }

    #[test]
    fn summary_omits_zero_segments() {
        let s = GitStatus {
            branch: "main".into(), ahead: 2, behind: 0,
            staged: 3, unstaged: 5, untracked: 2, files: vec![], has_upstream: true,
        };
        let segs = summary_segments(&s);
        assert_eq!(segs, vec![
            (SegKind::Branch, "main".to_string()),
            (SegKind::Ahead, "↑2".to_string()),
            (SegKind::Staged, "●3".to_string()),
            (SegKind::Unstaged, "+5".to_string()),
            (SegKind::Untracked, "?2".to_string()),
        ]); // behind=0 omitted
    }

    #[test]
    fn summary_clean_repo_shows_check() {
        let s = GitStatus { branch: "main".into(), has_upstream: true, ..Default::default() };
        let segs = summary_segments(&s);
        assert_eq!(segs, vec![
            (SegKind::Branch, "main".to_string()),
            (SegKind::Clean, "✓".to_string()),
        ]);
    }
}
