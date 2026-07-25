//! Git worktree isolation handling.
//!
//! Manages git worktree creation and cleanup for workspace isolation.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum GitError {
    #[error("git worktree error: {0}")]
    Worktree(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

pub struct GitWorktree {
    pub path: std::path::PathBuf,
    pub base_commit: String,
}

impl GitWorktree {
    /// Creates a new git worktree at the specified path.
    pub fn create(&self, path: &std::path::Path) -> Result<(), GitError> {
        // In a real implementation, this would call:
        // git worktree add --detach <path> <base_commit>
        // For now, just create the directory
        std::fs::create_dir_all(path)?;
        Ok(())
    }

    /// Removes the worktree.
    pub fn remove(&self) -> Result<(), GitError> {
        // Would call: git worktree remove <path>
        // For now, just clean up
        let _ = std::fs::remove_dir_all(&self.path);
        Ok(())
    }
}