//! Content-addressed artifact storage contracts.
//!
//! Immutable artifacts for patches, commit lists, conflict reports, and
//! workspace manifests, stored under `artifacts/sha256/<prefix>/<digest>`.

use crate::ids::ArtifactId;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The kind of an artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ArtifactKind {
    Patch,
    CommitList,
    ConflictReport,
    WorkspaceManifest,
}

/// Metadata for a stored artifact.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct Artifact {
    pub artifact_id: ArtifactId,
    pub kind: ArtifactKind,
    pub sha256: String,
    #[ts(type = "number")]
    pub byte_length: u64,
    pub media_type: String,
    /// The relative storage path under the artifacts directory.
    pub storage_path: String,
    /// The run ID that produced this artifact, if known.
    pub run_id: Option<String>,
}

/// Parameters for listing artifacts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct ArtifactListRequest {
    pub project_id: String,
    pub kind: Option<ArtifactKind>,
}

/// Result of listing artifacts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct ArtifactListResult {
    pub artifacts: Vec<Artifact>,
}

/// Parameters for fetching an artifact's content.
///
/// Returns a bounded byte/text chunk plus the next offset; callers
/// iterate explicitly for larger artifacts. Capped at 256 KiB per call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct ArtifactFetchRequest {
    pub artifact_id: ArtifactId,
    #[ts(type = "number")]
    pub offset: u64,
    #[ts(type = "number")]
    pub length: u64,
}

/// Result of fetching an artifact.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct ArtifactFetchResult {
    pub artifact: Artifact,
    /// Base64-encoded chunk of artifact bytes; callers decode explicitly.
    /// Capped at 256 KiB per call.
    pub content_base64: String,
    #[ts(type = "number | null")]
    pub next_offset: Option<u64>,
    pub complete: bool,
}
