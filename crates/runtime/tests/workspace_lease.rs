//! Workspace lease arbitration tests.

use batman_protocol::{LeaseMode, ProjectId, RunId};
use batman_runtime::workspace::LeaseService;

fn test_project_id() -> ProjectId {
    ProjectId::parse("01900000-0000-0000-0000-000000000001").unwrap()
}

fn test_run_id(n: u32) -> RunId {
    RunId::parse(&format!("01900000-0000-0000-0000-00000000000{0}", n)).unwrap()
}

#[test]
fn multiple_shared_readonly_leases_succeed() {
    let service = LeaseService::open_in_memory(test_project_id()).unwrap();
    let run1 = test_run_id(1);
    let run2 = test_run_id(2);

    let lease1 = service
        .acquire(run1, LeaseMode::ReadOnly, None)
        .expect("first read-only lease");
    assert_eq!(lease1.mode, LeaseMode::ReadOnly);

    let lease2 = service
        .acquire(run2, LeaseMode::ReadOnly, None)
        .expect("second read-only lease");
    assert_eq!(lease2.mode, LeaseMode::ReadOnly);

    let info1 = service.get(lease1.lease_id.clone()).unwrap();
    assert_eq!(info1.run_id, run1);
    let info2 = service.get(lease2.lease_id.clone()).unwrap();
    assert_eq!(info2.run_id, run2);

    service.release(lease1.lease_id).unwrap();
    service.release(lease2.lease_id).unwrap();
}

#[test]
fn write_lease_excludes_all_others() {
    let service = LeaseService::open_in_memory(test_project_id()).unwrap();
    let run1 = test_run_id(1);
    let run2 = test_run_id(2);

    let lease1 = service
        .acquire(run1, LeaseMode::Write, None)
        .expect("first write lease");
    assert_eq!(lease1.mode, LeaseMode::Write);

    let result = service.acquire(run2, LeaseMode::Write, None);
    assert!(
        result.is_err(),
        "second write lease for same project must fail"
    );

    service.release(lease1.lease_id).unwrap();
    let lease2 = service
        .acquire(run2, LeaseMode::Write, None)
        .expect("write lease after first released");
    assert_eq!(lease2.mode, LeaseMode::Write);
}

#[test]
fn write_lease_blocks_readonly() {
    let service = LeaseService::open_in_memory(test_project_id()).unwrap();
    let run1 = test_run_id(1);
    let run2 = test_run_id(2);

    let lease1 = service
        .acquire(run1, LeaseMode::Write, None)
        .expect("write lease");

    let result = service.acquire(run2, LeaseMode::ReadOnly, None);
    assert!(
        result.is_err(),
        "read-only lease must fail when write lease exists"
    );

    service.release(lease1.lease_id).unwrap();
}

#[test]
fn readonly_lease_blocks_write() {
    let service = LeaseService::open_in_memory(test_project_id()).unwrap();
    let run1 = test_run_id(1);
    let run2 = test_run_id(2);

    let _lease1 = service.acquire(run1, LeaseMode::ReadOnly, None).unwrap();

    let result = service.acquire(run2, LeaseMode::Write, None);
    assert!(
        result.is_err(),
        "write lease must fail when read-only lease exists"
    );
}

#[test]
fn released_lease_cannot_be_reused() {
    let service = LeaseService::open_in_memory(test_project_id()).unwrap();
    let run1 = test_run_id(1);
    let run2 = test_run_id(2);

    let lease1 = service
        .acquire(run1, LeaseMode::Write, None)
        .expect("write lease");
    let lease1_id = lease1.lease_id.clone();

    service.release(lease1_id.clone()).unwrap();

    let result = service.acquire(run2, LeaseMode::Write, None);
    assert!(result.is_ok(), "new acquire should succeed");
    let new_lease = result.unwrap();
    assert_ne!(new_lease.lease_id, lease1_id);
}

#[test]
fn active_for_repository_returns_active_count() {
    let service = LeaseService::open_in_memory(test_project_id()).unwrap();
    let run1 = test_run_id(1);
    let run2 = test_run_id(2);

    assert_eq!(service.active_for_repository().unwrap(), 0);

    let lease1 = service.acquire(run1, LeaseMode::ReadOnly, None).unwrap();
    assert_eq!(service.active_for_repository().unwrap(), 1);

    let _lease2 = service.acquire(run2, LeaseMode::ReadOnly, None).unwrap();
    assert_eq!(service.active_for_repository().unwrap(), 2);

    service.release(lease1.lease_id).unwrap();
    assert_eq!(service.active_for_repository().unwrap(), 1);
}
