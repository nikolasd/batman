//! Workspace inspection implementation.

use batman_protocol::{InspectRequest, InspectResult};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum InspectError {
    #[error("inspection error: {0}")]
    Inspect(String),
}

pub struct WorkspaceInspector {
    path: std::path::PathBuf,
}

impl WorkspaceInspector {
    pub fn new(path: std::path::PathBuf) -> Self {
        WorkspaceInspector { path }
    }

    pub fn inspect(&self, _request: &InspectRequest) -> Result<InspectResult, InspectError> {
        // Would run git diff, status, etc.
        Ok(InspectResult {
            lease_id: _request.lease_id.clone(),
            patch_artifact_id: Default::default(),
            commit_count: 0,
            commit_ids: vec![],
            dirty_file_count: 0,
            untracked_file_count: 0,
            base_revision: "HEAD".to_string(),
            current_revision: None,
        })
    }
}
