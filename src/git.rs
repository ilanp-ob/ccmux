use std::collections::HashMap;
use std::path::{Path, PathBuf};
use anyhow::{Context, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchEntry {
    pub name: String,
    pub is_remote: bool,
    /// Path to existing worktree for this branch, if any.
    pub worktree_path: Option<String>,
}

/// Walk up from `path` looking for a .git entry (file or directory).
pub fn find_repo_root(path: &Path) -> Option<PathBuf> {
    let mut cur = path.to_path_buf();
    loop {
        if cur.join(".git").exists() {
            return Some(cur);
        }
        if !cur.pop() {
            return None;
        }
    }
}

/// Return the main (non-worktree) repo root.
/// If `path` is inside a worktree (.git is a file), reads the gitdir and resolves up.
pub fn find_main_repo_root(path: &Path) -> Option<PathBuf> {
    let repo_root = find_repo_root(path)?;
    let git_path = repo_root.join(".git");
    if git_path.is_file() {
        // .git file content: "gitdir: /path/to/.git/worktrees/<name>"
        let content = std::fs::read_to_string(&git_path).ok()?;
        let gitdir = content.strip_prefix("gitdir: ")?.trim();
        // Three levels up from .git/worktrees/<name> → main repo
        PathBuf::from(gitdir).parent()?.parent()?.parent().map(|p| p.to_path_buf())
    } else {
        Some(repo_root)
    }
}

/// Fetch --prune from origin. Errors are non-fatal (offline dev is common).
pub fn fetch_origin(repo_root: &Path) -> Result<()> {
    let status = std::process::Command::new("git")
        .args(["-C", &repo_root.to_string_lossy(), "fetch", "--prune", "--quiet"])
        .status()
        .context("git fetch")?;
    if !status.success() {
        anyhow::bail!("git fetch non-zero");
    }
    Ok(())
}

/// List branches (local first by recency, then remote-only) with worktree annotations.
pub fn list_branches(repo_root: &Path) -> Result<Vec<BranchEntry>> {
    let root_str = repo_root.to_string_lossy();

    // Build worktree map: branch → path
    let wt_out = std::process::Command::new("git")
        .args(["-C", &root_str, "worktree", "list", "--porcelain"])
        .output()
        .context("git worktree list")?;
    let mut wt_map: HashMap<String, String> = HashMap::new();
    let mut cur_path = String::new();
    for line in String::from_utf8_lossy(&wt_out.stdout).lines() {
        if let Some(p) = line.strip_prefix("worktree ") {
            cur_path = p.to_string();
        } else if let Some(b) = line.strip_prefix("branch refs/heads/") {
            wt_map.insert(b.to_string(), cur_path.clone());
        }
    }

    // Local branches
    let local_out = std::process::Command::new("git")
        .args(["-C", &root_str, "branch", "--sort=-committerdate", "--format=%(refname:short)"])
        .output()
        .context("git branch")?;
    let mut branches: Vec<BranchEntry> = String::from_utf8_lossy(&local_out.stdout)
        .lines()
        .filter(|l| !l.is_empty())
        .map(|name| BranchEntry {
            name: name.to_string(),
            is_remote: false,
            worktree_path: wt_map.get(name).cloned(),
        })
        .collect();

    // Remote-only branches (not already covered by a local branch)
    let local_set: std::collections::HashSet<String> =
        branches.iter().map(|b| b.name.clone()).collect();
    let remote_out = std::process::Command::new("git")
        .args(["-C", &root_str, "branch", "-r", "--sort=-committerdate",
               "--format=%(refname:short)"])
        .output()
        .context("git branch -r")?;
    for line in String::from_utf8_lossy(&remote_out.stdout).lines() {
        let name = match line.trim().strip_prefix("origin/") {
            Some(n) => n,
            None => continue,
        };
        if name.is_empty() || name == "HEAD" || local_set.contains(name) { continue; }
        branches.push(BranchEntry {
            name: name.to_string(),
            is_remote: true,
            worktree_path: None,
        });
    }

    Ok(branches)
}

/// Derive a folder name for a new worktree: `<repo>-<safe-branch>` (max 50 chars total).
pub fn branch_to_folder(repo_root: &Path, branch: &str) -> String {
    let repo_name = repo_root
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "repo".to_string());
    let safe = branch
        .replace('/', "-")
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
        .take(35)
        .collect::<String>();
    format!("{}-{}", repo_name, safe)
}

/// Create a git worktree at `worktree_path` for `branch`.
/// Creates a local tracking branch if the branch is remote-only.
pub fn create_worktree(repo_root: &Path, worktree_path: &Path, branch: &str) -> Result<()> {
    let root_str = repo_root.to_string_lossy();

    // Ensure local branch exists
    let local_exists = std::process::Command::new("git")
        .args(["-C", &root_str, "show-ref", "--verify", "--quiet",
               &format!("refs/heads/{}", branch)])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if !local_exists {
        let _ = std::process::Command::new("git")
            .args(["-C", &root_str, "branch", branch, &format!("origin/{}", branch)])
            .status();
    }

    let status = std::process::Command::new("git")
        .args(["-C", &root_str, "worktree", "add",
               &worktree_path.to_string_lossy(), branch])
        .status()
        .context("git worktree add")?;

    if !status.success() {
        anyhow::bail!("git worktree add failed");
    }
    Ok(())
}

/// Remove a git worktree by path and prune stale entries.
pub fn remove_worktree(repo_root: &Path, worktree_path: &Path) -> Result<()> {
    let root_str = repo_root.to_string_lossy();
    let _ = std::process::Command::new("git")
        .args(["-C", &root_str, "worktree", "remove", "--force",
               &worktree_path.to_string_lossy()])
        .status();
    let _ = std::process::Command::new("git")
        .args(["-C", &root_str, "worktree", "prune"])
        .status();
    Ok(())
}
