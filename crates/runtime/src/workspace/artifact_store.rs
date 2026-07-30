//! Artifact store for persisting and retrieving workspace artifacts.
//!
//! Stores artifacts with full metadata (kind, SHA-256, length, media type,
//! storage path, run_id) and supports bounded base64 chunked fetch.

use batman_protocol::{
    Artifact, ArtifactFetchResult, ArtifactId, ArtifactKind, ArtifactListResult,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::RwLock;

#[derive(Debug, Error)]
pub enum ArtifactStoreError {
    #[error("artifact not found: {0}")]
    NotFound(ArtifactId),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("storage error: {0}")]
    Storage(String),
}

/// In-memory artifact store with metadata tracking.
#[derive(Debug, Clone)]
pub struct ArtifactStore {
    /// Maps artifact ID to its metadata and content.
    artifacts: Arc<RwLock<HashMap<ArtifactId, StoredArtifact>>>,
    /// Base directory for on-disk storage (optional).
    storage_dir: Option<PathBuf>,
}

#[derive(Debug, Clone)]
struct StoredArtifact {
    metadata: Artifact,
    content: Vec<u8>,
}

impl ArtifactStore {
    /// Creates a new in-memory artifact store.
    pub fn new() -> Self {
        ArtifactStore {
            artifacts: Arc::new(RwLock::new(HashMap::new())),
            storage_dir: None,
        }
    }

    /// Creates a new artifact store with on-disk persistence.
    pub fn with_storage(base_dir: PathBuf) -> Result<Self, ArtifactStoreError> {
        std::fs::create_dir_all(&base_dir)?;
        Ok(ArtifactStore {
            artifacts: Arc::new(RwLock::new(HashMap::new())),
            storage_dir: Some(base_dir),
        })
    }

    /// Stores an artifact with full metadata.
    pub async fn store(
        &self,
        artifact: Artifact,
        content: Vec<u8>,
    ) -> Result<ArtifactId, ArtifactStoreError> {
        let id = artifact.artifact_id;

        // If on-disk storage is configured, write the content
        if let Some(ref storage_dir) = self.storage_dir {
            let storage_path = &artifact.storage_path;
            let full_path = storage_dir.join(storage_path);
            if let Some(parent) = full_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&full_path, &content)?;
        }

        let mut artifacts = self.artifacts.write().await;
        artifacts.insert(id, StoredArtifact {
            metadata: artifact,
            content,
        });

        Ok(id)
    }

    /// Fetches an artifact's metadata.
    pub async fn fetch(&self, id: &ArtifactId) -> Result<Artifact, ArtifactStoreError> {
        let artifacts = self.artifacts.read().await;
        artifacts.get(id).map(|a| a.metadata.clone())
            .ok_or(ArtifactStoreError::NotFound(*id))
    }

    /// Fetches an artifact's content (bytes).
    pub async fn fetch_content(&self, id: &ArtifactId) -> Result<Vec<u8>, ArtifactStoreError> {
        let artifacts = self.artifacts.read().await;
        artifacts.get(id).map(|a| a.content.clone())
            .ok_or(ArtifactStoreError::NotFound(*id))
    }

    /// Fetches a bounded chunk of an artifact's content as base64.
    pub async fn fetch_chunked(
        &self,
        id: &ArtifactId,
        offset: u64,
        length: u64,
    ) -> Result<ArtifactFetchResult, ArtifactStoreError> {
        let artifacts = self.artifacts.read().await;
        let stored = artifacts.get(id).ok_or(ArtifactStoreError::NotFound(*id))?;

        let metadata = &stored.metadata;
        let content = &stored.content;

        // Calculate the chunk
        let end = std::cmp::min(offset + length, content.len() as u64);
        let chunk = if offset >= content.len() as u64 {
            vec![]
        } else {
            content[offset as usize..end as usize].to_vec()
        };

        // Base64 encode the chunk
        let content_base64 = base64_encode(&chunk);

        let next_offset = if end < content.len() as u64 {
            Some(end)
        } else {
            None
        };

        Ok(ArtifactFetchResult {
            artifact: metadata.clone(),
            content_base64,
            next_offset,
            complete: next_offset.is_none(),
        })
    }

    /// Lists all artifacts, optionally filtered by kind.
    pub async fn list(&self, kind: Option<ArtifactKind>) -> ArtifactListResult {
        let artifacts = self.artifacts.read().await;
        let filtered: Vec<Artifact> = if let Some(k) = kind {
            artifacts.values().filter(|a| a.metadata.kind == k).map(|a| a.metadata.clone()).collect()
        } else {
            artifacts.values().map(|a| a.metadata.clone()).collect()
        };

        ArtifactListResult { artifacts: filtered }
    }
}

impl Default for ArtifactStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Simple base64 encoding.
fn base64_encode(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::new();
    let mut i = 0;
    while i < data.len() {
        let b0 = data[i] as u32;
        let b1 = if i + 1 < data.len() { data[i + 1] as u32 } else { 0 };
        let b2 = if i + 2 < data.len() { data[i + 2] as u32 } else { 0 };

        let triple = (b0 << 16) | (b1 << 8) | b2;

        result.push(CHARS[((triple >> 18) & 0x3F) as usize] as char);
        result.push(CHARS[((triple >> 12) & 0x3F) as usize] as char);

        if i + 1 < data.len() {
            result.push(CHARS[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }

        if i + 2 < data.len() {
            result.push(CHARS[(triple & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }

        i += 3;
    }
    result
}
