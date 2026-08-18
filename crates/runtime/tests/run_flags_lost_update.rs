//! Regression test for R73: `ApprovalService::decide`'s callback-failure
//! path (`crates/runtime/src/approval/service.rs:250-268`) writes back the
//! *whole* `RunFlags` struct it read into `ApprovalSnapshot` before the
//! decision write and the vendor callback await. If anything else -- most
//! plausibly `ViolationService::apply_action`'s read-modify-write shape
//! (`crates/runtime/src/policy/violation.rs:346-413`, e.g.
//! `set_quarantined`) -- mutates a different flag on the same run during
//! that callback window, `decide`'s write-back silently reverts it: a lost
//! update, not a conflict either side detects.
//!
//! `FailingCallback::acknowledge` below plays the innocent case: it fails
//! without touching flags, so `decide`'s write-back is the *only* writer and
//! `protocolUnhealthy` lands correctly. `QuarantineDuringCallback::acknowledge`
//! plays the concurrent-mutation case: while `decide` awaits it, it performs
//! exactly the read-modify-write `ViolationService::set_quarantined` performs
//! (read the run's current flags through `DatabaseHandle`, flip
//! `policy_quarantined`, write back through `DomainRepository::set_run_flags`)
//! and then fails, so `decide`'s subsequent write-back of its stale
//! pre-callback snapshot has something to clobber.

use std::sync::Arc;

use batman_protocol::{
    ApprovalId, ApprovalRequest, DecidedBy, ProjectId, Run, RunFlags, RunId, RunState, TaskId,
    TaskRef, Timestamp, Worker, WorkerId, WorkerProfileRef,
};
use batman_runtime::approval::{
    ApprovalCallback, ApprovalService, CallbackFuture, DecideOutcome,
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

/// Seeds one task/worker/run, drives the run to `working`, and creates one
/// pending, human-required approval against it -- identical in shape to
/// `approval_decide_race.rs`'s helper of the same name.
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

/// Reads a run's current flags directly, independent of `ApprovalService`'s
/// own (potentially stale) `ApprovalSnapshot` -- this is the ground truth
/// each test asserts against.
async fn read_run_flags(db: &DatabaseHandle, run_id: RunId) -> RunFlags {
    let value = db
        .run_domain_op(Box::new(move |conn| {
            conn.query_row(
                "SELECT flags_degraded_control, flags_needs_reconciliation, flags_protocol_unhealthy,
                        flags_policy_quarantined, flags_workspace_dirty, flags_children_active
                 FROM runs WHERE run_id = ?1",
                [run_id.to_string()],
                |row| {
                    Ok(json!({
                        "degradedControl": row.get::<_, i64>(0)? != 0,
                        "needsReconciliation": row.get::<_, i64>(1)? != 0,
                        "protocolUnhealthy": row.get::<_, i64>(2)? != 0,
                        "policyQuarantined": row.get::<_, i64>(3)? != 0,
                        "workspaceDirty": row.get::<_, i64>(4)? != 0,
                        "childrenActive": row.get::<_, i64>(5)? != 0,
                    }))
                },
            )
            .map_err(Into::into)
        }))
        .await
        .expect("read run flags");

    RunFlags {
        degraded_control: value["degradedControl"].as_bool().unwrap_or(false),
        needs_reconciliation: value["needsReconciliation"].as_bool().unwrap_or(false),
        protocol_unhealthy: value["protocolUnhealthy"].as_bool().unwrap_or(false),
        policy_quarantined: value["policyQuarantined"].as_bool().unwrap_or(false),
        workspace_dirty: value["workspaceDirty"].as_bool().unwrap_or(false),
        children_active: value["childrenActive"].as_bool().unwrap_or(false),
    }
}

/// Fails every callback without touching the run at all -- the baseline
/// case where `decide`'s write-back of its pre-callback snapshot is the
/// only writer and therefore correct.
struct FailingCallback;

impl ApprovalCallback for FailingCallback {
    fn acknowledge(&self, _approval_id: ApprovalId, _decision: &str) -> CallbackFuture<'static> {
        Box::pin(async { Err("adapter unreachable".to_string()) })
    }
}

