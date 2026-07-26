//! Workspace apply integration tests.

use batman_protocol::{ArtifactId, ApplyRequest, ApplyStrategy, LeaseMode, ProjectId, RunId};
use batman_runtime::workspace::{ArtifactStore, LeaseService, WorkspaceApplier};
use std::path::PathBuf;

fn project_id() -> ProjectId {
    ProjectId::parse("01900000-0000-0000-0000-000000000001").unwrap()
}

fn run_id(n: u32) -> RunId {
    RunId::parse(&format!("01900000-0000-0000-0000-00000000000{0}", n)).unwrap()
}

#[tokio::test]
async fn artifact_store_list_and_fetch() {
    let store = ArtifactStore::new();

    let content = b"test content".to_vec();
    let id = store.store(content.clone()).await;

    let list = store.list().await;
    assert!(list.contains(&id));

    let fetched = store.fetch(id).await;
    assert_eq!(fetched, Some(content));
}

#[tokio::test]
async fn artifact_store_missing_artifact() {
    let store = ArtifactStore::new();
    let missing = ArtifactId::new();
    let result = store.fetch(missing).await;
    assert!(result.is_none());
}

#[tokio::test]
async fn workspace_apply_with_artifact() {
    let store = ArtifactStore::new();
    let service = LeaseService::open_in_memory(project_id()).unwrap();
    let run = run_id(1);

    let lease = service.acquire(run, LeaseMode::Write, None)
        .expect("acquire write lease");

    let artifact_content = b"patch data".to_vec();
    let artifact_id = store.store(artifact_content.clone()).await;

    let workspace = PathBuf::from(&lease.path);
    
    let applier = WorkspaceApplier::from_store(
        workspace.clone(),
        std::sync::Arc::new(store),
    );

    let request = ApplyRequest {
        lease_id: lease.lease_id.clone(),
        strategy: ApplyStrategy::ApplyPatch,
        artifact_id,
        expected_target_revision: "HEAD".to_string(),
        approval_correlation_id: None,
    };

    let result = applier.apply(&request).await.unwrap();
    assert!(result.success);

    // Verify the content was written to the workspace
    let written = std::fs::read(workspace.join("artifact_content.bin")).unwrap();
    assert_eq!(written, artifact_content);

    service.release(lease.lease_id).unwrap();
}

#[tokio::test]
async fn workspace_apply_missing_artifact_id() {
    let store = ArtifactStore::new();
    let service = LeaseService::open_in_memory(project_id()).unwrap();
    let run = run_id(2);

    let lease = service.acquire(run, LeaseMode::Write, None)
        .expect("acquire write lease");

    let workspace = PathBuf::from(&lease.path);
    
    let applier = WorkspaceApplier::from_store(
        workspace,
        std::sync::Arc::new(store),
    );

    let missing_id = ArtifactId::new();
    let request = ApplyRequest {
        lease_id: lease.lease_id.clone(),
        strategy: ApplyStrategy::ApplyPatch,
        artifact_id: missing_id,
        expected_target_revision: "HEAD".to_string(),
        approval_correlation_id: None,
    };

    let result = applier.apply(&request).await;
    assert!(result.is_err(), "missing artifact ID should fail");

    service.release(lease.lease_id).unwrap();
}
