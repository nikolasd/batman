//! Git worktree isolation handling.
//!
//! Manages git worktree creation and cleanup for workspace isolation.

use std::path::Path;
use std::process::Command;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum GitError {
    #[error("git worktree error: {0}")]
    Worktree(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

#[allow(dead_code)]
pub struct GitWorktree {
    pub repository: std::path::PathBuf,
    pub path: std::path::PathBuf,
    pub base_commit: String,
}

impl GitWorktree {
    /// Creates a new git worktree at the specified path.
    /// Executes `git worktree add --detach <path> <base_commit>` from the source repository.
    pub fn create(&self, path: &Path) -> Result<(), GitError> {
        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(GitError::Io)?;
        }

        // Execute git worktree add --detach from the source repository
        // Use .arg() with Path directly to preserve non-UTF-8 paths
        let output = Command::new("git")
            .current_dir(&self.repository)
            .arg("worktree")
            .arg("add")
            .arg("--detach")
            .arg(path)
            .arg(&self.base_commit)
            .output()
            .map_err(|e| GitError::Worktree(format!("Failed to execute git: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(GitError::Worktree(format!(
                "git worktree add failed: {}",
                stderr
            )));
        }

        Ok(())
    }

    /// Removes the worktree.
    /// Executes `git worktree remove <path>` from the source repository.
    #[allow(dead_code)]
    pub fn remove(&self) -> Result<(), GitError> {
        // Use .arg() with Path directly to preserve non-UTF-8 paths
        let output = Command::new("git")
            .current_dir(&self.repository)
            .arg("worktree")
            .arg("remove")
            .arg(&self.path)
            .output()
            .map_err(|e| GitError::Worktree(format!("Failed to execute git: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(GitError::Worktree(format!(
                "git worktree remove failed: {}",
                stderr
            )));
        }

        Ok(())
    }
}