/// Fails every callback, but first performs exactly the read-modify-write
/// `ViolationService::set_quarantined` performs
/// (`crates/runtime/src/policy/violation.rs:389-413`): read the run's
/// current flags through `DatabaseHandle`, flip `policy_quarantined`, write
/// the whole struct back through `DomainRepository::set_run_flags`. This is
/// the same shape `ViolationService::apply_action` can run concurrently
/// with an in-flight `decide` -- both go through the same single-consumer
/// database actor, so this mutation is guaranteed to land, and to land
/// strictly between `decide`'s pre-callback snapshot read and its
/// post-callback-failure write-back, because it runs *inside* the callback
/// await `decide` is blocked on.
struct QuarantineDuringCallback {
    db: Arc<DatabaseHandle>,
    project_id: ProjectId,
    run_id: RunId,
}

impl ApprovalCallback for QuarantineDuringCallback {
    fn acknowledge(&self, _approval_id: ApprovalId, _decision: &str) -> CallbackFuture<'static> {
        let db = Arc::clone(&self.db);
        let project_id = self.project_id;
        let run_id = self.run_id;
        Box::pin(async move {
            let mut flags = read_run_flags(&db, run_id).await;
            flags.policy_quarantined = true;
            db.run_domain_op(Box::new(move |conn| {
                let mut repo = DomainRepository::new(conn, project_id);
                repo.set_run_flags(run_id, &flags).map(|_| json!({}))
            }))
            .await
            .expect("apply quarantine inside the callback window");

            Err("adapter unreachable".to_string())
        })
    }
}

#[tokio::test]
async fn a_flag_set_during_the_callback_window_survives_a_callback_failure() {
    let (_state_dir, db) = open_db().await;
    let db = Arc::new(db);
    let project_id = ProjectId::new();
    let (approval_id, run_id, _task_id) = seed_pending_approval(&db, project_id).await;

    let svc = service(
        Arc::clone(&db),
        project_id,
        Arc::new(QuarantineDuringCallback {
            db: Arc::clone(&db),
            project_id,
            run_id,
        }),
    );

    let outcome = svc
        .decide(approval_id, "omp-1", "approve", "ok", DecidedBy::Human)
        .await;

    assert!(
        matches!(outcome, Ok(DecideOutcome::DecidedCallbackFailed)),
        "a failing callback must still record the decision: {outcome:?}"
    );

    let flags = read_run_flags(&db, run_id).await;
    assert!(
        flags.policy_quarantined,
        "policy_quarantined was set by a concurrent writer inside the callback \
         window and must survive decide's callback-failure write-back, but it \
         was reverted to false: {flags:?}"
    );
    assert!(
        flags.protocol_unhealthy,
        "the callback failure must still mark the run protocol_unhealthy: {flags:?}"
    );
}

#[tokio::test]
async fn the_unhealthy_flag_is_applied_when_no_concurrent_mutation_happens() {
    let (_state_dir, db) = open_db().await;
    let db = Arc::new(db);
    let project_id = ProjectId::new();
    let (approval_id, run_id, _task_id) = seed_pending_approval(&db, project_id).await;

    let svc = service(Arc::clone(&db), project_id, Arc::new(FailingCallback));

    let outcome = svc
        .decide(approval_id, "omp-1", "approve", "ok", DecidedBy::Human)
        .await;

    assert!(
        matches!(outcome, Ok(DecideOutcome::DecidedCallbackFailed)),
        "a failing callback must still record the decision: {outcome:?}"
    );

    let flags = read_run_flags(&db, run_id).await;
    assert!(
        flags.protocol_unhealthy,
        "a failing callback must mark the run protocol_unhealthy: {flags:?}"
    );
    assert!(!flags.degraded_control, "no other flag should change: {flags:?}");
    assert!(!flags.needs_reconciliation, "no other flag should change: {flags:?}");
    assert!(!flags.policy_quarantined, "no other flag should change: {flags:?}");
    assert!(!flags.workspace_dirty, "no other flag should change: {flags:?}");
    assert!(!flags.children_active, "no other flag should change: {flags:?}");
}
