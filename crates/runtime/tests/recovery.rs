//! Integration tests for crash recovery.
//!
//! Exercises the real [`RecoveryCoordinator`] against a real, migrated
//! [`DatabaseHandle`] (never a hand-rolled schema): seeds a task/worker/run
//! through the real `DomainRepository` API via `run_domain_op`, drives each
//! run into the non-terminal state under test, then calls `recover()` and
//! asserts the resulting terminal state.
//!
//! Every "stuck" run in these tests is made stale by using a
//! near-zero `stuck_threshold` plus a short sleep, rather than by hand-
//! crafting backdated timestamps -- this exercises the exact same
//! last-activity computation (`MAX(events.timestamp)` per run, falling back
//! to `runs.created_at`) that a real crashed daemon's stuck runs would hit.
//!
//! Tests run with `--test-threads=1` since they manipulate real database
//! state through the same actor a concurrent test's `DatabaseHandle` would
//! also spawn a thread for; keeping DB files per-test (via `TempDir`)
//! already isolates them, but the crate-wide convention is one thread.

use std::sync::Arc;
use std::time::Duration;

use batman_protocol::{
    ProjectId, Run, RunFlags, RunState, TaskId, TaskRef, Timestamp, WorkerId, WorkerProfileRef,
};
use batman_runtime::db::DatabaseHandle;
use batman_runtime::domain::DomainRepository;
use batman_runtime::recovery::{RecoveryConfig, RecoveryCoordinator};
use tempfile::TempDir;

