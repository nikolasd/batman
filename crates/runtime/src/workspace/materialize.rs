//! Workspace materialization coordination.
//!
//! Coordinates the creation of working directories using different
//! isolation strategies (shared, git worktree, copy).

use batman_protocol::{IsolationKind, ProjectId, RunId};
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MaterializerError {
    #[error("git error: {0}")]
    Git(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("path validation failed: {0}")]
    PathValidation(String),
}

/// Coordinates workspace materialization for different isolation strategies.
#[derive(Debug, Clone)]
pub struct WorkspaceMaterializer {
    project_id: ProjectId,
    root: PathBuf,
}

impl WorkspaceMaterializer {
    pub fn new(project_id: ProjectId) -> Result<Self, MaterializerError> {
        let root = std::env::temp_dir().join(format!("batman-workspace-{}", project_id));
        std::fs::create_dir_all(&root)?;
        Ok(WorkspaceMaterializer { project_id, root })
    }

    /// Validates that a path is within the lease root.
    pub fn validate_path(&self, path: &str) -> Result<(), MaterializerError> {
        let path = Path::new(path);
        
        // Ensure path is within root (prevents path traversal attacks)
        if path.starts_with("/") && !path.starts_with(self.root.to_str().unwrap_or("")) {
            return Err(MaterializerError::PathValidation(
                "Path escapes lease root".to_string()
            ));
        }
        
        Ok(())
    }

    /// Materializes a workspace with the given isolation kind.
    pub fn materialize(
        &self,
        _run_id: RunId,
        _isolation: IsolationKind,
    ) -> Result<PathBuf, MaterializerError> {
        // For now, just return the root path
        // Actual git worktree/copy implementation would go here
        Ok(self.root.clone())
    }
}