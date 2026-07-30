//! Workspace apply with artifact store integration.
//!
//! Applies workspace changes by fetching artifacts and applying them
//! using the specified strategy (ApplyPatch or CherryPick).

use batman_protocol::{ApplyRequest, ApplyResult, ApplyStrategy};
use std::process::Command;
use std::sync::Arc;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ApplyError {
    #[error("artifact not found: {0}")]
    ArtifactNotFound(String),
    #[error("git error: {0}")]
    Git(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("conflict: expected revision {expected}, got {actual}")]
    StaleRevision { expected: String, actual: String },
    #[error("conflict: {0}")]
    Conflict(String),
}

/// Workspace applier that fetches artifacts from a store and applies them.
pub struct WorkspaceApplier {
    path: std::path::PathBuf,
    store: Option<Arc<crate::workspace::ArtifactStore>>,
}

impl WorkspaceApplier {
    pub fn new(path: std::path::PathBuf) -> Self {
        WorkspaceApplier {
            path,
            store: None,
        }
    }

    pub fn from_store(path: std::path::PathBuf, store: Arc<crate::workspace::ArtifactStore>) -> Self {
        WorkspaceApplier {
            path,
            store: Some(store),
        }
    }

    /// Applies a workspace change using the specified strategy.
    /// Validates `expected_target_revision` before mutating.
    pub async fn apply(&self, request: &ApplyRequest) -> Result<ApplyResult, ApplyError> {
        let store = self.store.as_ref()
            .ok_or_else(|| ApplyError::ArtifactNotFound("no artifact store".to_string()))?;

        // Fetch the artifact content
        let content = store.fetch_content(&request.artifact_id)
            .await
            .map_err(|e| ApplyError::ArtifactNotFound(e.to_string()))?;

        // Validate expected_target_revision BEFORE any mutation
        let current_head = self.get_current_head()?;
        if current_head != request.expected_target_revision {
            return Ok(ApplyResult {
                lease_id: request.lease_id.clone(),
                success: false,
                conflict_artifact_id: None,
                target_revision_after: Some(current_head),
                error_code: Some("STALE_REVISION".to_string()),
            });
        }

        match request.strategy {
            ApplyStrategy::ApplyPatch => {
                self.apply_patch(&content, &request.lease_id)
            }
            ApplyStrategy::CherryPick => {
                self.cherry_pick(&content, &request.lease_id)
            }
        }
    }

    /// Gets the current HEAD revision.
    fn get_current_head(&self) -> Result<String, ApplyError> {
        let output = Command::new("git")
            .current_dir(&self.path)
            .args(["rev-parse", "HEAD"])
            .output()
            .map_err(|e| ApplyError::Git(format!("Failed to execute git: {}", e)))?;

        if !output.status.success() {
            return Err(ApplyError::Git("Failed to get HEAD".to_string()));
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    /// Applies a patch file to the workspace.
    fn apply_patch(&self, patch_content: &[u8], lease_id: &str) -> Result<ApplyResult, ApplyError> {
        // Write patch to a temporary file
        let patch_path = self.path.join(format!("incoming_{}.patch", lease_id));
        std::fs::write(&patch_path, patch_content)?;

        // Apply the patch using git apply
        let status = Command::new("git")
            .current_dir(&self.path)
            .args(["apply", "--check", patch_path.to_str().unwrap_or("")])
            .status()
            .map_err(|e| ApplyError::Git(format!("Failed to execute git: {}", e)))?;

        if !status.success() {
            let _ = std::fs::remove_file(&patch_path);
            return Err(ApplyError::Conflict(
                "Patch does not apply cleanly".to_string()
            ));
        }

        // Apply the patch for real
        let status = Command::new("git")
            .current_dir(&self.path)
            .args(["apply", patch_path.to_str().unwrap_or("")])
            .status()
            .map_err(|e| ApplyError::Git(format!("Failed to execute git: {}", e)))?;

        let _ = std::fs::remove_file(&patch_path);

        if !status.success() {
            return Err(ApplyError::Conflict(
                "Patch application failed".to_string()
            ));
        }

        // Get the new HEAD
        let target_revision_after = self.get_current_head().ok();

        Ok(ApplyResult {
            lease_id: lease_id.to_string(),
            success: true,
            conflict_artifact_id: None,
            target_revision_after,
            error_code: None,
        })
    }

    /// Cherry-picks commits from the artifact.
    fn cherry_pick(&self, commit_content: &[u8], lease_id: &str) -> Result<ApplyResult, ApplyError> {
        // The artifact should contain commit IDs to cherry-pick
        let content_str = String::from_utf8_lossy(commit_content);
        let commit_ids: Vec<&str> = content_str
            .lines()
            .filter(|l| !l.is_empty())
            .collect();

        if commit_ids.is_empty() {
            return Err(ApplyError::Git("No commit IDs in artifact".to_string()));
        }

        // Cherry-pick each commit
        for commit_id in &commit_ids {
            let status = Command::new("git")
                .current_dir(&self.path)
                .args(["cherry-pick", commit_id])
                .status()
                .map_err(|e| ApplyError::Git(format!("Failed to execute git: {}", e)))?;

            if !status.success() {
                // Get conflict info
                // Try to get conflict info, but don't fail if git diff fails
                let conflict_files: Vec<String> = Command::new("git")
                    .current_dir(&self.path)
                    .args(["diff", "--name-only"])
                    .output()
                    .map(|output| {
                        String::from_utf8_lossy(&output.stdout)
                            .lines()
                            .filter(|l| !l.is_empty())
                            .map(|l| l.to_string())
                            .collect()
                    })
                    .unwrap_or_else(|_| vec!["unknown conflict".to_string()]);

                return Err(ApplyError::Conflict(format!(
                    "Cherry-pick failed: conflicting files: {:?}",
                    conflict_files
                )));
            }
        }

        // Get the new HEAD
        let target_revision_after = self.get_current_head().ok();

        Ok(ApplyResult {
            lease_id: lease_id.to_string(),
            success: true,
            conflict_artifact_id: None,
            target_revision_after,
            error_code: None,
        })
    }
}
