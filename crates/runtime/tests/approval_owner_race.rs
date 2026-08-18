//! Regression tests for R71: `ApprovalService::decide`'s caller-side
//! ownership pre-check races `reconcile/omp`'s task ownership rebind, and
//! `decide_approval`'s guarded write never re-checks ownership, so a stale
//! owner that passed the pre-check can still win the write.
//!
//! See `approval_decide_race.rs`'s header for the full actor-FIFO argument;
//! summarized here only as far as this file's construction needs it:
//! `DatabaseHandle::run_domain_op` sends whole boxed closures to a
//! single-owner thread over a FIFO channel, one `oneshot` reply per
//! command. The actor never interleaves the *inside* of two closures, only
//! whole closures with each other, in the order their commands were
//! enqueued.
//!
//! `ApprovalService::decide` is *two* round trips: `load_snapshot` (which
//! reads `tasks.owner_client_instance_id` for the caller-side pre-check),
//! then, once the pre-check and `humanRequired` check pass synchronously,
//! `decide_approval` (the guarded write). `DomainRepository::reconcile_ownership`
//! -- the same method `reconcile/omp` calls -- is *one* round trip: an
//! unguarded `UPDATE tasks SET owner_client_instance_id = ...`.
//!
//! The first test below drives `svc.decide` (as the *original* owner) and
//! a direct call to `reconcile_ownership` (rebinding to a *new* owner)
//! through `tokio::join!(biased; ...)`, `decide` declared first. On the
//! first poll of the combined future, `biased` polls `decide` before the
//! rebind, so `decide`'s `load_snapshot` command is enqueued in the actor's
//! channel *before* the rebind's `UPDATE` command -- the actor's FIFO order
//! processes `load_snapshot` first, so the pre-check reads the original
//! owner and passes. `decide`'s second command (`decide_approval`) does
//! not exist yet at this point: it is only sent after `load_snapshot`'s
//! reply arrives and wakes `decide`'s future for another poll, which can
//! only happen after the actor has already dequeued (and is processing or
//! has processed) the rebind's command, since the rebind's `send()` onto
//! the channel happened synchronously in the very first poll, strictly
//! before `decide`'s second `send()` could occur. So the actor's FIFO
//! order is guaranteed to be: `load_snapshot`, rebind `UPDATE`,
//! `decide_approval` -- the rebind always commits between the pre-check's
//! read and the guarded write, deterministically, with no dependence on
//! thread-scheduling timing. This is the same enqueue-order argument
//! `approval_decide_race.rs` uses to make "the first-declared call always
//! reaches the guarded write before the second" true of two `decide` calls;
//! here it makes "the rebind always lands between the first-declared
//! `decide`'s two round trips" true instead, because the rebind is only one
//! round trip and `decide`'s second round trip cannot be sent any earlier
//! than described above.
//!
//! Today `decide_approval` never re-reads task ownership, so this test
//! currently observes the stale owner's decision being *accepted* -- that
//! is the bug R71 describes, and this test is written to assert the fixed
//! contract (refused, nothing journaled, nothing recorded) so it fails RED
//! against the current code for that reason.
//!
//! The second test proves the fix (once it lands) must not over-reject:
//! a rebind followed, sequentially, by a decide from the *new* owner must
//! still succeed, with exactly one `ApprovalDecided` event.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use batman_protocol::{
    ApprovalId, ApprovalRequest, DecidedBy, ProjectId, Run, RunFlags, RunId, RunState, TaskId,
    TaskRef, Timestamp, Worker, WorkerId, WorkerProfileRef,
};
use batman_runtime::approval::{
    ApprovalCallback, ApprovalError, ApprovalService, CallbackFuture, DecideOutcome,
};
use batman_runtime::db::DatabaseHandle;
use batman_runtime::domain::DomainRepository;
use serde_json::json;
use tempfile::TempDir;
use tokio::sync::broadcast;

async fn open_db() -> (TempDir, DatabaseHandle) {
    let state_dir = TempDir::new().unwrap();
    let db_path = state_dir.path().join("runtime.db");
    let db = DatabaseHandle::start(db_path).await.unwrap();
    (state_dir, db)
}

/// An [`ApprovalCallback`] that counts invocations and always succeeds, for
/// asserting that a refused decision never reaches the adapter.
struct CountingCallback {
    calls: Arc<AtomicU32>,
}

impl ApprovalCallback for CountingCallback {
    fn acknowledge(&self, _approval_id: ApprovalId, _decision: &str) -> CallbackFuture<'static> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(()) })
    }
}

/// Seeds one task (owned by `"omp-1"`)/worker/run, drives the run to
/// `working`, and creates one pending, human-required approval against it.
/// Returns the ids a test needs to decide, rebind, and probe.
async fn seed_pending_approval(
    db: &DatabaseHandle,
    project_id: ProjectId,
) -> (ApprovalId, RunId, TaskId) {
    let task_id = TaskId::new();
    let worker_id = WorkerId::new();
    let run_id = RunId::new();
    let approval_id = ApprovalId::new();

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

    let request = ApprovalRequest {
        approval_id,
        run_id,
        task_id,
        action: "write file".into(),
        arguments: json!({ "path": "/tmp/x" }),
        human_required: true,
        policy_reason: "write requires human approval".into(),
        created_at: Timestamp::now(),
        decided_at: None,
        decision: None,
    };
    db.run_domain_op(Box::new(move |conn| {
        let mut repo = DomainRepository::new(conn, project_id);
        repo.create_approval(&request).map(|_| json!({}))
    }))
    .await
    .expect("create the approval");

    (approval_id, run_id, task_id)
}

