//! Workspace apply integration tests.

use batman_protocol::{
    ApplyRequest, ApplyStrategy, Artifact, ArtifactId, ArtifactKind, ProjectId,
};
use batman_runtime::workspace::{ArtifactStore, WorkspaceApplier, WorkspaceInspector};
use std::path::PathBuf;
use std::process::Command;

fn project_id() -> ProjectId {
    ProjectId::parse("01900000-0000-0000-0000-000000000001").unwrap()
}

/// Creates a fixture repository with sample files for testing.
fn create_fixture_repo() -> PathBuf {
    let repo = tempfile::TempDir::new()
        .expect("Failed to create temp dir")
        .into_path();

    // Initialize as a git repository
    Command::new("git")
        .current_dir(&repo)
        .args(["init"])
        .output()
        .expect("Failed to initialize git repo");

    Command::new("git")
        .current_dir(&repo)
        .args(["config", "user.email", "test@test.com"])
        .output()
        .expect("Failed to configure git user");

    Command::new("git")
        .current_dir(&repo)
        .args(["config", "user.name", "Test User"])
        .output()
        .expect("Failed to configure git user");

    // Create initial file and commit
    std::fs::write(repo.join("file1.txt"), "initial content\n").unwrap();
    Command::new("git")
        .current_dir(&repo)
        .args(["add", "."])
        .output()
        .expect("Failed to add files");

    Command::new("git")
        .current_dir(&repo)
        .args(["commit", "-m", "Initial commit"])
        .output()
        .expect("Failed to commit");

    repo
}

#[tokio::test]
async fn artifact_store_list_and_fetch() {
    let store = ArtifactStore::new();

    let content = b"test content".to_vec();
    let artifact = Artifact {
        artifact_id: ArtifactId::new(),
        kind: ArtifactKind::Patch,
        sha256: String::new(),
        byte_length: content.len() as u64,
        media_type: "text/plain".to_string(),
        storage_path: "test.patch".to_string(),
        run_id: None,
    };
    let id = store.store(artifact, content.clone()).await.unwrap();

    let list = store.list(None).await;
    assert_eq!(list.artifacts.len(), 1);
    assert_eq!(list.artifacts[0].artifact_id, id);

    let fetched = store.fetch(&id).await.unwrap();
    assert_eq!(fetched.artifact_id, id);
}

