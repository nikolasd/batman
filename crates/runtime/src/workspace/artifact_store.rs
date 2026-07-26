//! Artifact store for persisting and retrieving workspace artifacts.

use batman_protocol::ArtifactId;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Persistent store for workspace artifacts.
#[derive(Debug, Clone)]
pub struct ArtifactStore {
    artifacts: Arc<RwLock<HashMap<ArtifactId, Vec<u8>>>>,
}

impl ArtifactStore {
    pub fn new() -> Self {
        ArtifactStore {
            artifacts: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn store(&self, content: Vec<u8>) -> ArtifactId {
        let id = ArtifactId::new();
        let mut artifacts = self.artifacts.write().await;
        artifacts.insert(id, content);
        id
    }

    pub async fn fetch(&self, id: ArtifactId) -> Option<Vec<u8>> {
        let artifacts = self.artifacts.read().await;
        artifacts.get(&id).cloned()
    }

    pub async fn list(&self) -> Vec<ArtifactId> {
        let artifacts = self.artifacts.read().await;
        artifacts.keys().cloned().collect()
    }
}

impl Default for ArtifactStore {
    fn default() -> Self {
        Self::new()
    }
}