/// An [`ApprovalService`] wired to the given callback; the broadcast sender
/// has no subscribers, which is the production shape for an unattached
/// console.
fn service(
    db: Arc<DatabaseHandle>,
    project_id: ProjectId,
    callback: Arc<dyn ApprovalCallback>,
) -> ApprovalService {
    ApprovalService::new(db, project_id, callback, broadcast::channel(64).0)
}

/// A single `run_domain_op` round trip calling
/// [`DomainRepository::reconcile_ownership`] -- the same repo method
/// `reconcile/omp` calls (`service/orchestration.rs::reconcile_omp`) --
/// directly, bypassing the RPC layer's own revision-match check, which is
/// irrelevant to the race under test.
async fn rebind_owner(
    db: &DatabaseHandle,
    project_id: ProjectId,
    task_id: TaskId,
    new_owner: &str,
    revision: u64,
) {
    let new_owner = new_owner.to_string();
    db.run_domain_op(Box::new(move |conn| {
        let mut repo = DomainRepository::new(conn, project_id);
        repo.reconcile_ownership(task_id, &new_owner, revision)
            .map(|_| json!({}))
    }))
    .await
    .expect("rebind task ownership");
}

async fn approval_decision(db: &DatabaseHandle, approval_id: ApprovalId) -> Option<String> {
    db.run_domain_op(Box::new(move |conn| {
        let decision: Option<String> = conn.query_row(
            "SELECT decision FROM approvals WHERE approval_id = ?1",
            [approval_id.to_string()],
            |row| row.get(0),
        )?;
        Ok(json!(decision))
    }))
    .await
    .expect("read approval decision")
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

async fn decided_event_count(db: &DatabaseHandle) -> i64 {
    db.run_domain_op(Box::new(|conn| {
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM events WHERE event_json LIKE '%approvalDecided%'",
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
async fn a_stale_owner_that_passed_the_pre_check_is_refused_by_the_guarded_write() {
    let (_state_dir, db) = open_db().await;
    let db = Arc::new(db);
    let project_id = ProjectId::new();
    let (approval_id, run_id, task_id) = seed_pending_approval(&db, project_id).await;
    let calls = Arc::new(AtomicU32::new(0));
    let svc = service(
        Arc::clone(&db),
        project_id,
        Arc::new(CountingCallback {
            calls: Arc::clone(&calls),
        }),
    );

    // `decide` (as the original owner "omp-1") is declared first, so its
    // `load_snapshot` round trip is enqueued -- and thus processed -- before
    // the rebind's `UPDATE`, reading the pre-rebind owner and passing the
    // caller-side pre-check; the rebind then commits before `decide`'s
    // second round trip (`decide_approval`) can possibly be sent, per the
    // enqueue-order argument in this file's header. Deterministic, no
    // dependence on real timing.
    let (decide_result, _rebind) = tokio::join!(
        biased;
        svc.decide(approval_id, "omp-1", "approve", "ok", DecidedBy::Human),
        rebind_owner(&db, project_id, task_id, "omp-2", 2),
    );

    assert!(
        matches!(decide_result, Err(ApprovalError::Forbidden { .. })),
        "a stale owner must be refused once the guarded write observes the rebind, not accepted: {decide_result:?}"
    );
    assert_eq!(
        approval_decision(&db, approval_id).await,
        None,
        "a refused decision must never be recorded"
    );
    assert_eq!(
        decided_event_count(&db).await,
        0,
        "no approvalDecided event may survive a refused decision"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "a refused decision must never reach the adapter callback"
    );
    assert_eq!(
        run_state(&db, run_id).await,
        "waitingUser",
        "a refused decision must not move the run out of waitingUser"
    );
}

#[tokio::test]
async fn the_new_owner_can_decide_after_a_rebind() {
    let (_state_dir, db) = open_db().await;
    let db = Arc::new(db);
    let project_id = ProjectId::new();
    let (approval_id, run_id, task_id) = seed_pending_approval(&db, project_id).await;
    let calls = Arc::new(AtomicU32::new(0));
    let svc = service(
        Arc::clone(&db),
        project_id,
        Arc::new(CountingCallback {
            calls: Arc::clone(&calls),
        }),
    );

    // Rebind sequentially, fully committed before decide is even called --
    // zero timing dependency -- so this test guards the eventual fix
    // against over-rejection: a legitimate new owner must still be able to
    // decide after a rebind.
    rebind_owner(&db, project_id, task_id, "omp-2", 2).await;

    let outcome = svc
        .decide(approval_id, "omp-2", "approve", "ok", DecidedBy::Human)
        .await;

    assert!(
        matches!(outcome, Ok(DecideOutcome::Decided)),
        "the rebound owner must be able to decide: {outcome:?}"
    );
    assert_eq!(
        approval_decision(&db, approval_id).await,
        Some("approve".to_string())
    );
    assert_eq!(
        decided_event_count(&db).await,
        1,
        "exactly one approvalDecided event must be journaled"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "the deciding call must reach the adapter callback"
    );
    assert_eq!(run_state(&db, run_id).await, "working");
}
