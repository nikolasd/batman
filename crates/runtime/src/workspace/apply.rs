//! Workspace apply with artifact store integration.

use batman_protocol::{ApplyRequest, ApplyResult};
use std::sync::Arc;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ApplyError {
    #[error("artifact not found: {0}")]
    ArtifactNotFound(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
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

    pub async fn apply(&self, request: &ApplyRequest) -> Result<ApplyResult, ApplyError> {
        let store = self.store.as_ref()
            .ok_or_else(|| ApplyError::ArtifactNotFound("no artifact store".to_string()))?;

        let content = store.fetch(request.artifact_id)
            .await
            .ok_or_else(|| ApplyError::ArtifactNotFound(request.artifact_id.to_string()))?;

        // Apply content to workspace - write as binary file
        std::fs::create_dir_all(&self.path)?;
        std::fs::write(self.path.join("artifact_content.bin"), &content)?;

        Ok(ApplyResult {
            lease_id: request.lease_id.clone(),
            success: true,
            conflict_artifact_id: None,
            target_revision_after: None,
            error_code: None,
        })
    }
}
