//! Git worktree and branch management
//!
//! Provides operations for listing branches and managing worktrees.

use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};
use git2::Repository;

use super::GitContext;

impl GitContext {
    /// List all local branch names in the repository
    pub fn list_branches(repo_path: &Path) -> Result<Vec<String>> {
        let repo = Repository::discover(repo_path).context("Failed to open repository")?;
        let mut branches = Vec::new();

        for branch_result in repo.branches(Some(git2::BranchType::Local))? {
            let (branch, _) = branch_result?;
            if let Ok(Some(name)) = branch.name() {
                branches.push(name.to_string());
            }
        }

        // Sort with main/master first, then alphabetically
        branches.sort_by(|a, b| {
            let a_is_main = a == "main" || a == "master";
            let b_is_main = b == "main" || b == "master";
            match (a_is_main, b_is_main) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => a.cmp(b),
            }
        });

        Ok(branches)
    }

    /// Create a new worktree for a branch using the git CLI.
    /// - If `is_new_branch` is true: creates a new branch from HEAD
    /// - If `is_new_branch` is false: checks out an existing branch
    pub fn create_worktree(
        repo_path: &Path,
        worktree_path: &Path,
        branch_name: &str,
        is_new_branch: bool,
    ) -> Result<()> {
        if worktree_path.exists() {
            anyhow::bail!("Path '{}' already exists", worktree_path.display());
        }

        let mut cmd = Command::new("git");
        cmd.arg("-C").arg(repo_path);
        cmd.arg("worktree").arg("add");

        if is_new_branch {
            cmd.arg("-b").arg(branch_name);
        }

        cmd.arg(worktree_path);

        if !is_new_branch {
            cmd.arg(branch_name);
        }

        let output = cmd.output().context("Failed to execute git worktree add")?;

        if output.status.success() {
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!(
                "Failed to create worktree for '{}' at '{}': {}",
                branch_name,
                worktree_path.display(),
                stderr.trim()
            )
        }
    }

    /// List all branches (local + remote), with remote branches prefixed by their remote name.
    pub fn list_all_branches(repo_path: &Path) -> Result<Vec<String>> {
        let repo = Repository::discover(repo_path).context("Failed to open repository")?;
        let mut branches = Vec::new();

        // Local branches
        for branch_result in repo.branches(Some(git2::BranchType::Local))? {
            let (branch, _) = branch_result?;
            if let Ok(Some(name)) = branch.name() {
                branches.push(name.to_string());
            }
        }

        // Remote branches (skip HEAD references)
        for branch_result in repo.branches(Some(git2::BranchType::Remote))? {
            let (branch, _) = branch_result?;
            if let Ok(Some(name)) = branch.name() {
                if name.ends_with("/HEAD") {
                    continue;
                }
                branches.push(name.to_string());
            }
        }

        // Sort: main/master first, then local branches, then remote branches
        branches.sort_by(|a, b| {
            let a_is_main = a == "main" || a == "master";
            let b_is_main = b == "main" || b == "master";
            let a_is_remote = a.contains('/');
            let b_is_remote = b.contains('/');
            match (a_is_main, b_is_main) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => match (a_is_remote, b_is_remote) {
                    (false, true) => std::cmp::Ordering::Less,
                    (true, false) => std::cmp::Ordering::Greater,
                    _ => a.cmp(b),
                },
            }
        });

        Ok(branches)
    }

    /// Delete the worktree at the given path using `git worktree remove`
    /// Returns an error if the worktree has uncommitted changes (unless force=true)
    pub fn delete_worktree(worktree_path: &Path, force: bool) -> Result<()> {
        // Verify it's actually a worktree
        let repo = Repository::discover(worktree_path).context("Failed to open repository")?;
        if !repo.is_worktree() {
            anyhow::bail!(
                "'{}' is not a worktree (it may be the main repository)",
                worktree_path.display()
            );
        }

        // Use git CLI for worktree removal - run from the worktree itself
        let mut cmd = Command::new("git");
        cmd.arg("-C").arg(worktree_path);
        cmd.arg("worktree").arg("remove");

        if force {
            cmd.arg("--force");
        }

        cmd.arg(worktree_path);

        let output = cmd
            .output()
            .context("Failed to execute git worktree remove")?;

        if output.status.success() {
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let hint = if stderr.contains("contains modified or untracked files") {
                " Commit or stash your changes first, or use force delete."
            } else if stderr.contains("is locked") {
                &format!(
                    " Unlock it first with: git worktree unlock {}",
                    worktree_path.display()
                )
            } else {
                ""
            };

            anyhow::bail!(
                "git worktree remove failed: {}.{}",
                stderr.trim(),
                hint
            )
        }
    }
}