/// Seeds one task + one worker + one run in `initial_state` against a real,
/// migrated database, and returns the run's identifiers for the caller to
/// drive further.
async fn seed_run(
    db: &DatabaseHandle,
    project_id: ProjectId,
    initial_state: &str,
) -> (TaskId, WorkerId, batman_protocol::RunId) {
    let task_id = TaskId::new();
    let worker_id = WorkerId::new();
    let run_id = batman_protocol::RunId::new();

    db.run_domain_op(Box::new(move |conn| {
        let mut repo = DomainRepository::new(conn, project_id);
        repo.upsert_task(
            task_id,
            &TaskRef {
                owner_client_instance_id: "omp-1".into(),
                revision: 1,
            },
        )?;
        let worker = batman_protocol::Worker {
            worker_id,
            profile_ref: WorkerProfileRef {
                id: worker_id,
                fingerprint: "sha256:fake".into(),
                adapter: "fake".into(),
                model: "test".into(),
                permission_envelope: serde_json::json!({}),
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
        Ok(serde_json::json!({}))
    }))
    .await
    .expect("seed run");

    if initial_state != "queued" {
        drive_to_state(db, project_id, run_id, initial_state).await;
    }

    (task_id, worker_id, run_id)
}

/// Walks `run_id` through the legal edges from `queued` up to `target`.
async fn drive_to_state(
    db: &DatabaseHandle,
    project_id: ProjectId,
    run_id: batman_protocol::RunId,
    target: &str,
) {
    let path: &[&str] = match target {
        "starting" => &["starting"],
        "working" => &["starting", "working"],
        "waitingUser" => &["starting", "working", "waitingUser"],
        "waitingPeer" => &["starting", "working", "waitingPeer"],
        "paused" => &["starting", "working", "paused"],
        other => panic!("no drive path defined for {other}"),
    };
    for state in path {
        let to = RunState::try_from(*state).expect("valid state");
        db.run_domain_op(Box::new(move |conn| {
            let mut repo = DomainRepository::new(conn, project_id);
            repo.transition_run(run_id, &to)
                .map(|_| serde_json::json!({}))
        }))
        .await
        .unwrap_or_else(|e| panic!("drive to {state} failed: {e}"));
    }
}

/// Reads a run's current projected state directly, for assertions.
async fn run_state(db: &DatabaseHandle, run_id: batman_protocol::RunId) -> String {
    db.run_domain_op(Box::new(move |conn| {
        let state: String = conn.query_row(
            "SELECT state FROM runs WHERE run_id = ?1",
            [run_id.to_string()],
            |r| r.get(0),
        )?;
        Ok(serde_json::json!(state))
    }))
    .await
    .expect("read run state")
    .as_str()
    .expect("state is a string")
    .to_string()
}

/// A `RecoveryConfig` with a near-zero threshold, so any run seeded before
/// `RECOVERY_SETTLE` elapses is provably "stuck" without hand-crafted
/// timestamps.
fn immediate_config(recover_paused: bool, recover_waiting: bool) -> RecoveryConfig {
    RecoveryConfig {
        stuck_threshold: Duration::from_millis(1),
        recover_paused,
        recover_waiting,
    }
}

/// How long every test sleeps after seeding, so its runs' last-activity
/// timestamps provably predate `immediate_config`'s 1ms threshold.
const RECOVERY_SETTLE: Duration = Duration::from_millis(50);

async fn open_db() -> (TempDir, DatabaseHandle) {
    let state_dir = TempDir::new().unwrap();
    let db_path = state_dir.path().join("runtime.db");
    let db = DatabaseHandle::start(db_path).await.unwrap();
    (state_dir, db)
}

#[tokio::test]
async fn recovery_returns_empty_when_no_stuck_runs() {
    let (_state_dir, db) = open_db().await;
    let project_id = ProjectId::new();
    let coordinator = RecoveryCoordinator::with_defaults(Arc::new(db), project_id);
    let result = coordinator.recover().await.unwrap();

    assert_eq!(result.recovered_count, 0);
    assert!(result.recovered_runs.is_empty());
}

#[tokio::test]
async fn recovery_config_default_values() {
    let config = RecoveryConfig::default();
    assert_eq!(config.stuck_threshold, Duration::from_secs(300));
    assert!(!config.recover_paused);
    assert!(!config.recover_waiting);
}

#[tokio::test]
async fn recovery_config_custom_values() {
    let config = RecoveryConfig {
        stuck_threshold: Duration::from_secs(600),
        recover_paused: true,
        recover_waiting: true,
    };
    assert_eq!(config.stuck_threshold, Duration::from_secs(600));
    assert!(config.recover_paused);
    assert!(config.recover_waiting);
}

// --------------------------------------------------------- kill-point tests

/// Kill-point: intent recorded (`queued`) but never started -- no evidence
/// the vendor process was ever spawned. Recovers to `failed`.
#[tokio::test]
async fn stuck_queued_run_recovers_to_failed() {
    let (_state_dir, db) = open_db().await;
    let project_id = ProjectId::new();
    let (_task_id, _worker_id, run_id) = seed_run(&db, project_id, "queued").await;
    tokio::time::sleep(RECOVERY_SETTLE).await;

    let db = Arc::new(db);
    let coordinator =
        RecoveryCoordinator::new(Arc::clone(&db), project_id, immediate_config(false, false));
    let result = coordinator.recover().await.unwrap();

    assert_eq!(result.recovered_count, 1);
    assert!(result.recovered_runs[0].success);
    assert_eq!(run_state(&db, run_id).await, "failed");
}

/// Kill-point: identity allocation in progress (`starting`) when the
/// process died -- the vendor child may or may not have spawned; without
/// process/PID evidence this sweep cannot tell, so it recovers to `failed`
/// (the invariant this sweep guarantees is "no false success/`succeeded`",
/// not "no false negative on a possibly-still-running process" -- that is
/// `RecoveryCoordinator`'s own PID/executable verification, out of this
/// module's scope per the Hardening plan's kill-point matrix).
#[tokio::test]
async fn stuck_starting_run_recovers_to_failed() {
    let (_state_dir, db) = open_db().await;
    let project_id = ProjectId::new();
    let (_task_id, _worker_id, run_id) = seed_run(&db, project_id, "starting").await;
    tokio::time::sleep(RECOVERY_SETTLE).await;

    let db = Arc::new(db);
    let coordinator =
        RecoveryCoordinator::new(Arc::clone(&db), project_id, immediate_config(false, false));
    let result = coordinator.recover().await.unwrap();

    assert_eq!(result.recovered_count, 1);
    assert_eq!(run_state(&db, run_id).await, "failed");
}

/// Kill-point: mid-run (`working`, covers child spawn and vendor
/// acknowledgement -- both project onto this one state in the current
/// schema) when the process died. Recovers to `failed`.
#[tokio::test]
async fn stuck_working_run_recovers_to_failed() {
    let (_state_dir, db) = open_db().await;
    let project_id = ProjectId::new();
    let (_task_id, _worker_id, run_id) = seed_run(&db, project_id, "working").await;
    tokio::time::sleep(RECOVERY_SETTLE).await;

    let db = Arc::new(db);
    let coordinator =
        RecoveryCoordinator::new(Arc::clone(&db), project_id, immediate_config(false, false));
    let result = coordinator.recover().await.unwrap();

    assert_eq!(result.recovered_count, 1);
    assert_eq!(run_state(&db, run_id).await, "failed");
}

/// Kill-point: waiting on a peer worker's acknowledgement (`waitingPeer`)
/// when the process died. With `recover_waiting: true`, recovers to
/// `cancelled` -- never `failed`, since the run was legitimately paused on
/// external input, not evidence of a failure.
#[tokio::test]
async fn stuck_waiting_peer_run_recovers_to_cancelled_when_opted_in() {
    let (_state_dir, db) = open_db().await;
    let project_id = ProjectId::new();
    let (_task_id, _worker_id, run_id) = seed_run(&db, project_id, "waitingPeer").await;
    tokio::time::sleep(RECOVERY_SETTLE).await;

    let db = Arc::new(db);
    let coordinator =
        RecoveryCoordinator::new(Arc::clone(&db), project_id, immediate_config(false, true));
    let result = coordinator.recover().await.unwrap();

    assert_eq!(result.recovered_count, 1);
    assert_eq!(run_state(&db, run_id).await, "cancelled");
}

/// Kill-point: event append pending, surfaced here as waiting on user
/// approval (`waitingUser`) when the process died. With `recover_waiting:
/// false` (the default), the run is left untouched -- recovering it would
/// silently cancel work a human may still be about to approve.
#[tokio::test]
async fn stuck_waiting_user_run_is_untouched_when_not_opted_in() {
    let (_state_dir, db) = open_db().await;
    let project_id = ProjectId::new();
    let (_task_id, _worker_id, run_id) = seed_run(&db, project_id, "waitingUser").await;
    tokio::time::sleep(RECOVERY_SETTLE).await;

    let db = Arc::new(db);
    let coordinator =
        RecoveryCoordinator::new(Arc::clone(&db), project_id, immediate_config(false, false));
    let result = coordinator.recover().await.unwrap();

    assert_eq!(
        result.recovered_count, 0,
        "waitingUser must stay untouched by default"
    );
    assert_eq!(run_state(&db, run_id).await, "waitingUser");
}

/// Kill-point: projection update pending, surfaced here as `paused` when
/// the process died. With `recover_paused: true`, recovers to `cancelled`.
#[tokio::test]
async fn stuck_paused_run_recovers_to_cancelled_when_opted_in() {
    let (_state_dir, db) = open_db().await;
    let project_id = ProjectId::new();
    let (_task_id, _worker_id, run_id) = seed_run(&db, project_id, "paused").await;
    tokio::time::sleep(RECOVERY_SETTLE).await;

    let db = Arc::new(db);
    let coordinator =
        RecoveryCoordinator::new(Arc::clone(&db), project_id, immediate_config(true, false));
    let result = coordinator.recover().await.unwrap();

    assert_eq!(result.recovered_count, 1);
    assert_eq!(run_state(&db, run_id).await, "cancelled");
}

/// A `paused` run is protected (never recovered) unless `recover_paused`
/// explicitly opts in -- the same invariant as `waitingUser`/`waitingPeer`,
/// proven separately since `paused` is reachable from `working` alone
/// (unlike the waiting states) and has its own config flag.
#[tokio::test]
async fn stuck_paused_run_is_untouched_when_not_opted_in() {
    let (_state_dir, db) = open_db().await;
    let project_id = ProjectId::new();
    let (_task_id, _worker_id, run_id) = seed_run(&db, project_id, "paused").await;
    tokio::time::sleep(RECOVERY_SETTLE).await;

    let db = Arc::new(db);
    let coordinator =
        RecoveryCoordinator::new(Arc::clone(&db), project_id, immediate_config(false, false));
    let result = coordinator.recover().await.unwrap();

    assert_eq!(result.recovered_count, 0);
    assert_eq!(run_state(&db, run_id).await, "paused");
}

/// A run whose last activity is recent (well inside the stuck threshold)
/// is never recovered, even in a non-terminal state -- recovery must not
/// cancel/fail work that is merely in progress, only work that has gone
/// silent for longer than the threshold.
#[tokio::test]
async fn fresh_non_terminal_run_is_not_recovered() {
    let (_state_dir, db) = open_db().await;
    let project_id = ProjectId::new();
    let (_task_id, _worker_id, run_id) = seed_run(&db, project_id, "working").await;
    // No sleep: last activity is "now", well inside the default 5-minute
    // threshold.

    let db = Arc::new(db);
    let coordinator = RecoveryCoordinator::with_defaults(Arc::clone(&db), project_id);
    let result = coordinator.recover().await.unwrap();

    assert_eq!(result.recovered_count, 0);
    assert_eq!(run_state(&db, run_id).await, "working");
}

/// A run already in a terminal state is never touched by recovery, however
/// old its last activity -- it has no outgoing edges and recovery must
/// never attempt (and fail) a transition out of one.
#[tokio::test]
async fn terminal_run_is_never_touched_regardless_of_age() {
    let (_state_dir, db) = open_db().await;
    let project_id = ProjectId::new();
    let (_task_id, _worker_id, run_id) = seed_run(&db, project_id, "working").await;
    let failed = RunState::try_from("failed").unwrap();
    db.run_domain_op(Box::new(move |conn| {
        let mut repo = DomainRepository::new(conn, project_id);
        repo.transition_run(run_id, &failed)
            .map(|_| serde_json::json!({}))
    }))
    .await
    .unwrap();
    tokio::time::sleep(RECOVERY_SETTLE).await;

    let db = Arc::new(db);
    let coordinator =
        RecoveryCoordinator::new(Arc::clone(&db), project_id, immediate_config(true, true));
    let result = coordinator.recover().await.unwrap();

    assert_eq!(result.recovered_count, 0);
    assert_eq!(run_state(&db, run_id).await, "failed");
}

/// Multiple independently-stuck runs are each recovered in one sweep, to
/// their own state-appropriate targets.
#[tokio::test]
async fn multiple_stuck_runs_are_all_recovered_independently() {
    let (_state_dir, db) = open_db().await;
    let project_id = ProjectId::new();
    let (_t1, _w1, queued_run) = seed_run(&db, project_id, "queued").await;
    let (_t2, _w2, working_run) = seed_run(&db, project_id, "working").await;
    let (_t3, _w3, paused_run) = seed_run(&db, project_id, "paused").await;
    tokio::time::sleep(RECOVERY_SETTLE).await;

    let db = Arc::new(db);
    let coordinator =
        RecoveryCoordinator::new(Arc::clone(&db), project_id, immediate_config(true, true));
    let result = coordinator.recover().await.unwrap();

    assert_eq!(result.recovered_count, 3);
    assert_eq!(run_state(&db, queued_run).await, "failed");
    assert_eq!(run_state(&db, working_run).await, "failed");
    assert_eq!(run_state(&db, paused_run).await, "cancelled");
}
