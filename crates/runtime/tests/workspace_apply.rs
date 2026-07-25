//! Workspace apply placeholder test.
use batman_protocol::{LeaseMode, ProjectId, RunId};
use batman_runtime::workspace::LeaseService;

fn test_project_id() -> ProjectId {
    ProjectId::parse("01900000-0000-0000-0000-000000000001").unwrap()
}

fn test_run_id(n: u32) -> RunId {
    RunId::parse(&format!("01900000-0000-0000-0000-00000000000{0}", n)).unwrap()
}

#[test]
fn apply_patch_strategy() {
    let service = LeaseService::open_in_memory(test_project_id()).unwrap();
    let run = test_run_id(1);

    let lease = service.acquire(run, LeaseMode::Write, None)
        .expect("acquire write lease");

    let info = service.get(lease.lease_id.clone()).unwrap();
    assert_eq!(info.mode, LeaseMode::Write);

    service.release(lease.lease_id).unwrap();
}

