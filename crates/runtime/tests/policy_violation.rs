//! Regression tests for R54: `ViolationService::decide` must not admit two
//! concurrent, contradictory decisions for the same policy violation.
//!
//! `DatabaseHandle::run_domain_op` (`crates/runtime/src/db/actor.rs`) sends a
//! whole boxed closure to a single-owner `std::thread` over a *bounded*
//! `tokio::sync::mpsc` channel (capacity 32) and awaits a `oneshot` reply.
//! That actor thread is a strictly FIFO single consumer: it never
//! interleaves the *inside* of two closures, only whole closures with each
//! other. A service method that spans more than one `run_domain_op` round
//! trip is therefore not a transaction -- a decision made from the result
//! of an earlier round trip can be stale by the time a later one runs.
//!
//! These tests drive two `decide` calls through `tokio::join!` in a single
//! task, never `tokio::spawn`. That is what makes the interleave
//! deterministic here: `join!` polls its child futures in argument order on
//! every poll of the combined future, and each `run_domain_op` await parks
//! the polling task until the actor replies. So on the first poll, future A
//! enqueues its first command and parks *before* future B is polled at all
//! -- future B's first command is enqueued before A can possibly resume.
//! The channel's capacity (32) is never exhausted by the handful of
//! commands these tests issue, so every send completes synchronously; the
//! actor is the only consumer, so enqueue order is exactly processing
//! order. The result is a provable command sequence: A's round trip N is
//! always enqueued -- and thus processed -- before B's round trip N, for
//! every N, because A is always polled first on the poll that lets it
//! enqueue that round trip. `tokio::spawn` would give each `decide` call
//! its own task, and the executor is free to interleave two tasks in any
//! order; only single-task `join!` polling gives the deterministic
//! ordering these tests rely on.
//!
//! Each test verified this determinism empirically in addition to the
//! argument above: run 20x with `--exact` during development, no flakes
//! observed.

use std::sync::Arc;

use batman_protocol::{
    PolicyViolationId, ProjectId, Run, RunFlags, RunId, RunState, TaskId, TaskRef, Timestamp,
    Worker, WorkerId, WorkerProfileRef,
};
use batman_runtime::config::NestedViolationAction;
use batman_runtime::db::DatabaseHandle;
use batman_runtime::domain::DomainRepository;
use batman_runtime::policy::{DecideOutcome, ViolationError, ViolationService};
use serde_json::json;
use tempfile::TempDir;
use tokio::sync::broadcast;

async fn open_db() -> (TempDir, DatabaseHandle) {
    let state_dir = TempDir::new().unwrap();
    let db_path = state_dir.path().join("runtime.db");
    let db = DatabaseHandle::start(db_path).await.unwrap();
    (state_dir, db)
}

/// Seeds one task/worker/run, drives the run to `working`, quarantines it,
/// and records one unresolved nested-worker violation against it. Returns
/// the ids a test needs to decide and probe.
async fn seed_quarantined_violation(
    db: &DatabaseHandle,
    project_id: ProjectId,
) -> (PolicyViolationId, RunId, TaskId, WorkerId) {
    let task_id = TaskId::new();
    let worker_id = WorkerId::new();
    let run_id = RunId::new();

    db.run_domain_op(Box::new(move |conn| {
        let mut repo = DomainRepository::new(conn, project_id);
        repo.upsert_task(
            task_id,
            &TaskRef {
                owner_client_instance_id: "omp-1".into(),
                revision: 1,
            },
        )?;
        let worker = Worker {
            worker_id,
            profile_ref: WorkerProfileRef {
                id: worker_id,
                fingerprint: "sha256:fake".into(),
                adapter: "fake".into(),
                model: "test".into(),
                permission_envelope: json!({}),
            },
            parent_worker_id: None,
            created_at: Timestamp::now(),
        };
        repo.create_worker(&worker)?;
        let run = Run {
            run_id,
            task_id,
            worker_id,
            state: RunState::try_from("queued").expect("queued is a valid state"),
            flags: RunFlags::default(),
            vendor_session_id: None,
            started_at: None,
            completed_at: None,
        };
        repo.submit_run(&run, None)?;
        Ok(json!({}))
    }))
    .await
    .expect("seed task/worker/run");

    for state in ["starting", "working"] {
        let to = RunState::try_from(state).expect("valid state");
        db.run_domain_op(Box::new(move |conn| {
            let mut repo = DomainRepository::new(conn, project_id);
            repo.transition_run(run_id, &to).map(|_| json!({}))
        }))
        .await
        .unwrap_or_else(|e| panic!("drive to {state} failed: {e}"));
    }

    let flags = RunFlags {
        policy_quarantined: true,
        ..Default::default()
    };
    db.run_domain_op(Box::new(move |conn| {
        let mut repo = DomainRepository::new(conn, project_id);
        repo.set_run_flags(run_id, &flags).map(|_| json!({}))
    }))
    .await
    .expect("quarantine the run");

    let violation_id = PolicyViolationId::new();
    db.run_domain_op(Box::new(move |conn| {
        let mut repo = DomainRepository::new(conn, project_id);
        repo.record_policy_violation(
            violation_id,
            run_id,
            task_id,
            worker_id,
            "nested_worker_denied",
            7,
            "sha256:fp",
            Some("child-1"),
            Some("parent-1"),
            "quarantine",
        )
        .map(|_| json!({}))
    }))
    .await
    .expect("record the violation");

    (violation_id, run_id, task_id, worker_id)
}

