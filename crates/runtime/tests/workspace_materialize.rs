//! Workspace materialization tests.
//!
//! Tests for git worktree and copy isolation strategies.

use batman_protocol::{IsolationKind, LeaseMode, ProjectId, RunId};
use batman_runtime::workspace::{LeaseService, WorkspaceMaterializer};

fn test_project_id() -> ProjectId {
    ProjectId::parse("01900000-0000-0000-0000-000000000001").unwrap()
}

fn test_run_id(n: u32) -> RunId {
    RunId::parse(&format!("01900000-0000-0000-0000-00000000000{0}", n)).unwrap()
}

#[test]
fn workspace_materializer_create() {
    let materializer = WorkspaceMaterializer::new(test_project_id());
    assert!(materializer.is_ok());
}

#[test]
fn git_worktree_isolation() {
    let service = LeaseService::open_in_memory(test_project_id()).unwrap();
    let run = test_run_id(1);

    let lease = service.acquire(run, LeaseMode::Write, Some(IsolationKind::GitWorktree))
        .expect("acquire write lease with git worktree");

    assert_eq!(lease.isolation_kind, IsolationKind::GitWorktree);
    assert!(lease.path.starts_with("/tmp/"));

    service.release(lease.lease_id).unwrap();
}

#[test]
fn copy_isolation() {
    let service = LeaseService::open_in_memory(test_project_id()).unwrap();
    let run = test_run_id(2);

    let lease = service.acquire(run, LeaseMode::Write, Some(IsolationKind::Copy))
        .expect("acquire write lease with copy isolation");

    assert_eq!(lease.isolation_kind, IsolationKind::Copy);

    service.release(lease.lease_id).unwrap();
}

#[test]
fn path_guard_rejects_escape() {
    let materializer = WorkspaceMaterializer::new(test_project_id()).unwrap();
    
    // Path guard should reject paths outside the lease root
    let result = materializer.validate_path("/etc/passwd");
    assert!(result.is_err(), "path should be rejected for being outside lease root");
}