#[tokio::test]
async fn artifact_store_missing_artifact() {
    let store = ArtifactStore::new();
    let missing = ArtifactId::new();
    let result = store.fetch(&missing).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn workspace_apply_with_real_patch() {
    let repo = create_fixture_repo();
    let store = ArtifactStore::new();

    // Create a second clone to modify (source of the patch)
    let source = tempfile::TempDir::new().unwrap().into_path();
    std::fs::copy(repo.join("file1.txt"), source.join("file1.txt")).unwrap();
    Command::new("git")
        .current_dir(&source)
        .args(["init"])
        .output().ok();
    Command::new("git")
        .current_dir(&source)
        .args(["config", "user.email", "test@test.com"])
        .output().ok();
    Command::new("git")
        .current_dir(&source)
        .args(["config", "user.name", "Test User"])
        .output().ok();
    Command::new("git")
        .current_dir(&source)
        .args(["add", "."])
        .output().ok();
    Command::new("git")
        .current_dir(&source)
        .args(["commit", "-m", "Initial"])
        .output().ok();

    // Modify the source and generate a patch
    std::fs::write(source.join("file1.txt"), "modified content\n").unwrap();
    let patch_output = Command::new("git")
        .current_dir(&source)
        .args(["diff"])
        .output()
        .expect("Failed to generate diff");

    let patch_content = patch_output.stdout;
    assert!(!patch_content.is_empty(), "patch should be nonempty");

    // Store the patch
    let artifact = Artifact {
        artifact_id: ArtifactId::new(),
        kind: ArtifactKind::Patch,
        sha256: String::new(),
        byte_length: patch_content.len() as u64,
        media_type: "application/x-git-diff".to_string(),
        storage_path: "test.patch".to_string(),
        run_id: None,
    };
    let artifact_id = store.store(artifact, patch_content).await.unwrap();

    // Get the current HEAD before applying
    let head_output = Command::new("git")
        .current_dir(&repo)
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("Failed to get HEAD");
    let expected_head = String::from_utf8_lossy(&head_output.stdout).trim().to_string();

    let applier = WorkspaceApplier::from_store(
        repo.clone(),
        std::sync::Arc::new(store.clone()),
    );

    let request = ApplyRequest {
        lease_id: "test-lease".to_string(),
        strategy: ApplyStrategy::ApplyPatch,
        artifact_id,
        expected_target_revision: expected_head,
        approval_correlation_id: None,
    };

    let result = applier.apply(&request).await.unwrap();
    assert!(result.success, "apply should succeed");

    // Verify the file was modified
    let content = std::fs::read_to_string(repo.join("file1.txt")).unwrap();
    assert_eq!(content, "modified content\n");
}

#[tokio::test]
async fn workspace_apply_stale_revision_returns_conflict() {
    let repo = create_fixture_repo();
    let store = ArtifactStore::new();

    // Create a second clone to generate the patch from (source)
    let source = tempfile::TempDir::new().unwrap().into_path();
    std::fs::copy(repo.join("file1.txt"), source.join("file1.txt")).unwrap();
    Command::new("git")
        .current_dir(&source)
        .args(["init"])
        .output().ok();
    Command::new("git")
        .current_dir(&source)
        .args(["config", "user.email", "test@test.com"])
        .output().ok();
    Command::new("git")
        .current_dir(&source)
        .args(["config", "user.name", "Test User"])
        .output().ok();
    Command::new("git")
        .current_dir(&source)
        .args(["add", "."])
        .output().ok();
    Command::new("git")
        .current_dir(&source)
        .args(["commit", "-m", "Initial"])
        .output().ok();

    // Modify the source and generate a patch
    std::fs::write(source.join("file1.txt"), "modified content\n").unwrap();
    let patch_output = Command::new("git")
        .current_dir(&source)
        .args(["diff"])
        .output()
        .expect("Failed to generate diff");

    let artifact = Artifact {
        artifact_id: ArtifactId::new(),
        kind: ArtifactKind::Patch,
        sha256: String::new(),
        byte_length: patch_output.stdout.len() as u64,
        media_type: "application/x-git-diff".to_string(),
        storage_path: "test.patch".to_string(),
        run_id: None,
    };
    let artifact_id = store.store(artifact, patch_output.stdout).await.unwrap();

    // Use a STALE revision (not the current HEAD)
    let stale_revision = "0000000000000000000000000000000000000000";

    let applier = WorkspaceApplier::from_store(
        repo.clone(),
        std::sync::Arc::new(store.clone()),
    );

    let request = ApplyRequest {
        lease_id: "test-lease".to_string(),
        strategy: ApplyStrategy::ApplyPatch,
        artifact_id,
        expected_target_revision: stale_revision.to_string(),
        approval_correlation_id: None,
    };

    let result = applier.apply(&request).await.unwrap();
    assert!(!result.success, "apply should fail with stale revision");
    assert_eq!(result.error_code.as_deref(), Some("STALE_REVISION"));

    // Verify the workspace was NOT mutated
    let content = std::fs::read_to_string(repo.join("file1.txt")).unwrap();
    assert_eq!(content, "initial content\n");
}

#[tokio::test]
async fn workspace_inspect_captures_real_evidence() {
    let repo = create_fixture_repo();
    let store = ArtifactStore::new();

    // Modify a file to create dirty state
    std::fs::write(repo.join("file1.txt"), "dirty content\n").unwrap();

    // Create an untracked file
    std::fs::write(repo.join("untracked.txt"), "untracked\n").unwrap();

    let inspector = WorkspaceInspector::with_store(
        repo.clone(),
        std::sync::Arc::new(store.clone()),
    );

    let request = batman_protocol::InspectRequest {
        lease_id: "test-lease".to_string(),
    };

    let result = inspector.inspect(&request).await.unwrap();

    // Verify real evidence was captured
    assert_eq!(result.lease_id, "test-lease");
    assert!(result.dirty_file_count > 0, "should have dirty files");
    assert!(result.untracked_file_count > 0, "should have untracked files");
    assert!(!result.commit_ids.is_empty(), "should have commits");
    assert!(!result.base_revision.is_empty(), "should have base revision");
    assert!(!result.patch_artifact_id.to_string().is_empty(), "should have patch artifact ID");

    // Verify the patch was stored
    let list = store.list(None).await;
    assert!(!list.artifacts.is_empty(), "should have stored artifacts");
}