/// A `ViolationService` with no adapter driver: the cancel path only
/// transitions the run, so the run's projected state is the observable
/// that proves whether the cancel side effect fired.
fn service(db: Arc<DatabaseHandle>, project_id: ProjectId) -> ViolationService {
    ViolationService::new(
        db,
        project_id,
        broadcast::channel(64).0,
        None,
        NestedViolationAction::Quarantine,
    )
}

/// A single `run_domain_op` transitioning `run_id` straight to `cancelled`
/// -- the same edge `ViolationService::cancel_and_transition` uses in
/// production -- to simulate the run settling out from under a concurrent
/// `decide("release")`.
async fn cancel_the_run(db: &DatabaseHandle, project_id: ProjectId, run_id: RunId) {
    let to = RunState::try_from("cancelled").expect("cancelled is a valid state");
    db.run_domain_op(Box::new(move |conn| {
        let mut repo = DomainRepository::new(conn, project_id);
        repo.transition_run(run_id, &to).map(|_| json!({}))
    }))
    .await
    .expect("cancel the run");
}

async fn violation_resolution(
    db: &DatabaseHandle,
    violation_id: PolicyViolationId,
) -> Option<String> {
    db.run_domain_op(Box::new(move |conn| {
        let resolution: Option<String> = conn.query_row(
            "SELECT resolution FROM policy_violations WHERE violation_id = ?1",
            [violation_id.to_string()],
            |row| row.get(0),
        )?;
        Ok(json!(resolution))
    }))
    .await
    .expect("read violation resolution")
    .as_str()
    .map(str::to_string)
}

async fn run_state(db: &DatabaseHandle, run_id: RunId) -> String {
    db.run_domain_op(Box::new(move |conn| {
        let state: String = conn.query_row(
            "SELECT state FROM runs WHERE run_id = ?1",
            [run_id.to_string()],
            |r| r.get(0),
        )?;
        Ok(json!(state))
    }))
    .await
    .expect("read run state")
    .as_str()
    .expect("state is a string")
    .to_string()
}

async fn run_quarantined(db: &DatabaseHandle, run_id: RunId) -> bool {
    db.run_domain_op(Box::new(move |conn| {
        let quarantined: i64 = conn.query_row(
            "SELECT flags_policy_quarantined FROM runs WHERE run_id = ?1",
            [run_id.to_string()],
            |r| r.get(0),
        )?;
        Ok(json!(quarantined))
    }))
    .await
    .expect("read quarantine flag")
    .as_i64()
    .expect("flag is an integer")
        != 0
}

async fn decided_event_count(db: &DatabaseHandle) -> i64 {
    db.run_domain_op(Box::new(|conn| {
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM events WHERE event_json LIKE '%policyViolationDecided%'",
            [],
            |r| r.get(0),
        )?;
        Ok(json!(count))
    }))
    .await
    .expect("count decided events")
    .as_i64()
    .expect("count is an integer")
}

