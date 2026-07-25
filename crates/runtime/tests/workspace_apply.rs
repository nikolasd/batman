//! Workspace apply and inspection tests.
//!
//! Tests for evidence capture and workspace applying.

use batman_protocol::{ApplyRequest, ApplyStrategy, InspectRequest, LeaseMode, ProjectId, RunId};
use batman_runtime::workspace::{LeaseService, WorkspaceApplier, WorkspaceInspector};

fn test_project_id() -> ProjectId {
    ProjectId::parse("01900000-0000-0000-0000-000000000001").unwrap()
}

fn test_run_id(n: u32) -> RunId {
    RunId::parse(&format!("01900000-0000-0000-0000-00000000000{0}", n)).unwrap()
}

#[test]
fn inspector_creates_workspace_inspect() {
    let service = LeaseService::open_in_memory(test_project_id()).unwrap();
    let run = test_run_id(1);
    
    let lease = service.acquire(run, LeaseMode::ReadOnly, None)
        .expect("acquire read lease");

    let inspector = WorkspaceInspector::new(std::path::PathBuf::from(&lease.path));
    let request = InspectRequest {
        lease_id: lease.lease_id.clone(),
    };
    
    let result = inspector.inspect(&request).unwrap();
    assert_eq!(result.lease_id, lease.lease_id);

    service.release(lease.lease_id).unwrap();
}

#[test]
fn applier_returns_apply_result() {
    let service = LeaseService::open_in_memory(test_project_id()).unwrap();
    let run = test_run_id(2);
    
    let lease = service.acquire(run, LeaseMode::Write, None)
        .expect("acquire write lease");

    let applier = WorkspaceApplier::new(std::path::PathBuf::from(&lease.path));
    let request = ApplyRequest {
        lease_id: lease.lease_id.clone(),
        strategy: ApplyStrategy::ApplyPatch,
        artifact_id: Default::default(),
        expected_target_revision: "HEAD".to_string(),
        approval_correlation_id: None,
    };
    
    let result = applier.apply(&request).unwrap();
    assert_eq!(result.lease_id, lease.lease_id);
    assert!(result.success);

    service.release(lease.lease_id).unwrap();
}
