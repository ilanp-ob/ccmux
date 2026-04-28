//! Helper utilities for the app module
//!
//! Pure functions for path manipulation and name sanitization.

use std::path::PathBuf;

/// Expand ~ to home directory in a path string
pub fn expand_path(path: &str) -> PathBuf {
    if let Some(stripped) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(stripped);
        }
    } else if path == "~" {
        if let Some(home) = dirs::home_dir() {
            return home;
        }
    }
    PathBuf::from(path)
}

/// Sanitize a branch name for use as a session name
/// e.g., "feature/new-thing" -> "new-thing"
pub fn sanitize_for_session_name(branch: &str) -> String {
    branch
        .rsplit('/')
        .next()
        .unwrap_or(branch)
        .replace(['/', '\\', ' ', ':', '.'], "-")
}

/// Generate default worktree path from repo path and branch name
/// e.g., ~/repos/project + feature/foo -> ~/repos/project-foo
pub fn default_worktree_path(repo_path: &std::path::Path, branch: &str) -> PathBuf {
    let parent = repo_path.parent().unwrap_or(repo_path);
    let repo_name = repo_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("repo");
    let branch_suffix = sanitize_for_session_name(branch);
    parent.join(format!("{}-{}", repo_name, branch_suffix))
}

/// Derive a short folder name from a branch name for worktree creation.
pub fn derive_folder_name(branch: &str, is_houston: bool) -> String {
    let stripped = strip_branch_prefix(branch);

    if is_houston {
        if let Some(ticket) = extract_jira_ticket(branch) {
            return ticket.to_lowercase();
        }
        let slug = slugify(stripped, 25);
        return format!("ops-{}", slug);
    }

    slugify(stripped, 30)
}

fn strip_branch_prefix(branch: &str) -> &str {
    let prefixes = [
        "feature/", "fix/", "bugfix/", "hotfix/",
        "chore/", "refactor/", "docs/", "test/",
        "ci/", "build/", "perf/", "style/",
    ];
    for prefix in &prefixes {
        if let Some(rest) = branch.strip_prefix(prefix) {
            return rest;
        }
    }
    if let Some(rest) = branch.strip_prefix("origin/") {
        return strip_branch_prefix(rest);
    }
    branch
}

fn extract_jira_ticket(branch: &str) -> Option<String> {
    let upper = branch.to_uppercase();
    let mut start = 0;
    while let Some(pos) = upper[start..].find("OPS-") {
        let abs_pos = start + pos;
        let rest = &upper[abs_pos + 4..];
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if !digits.is_empty() {
            return Some(format!("ops-{}", digits));
        }
        start = abs_pos + 4;
    }
    None
}

fn slugify(s: &str, max_len: usize) -> String {
    let slug: String = s
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    let mut result = String::new();
    let mut prev_hyphen = false;
    for c in slug.chars() {
        if c == '-' {
            if !prev_hyphen && !result.is_empty() {
                result.push('-');
            }
            prev_hyphen = true;
        } else {
            result.push(c);
            prev_hyphen = false;
        }
    }
    let trimmed = result.trim_end_matches('-');
    if trimmed.len() > max_len {
        let cut = &trimmed[..max_len];
        cut.rfind('-')
            .map(|i| cut[..i].to_string())
            .unwrap_or_else(|| cut.to_string())
    } else {
        trimmed.to_string()
    }
}

/// Check if a repo path is the houston repo by comparing resolved paths.
pub fn is_houston_repo(repo_path: &std::path::Path, houston_config_path: &str) -> bool {
    let expanded_houston = expand_path(houston_config_path);
    let repo_canon = std::fs::canonicalize(repo_path).unwrap_or_else(|_| repo_path.to_path_buf());
    let houston_canon = std::fs::canonicalize(&expanded_houston).unwrap_or(expanded_houston);
    repo_canon == houston_canon
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_houston_with_jira_ticket() {
        assert_eq!(derive_folder_name("feature/OPS-12345-fix-scheduling", true), "ops-12345");
        assert_eq!(derive_folder_name("OPS-999-quick-fix", true), "ops-999");
        assert_eq!(derive_folder_name("hotfix/OPS-42-urgent", true), "ops-42");
    }

    #[test]
    fn test_houston_without_jira_ticket() {
        assert_eq!(derive_folder_name("fix/remove-dead-code", true), "ops-remove-dead-code");
        assert_eq!(derive_folder_name("feature/add-logging", true), "ops-add-logging");
    }

    #[test]
    fn test_non_houston() {
        assert_eq!(derive_folder_name("feature/add-search-bar", false), "add-search-bar");
        assert_eq!(derive_folder_name("fix/remove-dead-code", false), "remove-dead-code");
        assert_eq!(derive_folder_name("main", false), "main");
    }

    #[test]
    fn test_remote_branch_prefix() {
        assert_eq!(derive_folder_name("origin/feature/OPS-123-foo", true), "ops-123");
        assert_eq!(derive_folder_name("origin/feature/add-bar", false), "add-bar");
    }

    #[test]
    fn test_slugify_truncation() {
        let long_name = "this-is-a-very-long-branch-name-that-exceeds-the-limit";
        let result = derive_folder_name(&format!("feature/{}", long_name), false);
        assert!(result.len() <= 30);
        assert!(!result.ends_with('-'));
    }
}