#[tokio::test]
async fn concurrent_release_and_cancel_admit_exactly_one_decision() {
    let (_state_dir, db) = open_db().await;
    let db = Arc::new(db);
    let project_id = ProjectId::new();
    let (violation_id, run_id, ..) = seed_quarantined_violation(&db, project_id).await;
    let svc = service(Arc::clone(&db), project_id);

    let (release, cancel) = tokio::join!(
        svc.decide(violation_id, "omp-1", "release"),
        svc.decide(violation_id, "omp-1", "cancel"),
    );

    // Exactly one call decided the violation; the other was refused as a
    // conflicting concurrent decision.
    let decided = [
        matches!(release, Ok(DecideOutcome::Decided)),
        matches!(cancel, Ok(DecideOutcome::Decided)),
    ];
    assert_eq!(
        decided.iter().filter(|d| **d).count(),
        1,
        "exactly one of release/cancel must be the decision: release={release:?} cancel={cancel:?}"
    );
    let conflicted = [
        matches!(release, Err(ViolationError::Conflict { .. })),
        matches!(cancel, Err(ViolationError::Conflict { .. })),
    ];
    assert_eq!(
        conflicted.iter().filter(|c| **c).count(),
        1,
        "the losing call must see Conflict: release={release:?} cancel={cancel:?}"
    );
    assert_eq!(
        decided_event_count(&db).await,
        1,
        "exactly one PolicyViolationDecided event must be journaled, never two"
    );

    // Only the winner's side effect is visible -- derived from which call
    // actually won, so this assertion does not depend on join! argument
    // order (though the analysis above shows release always wins here).
    if release.is_ok() {
        assert_eq!(
            violation_resolution(&db, violation_id).await,
            Some("release".to_string())
        );
        assert_eq!(run_state(&db, run_id).await, "working");
        assert!(
            !run_quarantined(&db, run_id).await,
            "release must clear quarantine"
        );
    } else {
        assert_eq!(
            violation_resolution(&db, violation_id).await,
            Some("cancel".to_string())
        );
        assert_eq!(run_state(&db, run_id).await, "cancelled");
        assert!(
            run_quarantined(&db, run_id).await,
            "cancel must not touch quarantine"
        );
    }
}

#[tokio::test]
async fn concurrent_identical_releases_journal_one_event_and_report_already_decided() {
    let (_state_dir, db) = open_db().await;
    let db = Arc::new(db);
    let project_id = ProjectId::new();
    let (violation_id, run_id, ..) = seed_quarantined_violation(&db, project_id).await;
    let svc = service(Arc::clone(&db), project_id);

    let (first, second) = tokio::join!(
        svc.decide(violation_id, "omp-1", "release"),
        svc.decide(violation_id, "omp-1", "release"),
    );

    let outcomes = [
        first.expect("first release call must succeed"),
        second.expect("second release call must succeed"),
    ];
    assert_eq!(
        outcomes
            .iter()
            .filter(|o| **o == DecideOutcome::Decided)
            .count(),
        1,
        "exactly one call must be the new decision: {outcomes:?}"
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|o| **o == DecideOutcome::AlreadyDecided)
            .count(),
        1,
        "the other call must observe an idempotent replay: {outcomes:?}"
    );
    assert_eq!(
        decided_event_count(&db).await,
        1,
        "an idempotent replay must not journal a second event"
    );
    assert_eq!(
        violation_resolution(&db, violation_id).await,
        Some("release".to_string())
    );
    assert!(!run_quarantined(&db, run_id).await);
    assert_eq!(run_state(&db, run_id).await, "working");
}

#[tokio::test]
async fn deciding_the_same_resolution_twice_sequentially_stays_idempotent() {
    let (_state_dir, db) = open_db().await;
    let db = Arc::new(db);
    let project_id = ProjectId::new();
    let (violation_id, run_id, ..) = seed_quarantined_violation(&db, project_id).await;
    let svc = service(Arc::clone(&db), project_id);

    let first = svc.decide(violation_id, "omp-1", "release").await;
    let second = svc.decide(violation_id, "omp-1", "release").await;

    assert!(
        matches!(first, Ok(DecideOutcome::Decided)),
        "the first decide must be the new decision: {first:?}"
    );
    assert!(
        matches!(second, Ok(DecideOutcome::AlreadyDecided)),
        "the sequential replay must be idempotent, not an error: {second:?}"
    );
    assert_eq!(
        decided_event_count(&db).await,
        1,
        "a sequential replay must not journal a second event"
    );
    assert!(!run_quarantined(&db, run_id).await);
}

#[tokio::test]
async fn releasing_a_violation_whose_run_settles_mid_decide_is_refused() {
    let (_state_dir, db) = open_db().await;
    let db = Arc::new(db);
    let project_id = ProjectId::new();
    let (violation_id, run_id, ..) = seed_quarantined_violation(&db, project_id).await;
    let svc = service(Arc::clone(&db), project_id);

    let (release, ()) = tokio::join!(
        svc.decide(violation_id, "omp-1", "release"),
        cancel_the_run(&db, project_id, run_id),
    );

    assert!(
        matches!(release, Err(ViolationError::RunSettled { .. })),
        "a release racing a run settling to cancelled must be refused: {release:?}"
    );
    assert_eq!(
        violation_resolution(&db, violation_id).await,
        None,
        "the guard must roll back the UPDATE together with the appended event"
    );
    assert_eq!(
        decided_event_count(&db).await,
        0,
        "no PolicyViolationDecided event may survive a refused release"
    );
    assert_eq!(run_state(&db, run_id).await, "cancelled");
    assert!(
        run_quarantined(&db, run_id).await,
        "quarantine must still be set -- release was never applied"
    );
}
