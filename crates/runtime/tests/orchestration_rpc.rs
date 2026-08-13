//! Integration tests for the orchestration JSON-RPC methods.
//!
//! Drives the real [`batman_runtime::ipc::Server`] over a Unix domain
//! socket, exercising `task/upsert|get`, `worker/create|list|get`,
//! `run/submit|list|get|retry|cancel`, `message/send|list`,
//! `approval/list|decide`, and `reconcile/omp`.

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use batman_protocol::{RunId, TaskId, WorkerId};
use batman_runtime::adapter::{
    Adapter, AdapterEvent, AdapterEventPayload, AdapterEventSink, CancelScope, OmpRpcAdapter,
    OmpRpcAdapterOptions, OmpRpcStartupOptions, ProfileId, StartSpec, StartupOptions,
    WorkerProfile,
};
use batman_runtime::db::DatabaseHandle;
use batman_runtime::ipc::{PeerCredentialReader, PeerCredentials, Server, ServerConfig};
use batman_runtime::paths::RuntimePaths;
use batman_runtime::service::{AdapterFuture, FakeRunDriver, RunDriver, RunDriverContext};
use batman_runtime::workspace::ArtifactStore;
use nix::unistd::Uid;
use serde_json::{Value, json};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::net::unix::OwnedWriteHalf;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

// ------------------------------------------------------------------ fakes

struct FakeReader {
    uid: Option<u32>,
}

impl PeerCredentialReader for FakeReader {
    fn read(&self, _stream: &UnixStream) -> PeerCredentials {
        PeerCredentials {
            uid: self.uid,
            pid: Some(4242),
        }
    }
}

fn current_uid() -> u32 {
    Uid::current().as_raw()
}

fn matching_reader() -> Arc<dyn PeerCredentialReader> {
    Arc::new(FakeReader {
        uid: Some(current_uid()),
    })
}

/// A [`RunDriver`] that records every `send_follow_up` call verbatim and
/// always succeeds, so tests can assert exactly what a driver-backed run
/// received without needing a real (or fake-but-opinionated) adapter.
/// `start` never transitions run state -- these tests only exercise
/// `message/send`, not run lifecycle.
#[derive(Default)]
struct RecordingRunDriver {
    follow_ups: parking_lot::Mutex<Vec<(RunId, TaskId, WorkerId, String)>>,
}

impl RunDriver for RecordingRunDriver {
    fn start(&self, _ctx: RunDriverContext) -> AdapterFuture<'static, Result<(), String>> {
        Box::pin(async { Ok(()) })
    }

    fn send_follow_up(
        &self,
        run_id: RunId,
        task_id: TaskId,
        worker_id: WorkerId,
        prompt: String,
    ) -> AdapterFuture<'static, Result<(), String>> {
        self.follow_ups
            .lock()
            .push((run_id, task_id, worker_id, prompt));
        Box::pin(async { Ok(()) })
    }

    fn running_adapter(&self, _run_id: RunId) -> Option<Arc<dyn Adapter>> {
        None
    }

    fn cancel_run(
        &self,
        _run_id: RunId,
        _scope: CancelScope,
    ) -> AdapterFuture<'static, Result<(), String>> {
        Box::pin(async { Ok(()) })
    }
}
/// Tracks whether cancel_run was called and with what scope.
/// Used to verify item 33's wiring: OrchestrationService calls cancel_run on the live adapter.
#[derive(Default)]
struct CancelTrackingRunDriver {
    cancel_calls: parking_lot::Mutex<Vec<(RunId, CancelScope)>>,
    follow_ups: parking_lot::Mutex<Vec<(RunId, TaskId, WorkerId, String)>>,
}

impl CancelTrackingRunDriver {
    fn cancel_calls(&self) -> Vec<(RunId, CancelScope)> {
        self.cancel_calls.lock().clone()
    }
}

impl RunDriver for CancelTrackingRunDriver {
    fn start(&self, _ctx: RunDriverContext) -> AdapterFuture<'static, Result<(), String>> {
        Box::pin(async { Ok(()) })
    }

    fn send_follow_up(
        &self,
        run_id: RunId,
        task_id: TaskId,
        worker_id: WorkerId,
        prompt: String,
    ) -> AdapterFuture<'static, Result<(), String>> {
        self.follow_ups
            .lock()
            .push((run_id, task_id, worker_id, prompt));
        Box::pin(async { Ok(()) })
    }

    fn running_adapter(&self, _run_id: RunId) -> Option<Arc<dyn Adapter>> {
        None
    }

    fn cancel_run(
        &self,
        run_id: RunId,
        scope: CancelScope,
    ) -> AdapterFuture<'static, Result<(), String>> {
        self.cancel_calls.lock().push((run_id, scope));
        Box::pin(async { Ok(()) })
    }
}

#[tokio::test]
async fn run_cancel_calls_adapter_cancel_run_with_worker_scope() {
    let driver = Arc::new(CancelTrackingRunDriver::default());

    let harness = Harness::start(|c| {
        c.run_driver = Some(Arc::clone(&driver) as Arc<dyn RunDriver>);
    })
    .await;
    let mut client = omp_client(&harness, "omp-1").await;

    // Create a task, worker, and run
    let task = client
        .call(
            2,
            "task/upsert",
            json!({ "ownerClientInstanceId": "omp-1", "revision": 1 }),
        )
        .await;
    let task_id = task["result"]["taskId"].as_str().unwrap().to_string();

    let worker = client
        .call(
            3,
            "worker/create",
            json!({ "fingerprint": "sha256:f", "adapter": "fake", "model": "m" }),
        )
        .await;
    let worker_id = worker["result"]["workerId"].as_str().unwrap().to_string();

    let submit = client
        .call(
            4,
            "run/submit",
            json!({ "taskId": task_id, "workerId": worker_id }),
        )
        .await;
    assert!(
        submit.get("error").is_none(),
        "run/submit failed: {submit:?}"
    );

    let run_id = submit["result"]["runId"].as_str().unwrap().to_string();

    // Now cancel the run
    let cancel = client
        .call(5, "run/cancel", json!({ "runId": run_id }))
        .await;
    assert!(
        cancel.get("error").is_none(),
        "run/cancel failed: {cancel:?}"
    );

    // Verify cancel_run was called with CancelScope::Worker
    let calls = driver.cancel_calls();
    assert_eq!(calls.len(), 1, "expected exactly one cancel_run call");
    let (called_run_id, scope) = &calls[0];
    assert_eq!(
        called_run_id.to_string(),
        run_id,
        "cancel_run called with wrong run_id"
    );
    assert_eq!(
        *scope,
        CancelScope::Worker,
        "cancel_run called with wrong CancelScope"
    );
}

// ------------------------------------------------------- policy violation

/// Simulates what a real adapter/`AdapterRegistry` does when its
/// effective `nested` capability is not `Managed`: emits a
/// `NestedWorkerObserved` event through a real
/// [`batman_runtime::adapter::DomainAdapterEventSink`] constructed with
/// `nested_not_managed: true`, exercising the full
/// `ViolationService::record` pipeline (Hardening plan Task 1) without
/// spawning any vendor process. Captures its own `RunDriverContext` so a
/// test can trigger a *second* `NestedWorkerObserved` later (for the
/// idempotency case), reusing the same `db`/`events_tx`/`violation_service`
/// a real second observation on the same run would.
#[derive(Default)]
struct ViolationTriggeringRunDriver {
    cancel_calls: parking_lot::Mutex<Vec<(RunId, CancelScope)>>,
    captured: parking_lot::Mutex<Option<RunDriverContext>>,
}

impl ViolationTriggeringRunDriver {
    fn cancel_calls(&self) -> Vec<(RunId, CancelScope)> {
        self.cancel_calls.lock().clone()
    }

    async fn emit_nested_worker_observed(
        &self,
        vendor_child_id: &str,
        vendor_parent_ref: &str,
    ) -> Result<u64, String> {
        let ctx = self
            .captured
            .lock()
            .clone()
            .expect("start() must run before emit_nested_worker_observed");
        let sink = batman_runtime::adapter::DomainAdapterEventSink::new(
            ctx.db.clone(),
            ctx.project_id,
            ctx.events_tx.clone(),
            vec![],
            true,
            Arc::clone(&ctx.violation_service),
            None,
        );
        sink.emit(AdapterEvent {
            run_id: ctx.run_id,
            task_id: ctx.task_id,
            worker_id: ctx.worker_id,
            payload: AdapterEventPayload::NestedWorkerObserved {
                vendor_child_id: vendor_child_id.to_string(),
                vendor_parent_ref: vendor_parent_ref.to_string(),
            },
        })
        .await
        .map_err(|e| e.to_string())
    }
}

impl RunDriver for ViolationTriggeringRunDriver {
    fn start(&self, ctx: RunDriverContext) -> AdapterFuture<'static, Result<(), String>> {
        *self.captured.lock() = Some(ctx.clone());
        Box::pin(async move {
            let sink = batman_runtime::adapter::DomainAdapterEventSink::new(
                ctx.db.clone(),
                ctx.project_id,
                ctx.events_tx.clone(),
                vec![],
                true,
                Arc::clone(&ctx.violation_service),
                None,
            );
            sink.emit(AdapterEvent {
                run_id: ctx.run_id,
                task_id: ctx.task_id,
                worker_id: ctx.worker_id,
                payload: AdapterEventPayload::NestedWorkerObserved {
                    vendor_child_id: "child-vendor-1".to_string(),
                    vendor_parent_ref: "parent-vendor-1".to_string(),
                },
            })
            .await
            .map_err(|e| e.to_string())?;
            Ok(())
        })
    }

    fn send_follow_up(
        &self,
        _run_id: RunId,
        _task_id: TaskId,
        _worker_id: WorkerId,
        _prompt: String,
    ) -> AdapterFuture<'static, Result<(), String>> {
        Box::pin(async { Ok(()) })
    }

    fn running_adapter(&self, _run_id: RunId) -> Option<Arc<dyn Adapter>> {
        None
    }

    fn cancel_run(
        &self,
        run_id: RunId,
        scope: CancelScope,
    ) -> AdapterFuture<'static, Result<(), String>> {
        self.cancel_calls.lock().push((run_id, scope));
        Box::pin(async { Ok(()) })
    }
}

/// Submits a task/worker/run with `driver` as the injected `RunDriver`
/// (which emits the first `NestedWorkerObserved` from inside `start()`),
/// returning `(task_id, worker_id, run_id)`.
async fn submit_run_with_driver(client: &mut Client, owner: &str) -> (String, String, String) {
    let task = client
        .call(
            2,
            "task/upsert",
            json!({ "ownerClientInstanceId": owner, "revision": 1 }),
        )
        .await;
    let task_id = task["result"]["taskId"].as_str().unwrap().to_string();
    let worker = client
        .call(
            3,
            "worker/create",
            json!({ "fingerprint": "sha256:f", "adapter": "fake", "model": "m" }),
        )
        .await;
    let worker_id = worker["result"]["workerId"].as_str().unwrap().to_string();
    let submit = client
        .call(
            4,
            "run/submit",
            json!({ "taskId": task_id, "workerId": worker_id }),
        )
        .await;
    assert!(
        submit.get("error").is_none(),
        "run/submit failed: {submit:?}"
    );
    let run_id = submit["result"]["runId"].as_str().unwrap().to_string();
    (task_id, worker_id, run_id)
}

#[tokio::test]
async fn nested_worker_observed_quarantines_run_and_blocks_message_send_until_released() {
    let driver = Arc::new(ViolationTriggeringRunDriver::default());
    let harness = Harness::start(|c| {
        c.run_driver = Some(Arc::clone(&driver) as Arc<dyn RunDriver>);
        c.nested_violation_action = batman_runtime::config::NestedViolationAction::Quarantine;
    })
    .await;
    let mut client = omp_client(&harness, "omp-1").await;

    let (task_id, worker_id, run_id) = submit_run_with_driver(&mut client, "omp-1").await;

    // The pure-Quarantine action never cancels: the run stays non-terminal.
    let get = client.call(5, "run/get", json!({ "runId": run_id })).await;
    assert_eq!(
        get["result"]["flags"]["policyQuarantined"], true,
        "run must be quarantined after NestedWorkerObserved: {get:?}"
    );
    assert_ne!(
        get["result"]["state"], "cancelled",
        "Quarantine-only action must never cancel the run"
    );
    assert!(
        driver.cancel_calls().is_empty(),
        "Quarantine-only action must never call cancel_run"
    );

    // message/send is blocked while quarantined.
    let blocked_send = client
        .call(
            6,
            "message/send",
            json!({
                "runId": run_id,
                "senderWorkerId": worker_id,
                "taskId": task_id,
                "kind": "question",
                "payload": "should be blocked"
            }),
        )
        .await;
    assert_eq!(
        blocked_send["error"]["code"], -32101,
        "expected POLICY_QUARANTINED: {blocked_send:?}"
    );

    // Find the recorded violation's id from the replayed event.
    let replay = client
        .call(7, "events/replay", json!({ "afterSequence": 0 }))
        .await;
    let events = replay["result"].as_array().unwrap();
    let recorded = events
        .iter()
        .find(|e| e["event"]["type"] == "policyViolationRecorded")
        .expect("a policyViolationRecorded event must be journaled");
    let violation_id =
        recorded["event"]["payload"]["kind"]["policyViolationRecorded"]["violation_id"]
            .as_str()
            .expect("violation_id must be present on the recorded event")
            .to_string();

    // The owning client releases the quarantine.
    let decide = client
        .call(
            8,
            "policy/violation/decide",
            json!({ "violationId": violation_id, "resolution": "release" }),
        )
        .await;
    assert!(decide.get("error").is_none(), "decide failed: {decide:?}");
    assert_eq!(decide["result"]["outcome"], "decided");

    let get_after = client.call(9, "run/get", json!({ "runId": run_id })).await;
    assert_eq!(
        get_after["result"]["flags"]["policyQuarantined"], false,
        "release must clear the quarantine flag: {get_after:?}"
    );

    // message/send now succeeds.
    let unblocked_send = client
        .call(
            10,
            "message/send",
            json!({
                "runId": run_id,
                "senderWorkerId": worker_id,
                "taskId": task_id,
                "kind": "question",
                "payload": "should now succeed"
            }),
        )
        .await;
    assert!(
        unblocked_send.get("error").is_none(),
        "message/send must succeed once released: {unblocked_send:?}"
    );
}

#[tokio::test]
async fn policy_violation_decide_is_forbidden_for_a_non_owning_client() {
    let driver = Arc::new(ViolationTriggeringRunDriver::default());
    let harness = Harness::start(|c| {
        c.run_driver = Some(Arc::clone(&driver) as Arc<dyn RunDriver>);
        c.nested_violation_action = batman_runtime::config::NestedViolationAction::Quarantine;
    })
    .await;
    let mut owner_client = omp_client(&harness, "omp-owner").await;
    let (_, _, run_id) = submit_run_with_driver(&mut owner_client, "omp-owner").await;

    let replay = owner_client
        .call(5, "events/replay", json!({ "afterSequence": 0 }))
        .await;
    let events = replay["result"].as_array().unwrap();
    let recorded = events
        .iter()
        .find(|e| e["event"]["type"] == "policyViolationRecorded")
        .expect("a policyViolationRecorded event must be journaled");
    let violation_id =
        recorded["event"]["payload"]["kind"]["policyViolationRecorded"]["violation_id"]
            .as_str()
            .unwrap()
            .to_string();

    let mut other_client = omp_client(&harness, "omp-other").await;
    let decide = other_client
        .call(
            2,
            "policy/violation/decide",
            json!({ "violationId": violation_id, "resolution": "release" }),
        )
        .await;
    assert_eq!(
        decide["error"]["code"], -32602,
        "a non-owning client must be rejected: {decide:?}"
    );

    // The quarantine must remain untouched by the rejected attempt.
    let get = owner_client
        .call(6, "run/get", json!({ "runId": run_id }))
        .await;
    assert_eq!(get["result"]["flags"]["policyQuarantined"], true);
}

#[tokio::test]
async fn policy_violation_decide_release_is_refused_on_an_already_terminal_run() {
    let driver = Arc::new(ViolationTriggeringRunDriver::default());
    let harness = Harness::start(|c| {
        c.run_driver = Some(Arc::clone(&driver) as Arc<dyn RunDriver>);
        // Default action is QuarantineAndCancel: the violation itself
        // cancels the run.
    })
    .await;
    let mut client = omp_client(&harness, "omp-1").await;
    let (_, _, run_id) = submit_run_with_driver(&mut client, "omp-1").await;

    let get = client.call(5, "run/get", json!({ "runId": run_id })).await;
    assert_eq!(
        get["result"]["state"], "cancelled",
        "QuarantineAndCancel must cancel the run: {get:?}"
    );
    assert_eq!(
        driver.cancel_calls().len(),
        1,
        "cancel_run must be called exactly once"
    );

    let replay = client
        .call(6, "events/replay", json!({ "afterSequence": 0 }))
        .await;
    let events = replay["result"].as_array().unwrap();
    let recorded = events
        .iter()
        .find(|e| e["event"]["type"] == "policyViolationRecorded")
        .expect("a policyViolationRecorded event must be journaled");
    let violation_id =
        recorded["event"]["payload"]["kind"]["policyViolationRecorded"]["violation_id"]
            .as_str()
            .unwrap()
            .to_string();

    // Releasing quarantine on an already-terminal (cancelled) run must
    // never revive it.
    let decide = client
        .call(
            7,
            "policy/violation/decide",
            json!({ "violationId": violation_id, "resolution": "release" }),
        )
        .await;
    assert!(
        decide.get("error").is_some(),
        "releasing quarantine on a terminal run must be refused: {decide:?}"
    );

    let get_after = client.call(8, "run/get", json!({ "runId": run_id })).await;
    assert_eq!(
        get_after["result"]["state"], "cancelled",
        "the refused release must never revive the run"
    );
}
// ------------------------------------------------------------------ cost ceiling

/// Simulates a run whose adapter reports usage crossing the merged policy's
/// per-run cost ceiling: emits one `UsageReported` through a real
/// [`batman_runtime::adapter::DomainAdapterEventSink`] built with
/// `nested_not_managed: false` (so only the ceiling can trigger a violation)
/// and `cost_ceiling_per_run_usd: Some(1.0)`, exercising the whole
/// `ViolationService::record_cost_ceiling` pipeline with no vendor process.
#[derive(Default)]
struct CostCeilingRunDriver {
    cancel_calls: parking_lot::Mutex<Vec<(RunId, CancelScope)>>,
    captured: parking_lot::Mutex<Option<RunDriverContext>>,
}

impl CostCeilingRunDriver {
    fn cancel_calls(&self) -> Vec<(RunId, CancelScope)> {
        self.cancel_calls.lock().clone()
    }
}

impl RunDriver for CostCeilingRunDriver {
    fn start(&self, ctx: RunDriverContext) -> AdapterFuture<'static, Result<(), String>> {
        *self.captured.lock() = Some(ctx.clone());
        Box::pin(async move {
            let sink = batman_runtime::adapter::DomainAdapterEventSink::new(
                ctx.db.clone(),
                ctx.project_id,
                ctx.events_tx.clone(),
                vec![],
                false,
                Arc::clone(&ctx.violation_service),
                Some(1.0),
            );
            sink.emit(AdapterEvent {
                run_id: ctx.run_id,
                task_id: ctx.task_id,
                worker_id: ctx.worker_id,
                payload: AdapterEventPayload::UsageReported {
                    input_tokens: 1_000,
                    output_tokens: 2_000,
                    cost_usd: Some(2.5),
                },
            })
            .await
            .map_err(|e| e.to_string())?;
            Ok(())
        })
    }

    fn send_follow_up(
        &self,
        _run_id: RunId,
        _task_id: TaskId,
        _worker_id: WorkerId,
        _prompt: String,
    ) -> AdapterFuture<'static, Result<(), String>> {
        Box::pin(async { Ok(()) })
    }

    fn running_adapter(&self, _run_id: RunId) -> Option<Arc<dyn Adapter>> {
        None
    }

    fn cancel_run(
        &self,
        run_id: RunId,
        scope: CancelScope,
    ) -> AdapterFuture<'static, Result<(), String>> {
        self.cancel_calls.lock().push((run_id, scope));
        Box::pin(async { Ok(()) })
    }
}

/// A run whose adapter reports $2.50 against a $1.00 per-run ceiling
/// records a `cost_ceiling_exceeded` violation, quarantines the run,
/// and allows the owning client to release the quarantine via
/// `policy/violation/decide` — proving the full `record_cost_ceiling`
/// pipeline (R48).
#[tokio::test]
async fn crossing_the_per_run_cost_ceiling_records_an_actionable_violation() {
    let driver = Arc::new(CostCeilingRunDriver::default());
    let harness = Harness::start(|c| {
        c.run_driver = Some(Arc::clone(&driver) as Arc<dyn RunDriver>);
        c.nested_violation_action = batman_runtime::config::NestedViolationAction::Quarantine;
    })
    .await;
    let mut client = omp_client(&harness, "omp-1").await;

    let (_, _, run_id) = submit_run_with_driver(&mut client, "omp-1").await;

    // The pure-Quarantine action never cancels: the run stays non-terminal.
    let get = client.call(5, "run/get", json!({ "runId": run_id })).await;
    assert_eq!(
        get["result"]["flags"]["policyQuarantined"], true,
        "run must be quarantined after cost ceiling breach: {get:?}"
    );
    assert_ne!(
        get["result"]["state"], "cancelled",
        "Quarantine-only action must never cancel the run"
    );
    assert!(
        driver.cancel_calls().is_empty(),
        "Quarantine-only action must never call cancel_run"
    );

    // Find the recorded cost-ceiling violation from the replayed events.
    let replay = client
        .call(6, "events/replay", json!({ "afterSequence": 0 }))
        .await;
    let events = replay["result"].as_array().unwrap();
    let recorded = events
        .iter()
        .find(|e| e["event"]["type"] == "policyViolationRecorded")
        .expect("a policyViolationRecorded event must be journaled");
    let kind = &recorded["event"]["payload"]["kind"]["policyViolationRecorded"];
    assert_eq!(
        kind["code"], "cost_ceiling_exceeded",
        "the recorded violation must carry the cost ceiling code: {kind:?}"
    );
    assert!(
        kind["vendor_child_id"].is_null(),
        "a cost-ceiling violation has no vendor child: {kind:?}"
    );
    assert!(
        kind["vendor_parent_ref"].is_null(),
        "a cost-ceiling violation has no vendor parent: {kind:?}"
    );
    let violation_id = kind["violation_id"]
        .as_str()
        .expect("violation_id must be present")
        .to_string();

    // The owning client releases the quarantine — this step reads the
    // policy_violations projection row, so it proves the row was persisted.
    let decide = client
        .call(
            7,
            "policy/violation/decide",
            json!({ "violationId": violation_id, "resolution": "release" }),
        )
        .await;
    assert!(decide.get("error").is_none(), "decide failed: {decide:?}");
    assert_eq!(decide["result"]["outcome"], "decided");

    let get_after = client.call(8, "run/get", json!({ "runId": run_id })).await;
    assert_eq!(
        get_after["result"]["flags"]["policyQuarantined"], false,
        "release must clear the quarantine flag: {get_after:?}"
    );
}

/// A run's `policyFingerprint` is an immutable snapshot of the merge it
/// was authorized under: a later run carrying `policyOverrides` gets its
/// own fingerprint and never rewrites an existing run's.
#[tokio::test]
async fn per_run_policy_overrides_snapshot_only_their_own_run() {
    let org = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(org.path(), "max_workers: 8\n").unwrap();
    let layers = Arc::new(
        batman_runtime::config::LayeredConfig::load(Some(org.path()), None, None).unwrap(),
    );
    let startup = Arc::new(layers.merge(None).unwrap());
    let startup_fingerprint = startup.fingerprint.clone();

    let harness = Harness::start({
        let layers = Arc::clone(&layers);
        let startup = Arc::clone(&startup);
        move |c| {
            c.run_driver = Some(Arc::new(FakeRunDriver) as Arc<dyn RunDriver>);
            c.policy = Some((layers, startup));
        }
    })
    .await;
    let mut client = omp_client(&harness, "omp-1").await;

    let task = client
        .call(
            2,
            "task/upsert",
            json!({ "ownerClientInstanceId": "omp-1", "revision": 1 }),
        )
        .await;
    let task_id = task["result"]["taskId"].as_str().unwrap().to_string();
    let worker = client
        .call(
            3,
            "worker/create",
            json!({ "fingerprint": "sha256:f", "adapter": "fake", "model": "m" }),
        )
        .await;
    let worker_id = worker["result"]["workerId"].as_str().unwrap().to_string();

    let plain = client
        .call(
            4,
            "run/submit",
            json!({ "taskId": task_id, "workerId": worker_id }),
        )
        .await;
    let plain_run = plain["result"]["runId"].as_str().unwrap().to_string();

    let overridden = client
        .call(
            5,
            "run/submit",
            json!({
                "taskId": task_id,
                "workerId": worker_id,
                "policyOverrides": { "max_workers": 2 },
            }),
        )
        .await;
    assert!(
        overridden.get("error").is_none(),
        "an override that violates no lock must be accepted: {overridden:?}"
    );
    let overridden_run = overridden["result"]["runId"].as_str().unwrap().to_string();

    let plain_get = client
        .call(6, "run/get", json!({ "runId": plain_run }))
        .await;
    let overridden_get = client
        .call(7, "run/get", json!({ "runId": overridden_run }))
        .await;

    assert_eq!(
        plain_get["result"]["policyFingerprint"], startup_fingerprint,
        "a run without overrides is snapshotted under the startup merge"
    );
    let overridden_fingerprint = overridden_get["result"]["policyFingerprint"]
        .as_str()
        .expect("an overridden run must carry its own fingerprint");
    assert_ne!(
        overridden_fingerprint, startup_fingerprint,
        "an override must produce a distinct merge fingerprint"
    );

    // The snapshot is immutable: the second submit did not rewrite the
    // first run's row.
    let plain_again = client
        .call(8, "run/get", json!({ "runId": plain_run }))
        .await;
    assert_eq!(
        plain_again["result"]["policyFingerprint"], startup_fingerprint,
        "one run's overrides must never change another run's snapshot"
    );
}

/// A run whose display preference resolves to an available backend
/// journals exactly one `DisplayPaneAttached`, and reports the winning
/// backend on the submit response so the caller needs no second call.
#[tokio::test]
async fn run_submit_journals_the_display_pane_it_attached() {
    let harness = Harness::start(|c| {
        c.run_driver = Some(Arc::new(FakeRunDriver) as Arc<dyn RunDriver>);
    })
    .await;
    let mut client = omp_client(&harness, "omp-1").await;

    let task = client
        .call(
            2,
            "task/upsert",
            json!({ "ownerClientInstanceId": "omp-1", "revision": 1 }),
        )
        .await;
    let task_id = task["result"]["taskId"].as_str().unwrap().to_string();
    let worker = client
        .call(
            3,
            "worker/create",
            json!({ "fingerprint": "sha256:f", "adapter": "fake", "model": "m" }),
        )
        .await;
    let worker_id = worker["result"]["workerId"].as_str().unwrap().to_string();

    // `terminal` is the one backend that is always available, so this
    // resolves identically on a developer machine and in headless CI.
    let submit = client
        .call(
            4,
            "run/submit",
            json!({
                "taskId": task_id,
                "workerId": worker_id,
                "displayPreference": { "ordered": ["terminal"], "placement": "embedded" },
            }),
        )
        .await;
    let run_id = submit["result"]["runId"].as_str().unwrap().to_string();
    assert_eq!(submit["result"]["display"]["selected"], "terminal");
    assert_eq!(submit["result"]["display"]["attempts"], json!(["terminal"]));

    let replay = client
        .call(5, "events/replay", json!({ "afterSequence": 0 }))
        .await;
    let attached = pane_events(&replay, "displayPaneAttached");
    assert_eq!(attached.len(), 1, "exactly one attach: {attached:?}");
    assert_eq!(attached[0]["runId"], run_id);
    assert_eq!(attached[0]["backend"], "terminal");
    assert_eq!(
        attached[0]["paneRef"], "",
        "resolution never activates a backend, so there is no vendor pane id yet"
    );
}

/// Every `displayEvent` payload of `kind` in a replay response.
fn pane_events<'a>(replay: &'a Value, kind: &str) -> Vec<&'a Value> {
    replay["result"]
        .as_array()
        .expect("events/replay returns an array")
        .iter()
        .map(|e| &e["event"])
        .filter(|e| e["type"] == "displayEvent" && e["payload"]["kind"] == kind)
        .map(|e| &e["payload"])
        .collect()
}

/// An attach is journaled if and only if a backend was actually
/// selected. Whether `herdr` happens to be installed decides which branch
/// runs, so the test asserts the correspondence rather than the outcome.
#[tokio::test]
async fn a_pane_is_journaled_exactly_when_a_backend_was_selected() {
    let harness = Harness::start(|c| {
        c.run_driver = Some(Arc::new(FakeRunDriver) as Arc<dyn RunDriver>);
    })
    .await;
    let mut client = omp_client(&harness, "omp-1").await;

    let task = client
        .call(
            2,
            "task/upsert",
            json!({ "ownerClientInstanceId": "omp-1", "revision": 1 }),
        )
        .await;
    let task_id = task["result"]["taskId"].as_str().unwrap().to_string();
    let worker = client
        .call(
            3,
            "worker/create",
            json!({ "fingerprint": "sha256:f", "adapter": "fake", "model": "m" }),
        )
        .await;
    let worker_id = worker["result"]["workerId"].as_str().unwrap().to_string();

    let submit = client
        .call(
            4,
            "run/submit",
            json!({
                "taskId": task_id,
                "workerId": worker_id,
                // Only `herdr`, with no fallback: on a machine without
                // it this resolves to nothing at all.
                "displayPreference": { "ordered": ["herdr"], "placement": "tab" },
            }),
        )
        .await;
    assert!(
        submit.get("error").is_none(),
        "a headless run still submits: {submit:?}"
    );
    let selected = submit["result"]["display"]["selected"].clone();

    let replay = client
        .call(5, "events/replay", json!({ "afterSequence": 0 }))
        .await;
    let attached = pane_events(&replay, "displayPaneAttached");
    match selected.as_str() {
        Some(backend) => {
            assert_eq!(attached.len(), 1, "a selection journals one attach");
            assert_eq!(attached[0]["backend"], backend);
        }
        None => assert!(
            attached.is_empty(),
            "a headless run must journal no pane at all: {attached:?}"
        ),
    }
}

#[tokio::test]
async fn second_nested_worker_observed_on_an_already_actioned_run_never_double_cancels() {
    let driver = Arc::new(ViolationTriggeringRunDriver::default());
    let harness = Harness::start(|c| {
        c.run_driver = Some(Arc::clone(&driver) as Arc<dyn RunDriver>);
        // Default QuarantineAndCancel: the first observation cancels the
        // run (terminal), so the idempotency guard is state-based here.
    })
    .await;
    let mut client = omp_client(&harness, "omp-1").await;
    let (_, _, run_id) = submit_run_with_driver(&mut client, "omp-1").await;

    assert_eq!(
        driver.cancel_calls().len(),
        1,
        "first observation must cancel once"
    );

    // A second, independent NestedWorkerObserved on the same (now
    // terminal) run -- e.g. the adapter reports a further unexpected
    // child before it is torn down.
    driver
        .emit_nested_worker_observed("child-vendor-2", "parent-vendor-2")
        .await
        .expect("a second NestedWorkerObserved must still be journaled");

    // Still exactly one cancel_run call: the second observation must not
    // create a second cancellation intent or call cancel_run again.
    assert_eq!(
        driver.cancel_calls().len(),
        1,
        "an already-actioned run must not be cancelled twice"
    );

    // But both observations are durably recorded (Option B: still
    // journal, just skip re-applying the action) -- OMP can see that a
    // run was hit by more than one unexpected child.
    let replay = client
        .call(6, "events/replay", json!({ "afterSequence": 0 }))
        .await;
    let events = replay["result"].as_array().unwrap();
    let recorded_count = events
        .iter()
        .filter(|e| e["event"]["type"] == "policyViolationRecorded")
        .count();
    assert_eq!(
        recorded_count, 2,
        "both NestedWorkerObserved events must produce a durable policyViolationRecorded"
    );

    let get = client.call(7, "run/get", json!({ "runId": run_id })).await;
    assert_eq!(get["result"]["state"], "cancelled");
}

// --------------------------------------------------------------- harness
struct Harness {
    socket: PathBuf,
    owned_dir: PathBuf,
    _state: tempfile::TempDir,
    _repo: tempfile::TempDir,
    shutdown: Option<oneshot::Sender<()>>,
    join: Option<JoinHandle<()>>,
}

impl Harness {
    async fn start(config_fn: impl FnOnce(&mut ServerConfig)) -> Self {
        let state = tempfile::Builder::new()
            .prefix("bat-os-")
            .tempdir_in("/tmp")
            .unwrap();
        let repo = tempfile::Builder::new()
            .prefix("bat-or-")
            .tempdir_in("/tmp")
            .unwrap();
        std::fs::create_dir(repo.path().join(".git")).unwrap();

        let paths = RuntimePaths::resolve(state.path(), repo.path()).unwrap();
        let db = Arc::new(DatabaseHandle::start(paths.database.clone()).await.unwrap());

        let mut config = ServerConfig {
            credential_reader: matching_reader(),
            ..Default::default()
        };
        config_fn(&mut config);

        let server = Server::bind(paths.socket.clone(), db, paths.project_id, config)
            .await
            .unwrap();
        let socket = server.socket_path().to_path_buf();

        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let join = tokio::spawn(async move {
            let _ = server
                .serve(async move {
                    let _ = shutdown_rx.await;
                })
                .await;
        });

        let owned_dir = std::fs::canonicalize(repo.path()).unwrap();

        Self {
            socket,
            owned_dir,
            _state: state,
            _repo: repo,
            shutdown: Some(shutdown_tx),
            join: Some(join),
        }
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        if let Some(join) = self.join.take() {
            join.abort();
        }
    }
}

// ---------------------------------------------------------------- client

struct Client {
    reader: BufReader<tokio::net::unix::OwnedReadHalf>,
    writer: OwnedWriteHalf,
}

impl Client {
    async fn connect(socket: &Path) -> Self {
        let stream = UnixStream::connect(socket).await.unwrap();
        let (read, writer) = stream.into_split();
        Self {
            reader: BufReader::new(read),
            writer,
        }
    }

    async fn send(&mut self, value: &Value) {
        let line = serde_json::to_string(value).unwrap();
        self.writer.write_all(line.as_bytes()).await.unwrap();
        self.writer.write_all(b"\n").await.unwrap();
        self.writer.flush().await.unwrap();
    }

    async fn recv(&mut self) -> Value {
        let mut line = String::new();
        self.reader.read_line(&mut line).await.unwrap();
        serde_json::from_str(line.trim_end()).unwrap()
    }

    async fn initialize(
        &mut self,
        role: &str,
        instance_id: &str,
        agent_dir: Option<&str>,
    ) -> Value {
        let auth = match role {
            "ompExtension" => json!({
                "role": "ompExtension",
                "instanceId": instance_id,
                "agentDirectory": agent_dir.unwrap()
            }),
            "display" => json!({ "role": "display", "instanceId": instance_id }),
            other => panic!("unsupported role in test helper: {other}"),
        };
        self.send(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "client": { "name": "@nikolasd/batman", "version": "0.1.0" },
                "supported": { "min": { "major": 1, "minor": 0 }, "max": { "major": 1, "minor": 0 } },
                "repository": { "canonicalPath": agent_dir.unwrap_or("/tmp"), "vcsRoot": agent_dir.unwrap_or("/tmp") },
                "auth": auth,
                "capabilities": { "eventReplay": true, "maxFrameBytes": 1048576 },
                "lastSequence": null
            }
        }))
        .await;
        self.recv().await
    }

    async fn call(&mut self, id: i64, method: &str, params: Value) -> Value {
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))
            .await;
        self.recv().await
    }
}

async fn omp_client(harness: &Harness, instance_id: &str) -> Client {
    let mut client = Client::connect(&harness.socket).await;
    let init = client
        .initialize(
            "ompExtension",
            instance_id,
            Some(harness.owned_dir.to_str().unwrap()),
        )
        .await;
    assert!(init.get("error").is_none(), "initialize failed: {init:?}");
    client
}

// -------------------------------------------------------------- task CRUD

#[tokio::test]
async fn task_upsert_then_get_round_trips() {
    let harness = Harness::start(|_| {}).await;
    let mut client = omp_client(&harness, "omp-1").await;

    let upsert = client
        .call(
            2,
            "task/upsert",
            json!({ "ownerClientInstanceId": "omp-1", "revision": 1 }),
        )
        .await;
    assert!(upsert.get("error").is_none(), "upsert failed: {upsert:?}");
    let task_id = upsert["result"]["taskId"].as_str().unwrap().to_string();
    assert!(upsert["result"]["sequence"].as_u64().is_some());

    let get = client
        .call(3, "task/get", json!({ "taskId": task_id }))
        .await;
    assert_eq!(get["result"]["ownerClientInstanceId"], "omp-1");
    assert_eq!(get["result"]["revision"], 1);
}

#[tokio::test]
async fn events_replay_round_trips_committed_mutation_events() {
    // Regression test: `events/replay` must deserialize every stored event.
    // The domain repository previously persisted the full `EventEnvelope`
    // (embedding `sequence`, `timestamp`, ...) under `event_json`, but
    // `replay()` expects that column to hold only the bare `RuntimeEvent`
    // -- reconstructing the envelope from the `events` table's own
    // `sequence`/`timestamp`/`project_id`/`run_id` columns. The mismatch
    // made every replay fail once any mutation had committed.
    let harness = Harness::start(|_| {}).await;
    let mut client = omp_client(&harness, "omp-1").await;

    let upsert = client
        .call(
            2,
            "task/upsert",
            json!({ "ownerClientInstanceId": "omp-1", "revision": 1 }),
        )
        .await;
    assert!(upsert.get("error").is_none(), "upsert failed: {upsert:?}");
    let task_id = upsert["result"]["taskId"].as_str().unwrap().to_string();

    let replay = client
        .call(3, "events/replay", json!({ "afterSequence": 0 }))
        .await;
    assert!(
        replay.get("error").is_none(),
        "events/replay failed: {replay:?}"
    );
    let events = replay["result"]
        .as_array()
        .expect("events/replay returns an array");
    assert!(
        !events.is_empty(),
        "the committed task/upsert event must be replayable"
    );
    let task_event = events
        .iter()
        .find(|e| e["event"]["type"] == "taskEvent")
        .expect("a taskEvent must be present in the replayed events");
    assert_eq!(task_event["event"]["payload"]["taskId"], task_id);
    assert_eq!(
        task_event["event"]["payload"]["ownerClientInstanceId"],
        "omp-1"
    );
}

#[tokio::test]
async fn events_subscribe_delivers_live_notifications_for_orchestration_mutations() {
    // Regression test: every orchestration/coordination mutation must
    // broadcast its committed envelope to live `events/subscribe`
    // listeners. The broadcast channel previously had no publisher at
    // all, so a monitor connected before a mutation never observed it
    // without reconnecting (which re-triggers `events/replay`).
    let harness = Harness::start(|_| {}).await;
    let mut subscriber = omp_client(&harness, "omp-sub").await;
    let sub = subscriber.call(2, "events/subscribe", json!({})).await;
    assert!(
        sub.get("error").is_none(),
        "events/subscribe failed: {sub:?}"
    );
    assert_eq!(sub["result"]["active"], true);

    let mut mutator = omp_client(&harness, "omp-mut").await;
    let upsert = mutator
        .call(
            2,
            "task/upsert",
            json!({ "ownerClientInstanceId": "omp-mut", "revision": 1 }),
        )
        .await;
    assert!(upsert.get("error").is_none(), "upsert failed: {upsert:?}");
    let task_id = upsert["result"]["taskId"].as_str().unwrap().to_string();

    let notification = subscriber.recv().await;
    assert_eq!(notification["method"], "events/event");
    assert_eq!(notification["params"]["event"]["type"], "taskEvent");
    assert_eq!(
        notification["params"]["event"]["payload"]["taskId"],
        task_id
    );
}

#[tokio::test]
async fn task_upsert_is_idempotent_for_same_revision() {
    let harness = Harness::start(|_| {}).await;
    let mut client = omp_client(&harness, "omp-1").await;

    let first = client
        .call(
            2,
            "task/upsert",
            json!({ "ownerClientInstanceId": "omp-1", "revision": 5 }),
        )
        .await;
    let task_id = first["result"]["taskId"].as_str().unwrap().to_string();

    let second = client
        .call(
            3,
            "task/upsert",
            json!({ "taskId": task_id, "ownerClientInstanceId": "omp-1", "revision": 5 }),
        )
        .await;
    assert!(
        second.get("error").is_none(),
        "same-revision upsert must succeed: {second:?}"
    );

    let get = client
        .call(4, "task/get", json!({ "taskId": task_id }))
        .await;
    assert_eq!(get["result"]["revision"], 5);
}

#[tokio::test]
async fn task_upsert_rejects_lower_revision() {
    let harness = Harness::start(|_| {}).await;
    let mut client = omp_client(&harness, "omp-1").await;

    let first = client
        .call(
            2,
            "task/upsert",
            json!({ "ownerClientInstanceId": "omp-1", "revision": 5 }),
        )
        .await;
    let task_id = first["result"]["taskId"].as_str().unwrap().to_string();

    let lower = client
        .call(
            3,
            "task/upsert",
            json!({ "taskId": task_id, "ownerClientInstanceId": "omp-1", "revision": 3 }),
        )
        .await;
    assert_eq!(
        lower["error"]["code"], -32602,
        "lower revision must be INVALID_PARAMS: {lower:?}"
    );
}

// ------------------------------------------------------------ worker CRUD

#[tokio::test]
async fn worker_create_then_list_and_get() {
    let harness = Harness::start(|_| {}).await;
    let mut client = omp_client(&harness, "omp-1").await;

    let create = client
        .call(
            2,
            "worker/create",
            json!({ "fingerprint": "sha256:fake", "adapter": "fake", "model": "test-model" }),
        )
        .await;
    assert!(
        create.get("error").is_none(),
        "worker/create failed: {create:?}"
    );
    let worker_id = create["result"]["workerId"].as_str().unwrap().to_string();
    assert!(create["result"]["sequence"].as_u64().is_some());

    let list = client.call(3, "worker/list", json!({})).await;
    let workers = list["result"]["workers"].as_array().unwrap();
    assert_eq!(workers.len(), 1);
    assert_eq!(workers[0]["workerId"], worker_id);

    let get = client
        .call(4, "worker/get", json!({ "workerId": worker_id }))
        .await;
    assert_eq!(get["result"]["profileRef"]["adapter"], "fake");
}

// --------------------------------------------------------------- run flow

#[tokio::test]
async fn run_submit_without_driver_reports_adapter_unavailable_but_preserves_queued_run() {
    let harness = Harness::start(|_| {}).await;
    let mut client = omp_client(&harness, "omp-1").await;

    let task = client
        .call(
            2,
            "task/upsert",
            json!({ "ownerClientInstanceId": "omp-1", "revision": 1 }),
        )
        .await;
    let task_id = task["result"]["taskId"].as_str().unwrap().to_string();
    let worker = client
        .call(
            3,
            "worker/create",
            json!({ "fingerprint": "sha256:f", "adapter": "fake", "model": "m" }),
        )
        .await;
    let worker_id = worker["result"]["workerId"].as_str().unwrap().to_string();

    let submit = client
        .call(
            4,
            "run/submit",
            json!({ "taskId": task_id, "workerId": worker_id }),
        )
        .await;
    assert_eq!(submit["error"]["message"], "adapter_unavailable");

    // The run itself is still queued, not silently dropped.
    let list = client
        .call(5, "run/list", json!({ "taskId": task_id }))
        .await;
    let runs = list["result"]["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0]["state"], "queued");
}

#[tokio::test]
async fn run_submit_with_fake_driver_reaches_working_and_retry_creates_new_run() {
    let harness = Harness::start(|c| {
        c.run_driver = Some(Arc::new(FakeRunDriver));
    })
    .await;
    let mut client = omp_client(&harness, "omp-1").await;

    let task = client
        .call(
            2,
            "task/upsert",
            json!({ "ownerClientInstanceId": "omp-1", "revision": 1 }),
        )
        .await;
    let task_id = task["result"]["taskId"].as_str().unwrap().to_string();
    let worker = client
        .call(
            3,
            "worker/create",
            json!({ "fingerprint": "sha256:f", "adapter": "fake", "model": "m" }),
        )
        .await;
    let worker_id = worker["result"]["workerId"].as_str().unwrap().to_string();

    let submit = client
        .call(
            4,
            "run/submit",
            json!({ "taskId": task_id, "workerId": worker_id }),
        )
        .await;
    assert!(
        submit.get("error").is_none(),
        "run/submit failed: {submit:?}"
    );
    let run_id = submit["result"]["runId"].as_str().unwrap().to_string();

    let get = client.call(5, "run/get", json!({ "runId": run_id })).await;
    assert_eq!(
        get["result"]["state"], "working",
        "fake driver must reach working: {get:?}"
    );

    // Cancel the run to reach a terminal state before retrying.
    let cancel = client
        .call(6, "run/cancel", json!({ "runId": run_id }))
        .await;
    assert!(
        cancel.get("error").is_none(),
        "run/cancel failed: {cancel:?}"
    );

    let retry = client
        .call(
            7,
            "run/retry",
            json!({ "priorRunId": run_id, "workerId": worker_id }),
        )
        .await;
    assert!(retry.get("error").is_none(), "run/retry failed: {retry:?}");
    let new_run_id = retry["result"]["runId"].as_str().unwrap();
    assert_ne!(new_run_id, run_id, "retry must create a distinct RunId");
    assert_eq!(
        retry["result"]["taskId"], task_id,
        "retry must retain the same TaskId"
    );

    // The retried run must also reach working, proving retry actually starts the adapter.
    let get_retry = client
        .call(8, "run/get", json!({ "runId": new_run_id }))
        .await;
    assert_eq!(
        get_retry["result"]["state"], "working",
        "retried run must reach working: {get_retry:?}"
    );
}

/// A [`RunDriver`] that captures every [`RunDriverContext`] passed to `start()`
/// and delegates the actual state transitions to a [`FakeRunDriver`].
#[derive(Default)]
struct StartCapturingRunDriver {
    started: parking_lot::Mutex<Vec<RunDriverContext>>,
    inner: FakeRunDriver,
}

impl RunDriver for StartCapturingRunDriver {
    fn start(&self, ctx: RunDriverContext) -> AdapterFuture<'static, Result<(), String>> {
        self.started.lock().push(ctx.clone());
        self.inner.start(ctx)
    }

    fn send_follow_up(
        &self,
        run_id: RunId,
        task_id: TaskId,
        worker_id: WorkerId,
        prompt: String,
    ) -> AdapterFuture<'static, Result<(), String>> {
        self.inner
            .send_follow_up(run_id, task_id, worker_id, prompt)
    }

    fn running_adapter(&self, run_id: RunId) -> Option<Arc<dyn Adapter>> {
        self.inner.running_adapter(run_id)
    }

    fn cancel_run(
        &self,
        run_id: RunId,
        scope: CancelScope,
    ) -> AdapterFuture<'static, Result<(), String>> {
        self.inner.cancel_run(run_id, scope)
    }
}

#[tokio::test]
async fn retry_starts_the_adapter_with_the_supplied_prompt() {
    let driver = Arc::new(StartCapturingRunDriver::default());

    let harness = Harness::start(|c| {
        c.run_driver = Some(Arc::clone(&driver) as Arc<dyn RunDriver>);
    })
    .await;
    let mut client = omp_client(&harness, "omp-1").await;

    let task = client
        .call(
            2,
            "task/upsert",
            json!({ "ownerClientInstanceId": "omp-1", "revision": 1 }),
        )
        .await;
    let task_id = task["result"]["taskId"].as_str().unwrap().to_string();
    let worker = client
        .call(
            3,
            "worker/create",
            json!({ "fingerprint": "sha256:f", "adapter": "fake", "model": "m" }),
        )
        .await;
    let worker_id = worker["result"]["workerId"].as_str().unwrap().to_string();

    // Submit with prompt "first".
    let submit = client
        .call(
            4,
            "run/submit",
            json!({ "taskId": task_id, "workerId": worker_id, "prompt": "first" }),
        )
        .await;
    assert!(
        submit.get("error").is_none(),
        "run/submit failed: {submit:?}"
    );
    let run_id = submit["result"]["runId"].as_str().unwrap().to_string();

    // Cancel to reach terminal state.
    let cancel = client
        .call(5, "run/cancel", json!({ "runId": run_id }))
        .await;
    assert!(
        cancel.get("error").is_none(),
        "run/cancel failed: {cancel:?}"
    );

    // Retry with prompt "second".
    let retry = client
        .call(
            6,
            "run/retry",
            json!({ "priorRunId": run_id, "workerId": worker_id, "prompt": "second" }),
        )
        .await;
    assert!(retry.get("error").is_none(), "run/retry failed: {retry:?}");
    let new_run_id = retry["result"]["runId"].as_str().unwrap().to_string();

    // Assert the driver was started twice, and the second start carried the retry prompt.
    let started = driver.started.lock();
    assert_eq!(started.len(), 2, "driver should have started twice");
    assert_eq!(
        started[0].prompt.as_deref(),
        Some("first"),
        "first run should have had prompt 'first'"
    );
    assert_eq!(
        started[1].prompt.as_deref(),
        Some("second"),
        "retried run should have had prompt 'second'"
    );
    assert_eq!(
        started[1].run_id.to_string(),
        new_run_id,
        "retried run should have the new run id"
    );
}

#[tokio::test]
async fn retry_without_a_driver_reports_adapter_unavailable_and_preserves_the_queued_run() {
    // Harness with no driver injected.
    let harness = Harness::start(|_| {}).await;
    let mut client = omp_client(&harness, "omp-1").await;

    let task = client
        .call(
            2,
            "task/upsert",
            json!({ "ownerClientInstanceId": "omp-1", "revision": 1 }),
        )
        .await;
    let task_id = task["result"]["taskId"].as_str().unwrap().to_string();
    let worker = client
        .call(
            3,
            "worker/create",
            json!({ "fingerprint": "sha256:f", "adapter": "fake", "model": "m" }),
        )
        .await;
    let worker_id = worker["result"]["workerId"].as_str().unwrap().to_string();

    // Submit fails with adapter_unavailable, but the queued run is preserved.
    let submit = client
        .call(
            4,
            "run/submit",
            json!({ "taskId": task_id, "workerId": worker_id }),
        )
        .await;
    assert_eq!(submit["error"]["message"], "adapter_unavailable");

    // Get the run id from the list.
    let list = client
        .call(5, "run/list", json!({ "taskId": task_id }))
        .await;
    let runs = list["result"]["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0]["state"], "queued");
    let run_id = runs[0]["runId"].as_str().unwrap().to_string();

    // Manually transition to cancelled so we have a terminal state to retry from.
    let cancel = client
        .call(6, "run/cancel", json!({ "runId": run_id }))
        .await;
    assert!(
        cancel.get("error").is_none(),
        "run/cancel failed: {cancel:?}"
    );

    // Retry without a driver should also fail with adapter_unavailable.
    let retry = client
        .call(
            7,
            "run/retry",
            json!({ "priorRunId": run_id, "workerId": worker_id }),
        )
        .await;
    assert_eq!(retry["error"]["message"], "adapter_unavailable");

    // The retried run is still queued (preserved).
    let list2 = client
        .call(8, "run/list", json!({ "taskId": task_id }))
        .await;
    let runs2 = list2["result"]["runs"].as_array().unwrap();
    assert_eq!(runs2.len(), 2, "should have original and retried runs");
    assert_eq!(runs2[1]["state"], "queued", "retried run should be queued");
}

#[tokio::test]
async fn run_cancel_on_settled_run_is_illegal_transition() {
    let harness = Harness::start(|c| {
        c.run_driver = Some(Arc::new(FakeRunDriver));
    })
    .await;
    let mut client = omp_client(&harness, "omp-1").await;

    let task = client
        .call(
            2,
            "task/upsert",
            json!({ "ownerClientInstanceId": "omp-1", "revision": 1 }),
        )
        .await;
    let task_id = task["result"]["taskId"].as_str().unwrap().to_string();
    let worker = client
        .call(
            3,
            "worker/create",
            json!({ "fingerprint": "sha256:f", "adapter": "fake", "model": "m" }),
        )
        .await;
    let worker_id = worker["result"]["workerId"].as_str().unwrap().to_string();
    let submit = client
        .call(
            4,
            "run/submit",
            json!({ "taskId": task_id, "workerId": worker_id }),
        )
        .await;
    let run_id = submit["result"]["runId"].as_str().unwrap().to_string();

    // First cancel succeeds (working -> cancelled).
    let cancel = client
        .call(5, "run/cancel", json!({ "runId": run_id }))
        .await;
    assert!(cancel.get("error").is_none());

    // Second cancel on an already-terminal run is illegal.
    let second_cancel = client
        .call(6, "run/cancel", json!({ "runId": run_id }))
        .await;
    assert_eq!(
        second_cancel["error"]["code"], -32100,
        "expected ILLEGAL_TRANSITION: {second_cancel:?}"
    );
}

#[tokio::test]
async fn run_submit_rejects_task_outside_project() {
    let harness = Harness::start(|_| {}).await;
    let mut client = omp_client(&harness, "omp-1").await;

    let worker = client
        .call(
            2,
            "worker/create",
            json!({ "fingerprint": "sha256:f", "adapter": "fake", "model": "m" }),
        )
        .await;
    let worker_id = worker["result"]["workerId"].as_str().unwrap().to_string();

    // A well-formed but nonexistent taskId.
    let fake_task_id = "018f0000-0000-7000-8000-000000000000";
    let submit = client
        .call(
            3,
            "run/submit",
            json!({ "taskId": fake_task_id, "workerId": worker_id }),
        )
        .await;
    assert_eq!(
        submit["error"]["code"], -32602,
        "expected INVALID_PARAMS for unknown task: {submit:?}"
    );
}

// -------------------------------------------------------- message/approval

#[tokio::test]
async fn message_send_then_list() {
    let harness = Harness::start(|c| {
        c.run_driver = Some(Arc::new(FakeRunDriver));
    })
    .await;
    let mut client = omp_client(&harness, "omp-1").await;

    let task = client
        .call(
            2,
            "task/upsert",
            json!({ "ownerClientInstanceId": "omp-1", "revision": 1 }),
        )
        .await;
    let task_id = task["result"]["taskId"].as_str().unwrap().to_string();
    let worker = client
        .call(
            3,
            "worker/create",
            json!({ "fingerprint": "sha256:f", "adapter": "fake", "model": "m" }),
        )
        .await;
    let worker_id = worker["result"]["workerId"].as_str().unwrap().to_string();
    let submit = client
        .call(
            4,
            "run/submit",
            json!({ "taskId": task_id, "workerId": worker_id }),
        )
        .await;
    let run_id = submit["result"]["runId"].as_str().unwrap().to_string();

    let send = client
        .call(
            5,
            "message/send",
            json!({
                "runId": run_id,
                "senderWorkerId": worker_id,
                "taskId": task_id,
                "kind": "question",
                "payload": "what should I do next?"
            }),
        )
        .await;
    assert!(send.get("error").is_none(), "message/send failed: {send:?}");

    let list = client
        .call(6, "message/list", json!({ "runId": run_id }))
        .await;
    let messages = list["result"]["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["kind"], "question");
    assert_eq!(messages[0]["payload"], "what should I do next?");
}

#[tokio::test]
async fn message_send_on_a_run_with_no_running_adapter_still_succeeds_and_journals_a_diagnostic() {
    // FakeRunDriver's own `send_follow_up` always errors -- exactly the
    // shape of the real AdapterRegistry's `RegistryError::NoRunningAdapter`
    // for a `queued` run that has not reached a live adapter instance yet.
    // The RPC must still succeed and the message must still be recorded.
    let harness = Harness::start(|c| {
        c.run_driver = Some(Arc::new(FakeRunDriver));
    })
    .await;
    let mut client = omp_client(&harness, "omp-1").await;

    let task = client
        .call(
            2,
            "task/upsert",
            json!({ "ownerClientInstanceId": "omp-1", "revision": 1 }),
        )
        .await;
    let task_id = task["result"]["taskId"].as_str().unwrap().to_string();
    let worker = client
        .call(
            3,
            "worker/create",
            json!({ "fingerprint": "sha256:f", "adapter": "fake", "model": "m" }),
        )
        .await;
    let worker_id = worker["result"]["workerId"].as_str().unwrap().to_string();
    let submit = client
        .call(
            4,
            "run/submit",
            json!({ "taskId": task_id, "workerId": worker_id }),
        )
        .await;
    let run_id = submit["result"]["runId"].as_str().unwrap().to_string();

    let send = client
        .call(
            5,
            "message/send",
            json!({
                "runId": run_id,
                "senderWorkerId": worker_id,
                "taskId": task_id,
                "kind": "question",
                "payload": "should I proceed?"
            }),
        )
        .await;
    assert!(
        send.get("error").is_none(),
        "message/send must succeed despite delivery failure: {send:?}"
    );

    let list = client
        .call(6, "message/list", json!({ "runId": run_id }))
        .await;
    let messages = list["result"]["messages"].as_array().unwrap();
    assert_eq!(
        messages.len(),
        1,
        "the message must still be durably recorded"
    );

    let replay = client
        .call(7, "events/replay", json!({ "afterSequence": 0 }))
        .await;
    let events = replay["result"]
        .as_array()
        .expect("events/replay returns an array");
    let diagnostic = events
        .iter()
        .find(|e| e["event"]["type"] == "diagnostic")
        .expect("a diagnostic event must be journaled for the failed follow-up delivery");
    assert_eq!(
        diagnostic["event"]["payload"]["code"],
        "follow_up_delivery_failed"
    );
    assert_eq!(diagnostic["event"]["payload"]["level"], "warning");
    assert_eq!(
        diagnostic["runId"], run_id,
        "the diagnostic must be scoped to the run"
    );
}

#[tokio::test]
async fn message_send_on_a_driver_backed_run_reaches_send_follow_up_exactly_once() {
    let driver = Arc::new(RecordingRunDriver::default());
    let harness = Harness::start(|c| {
        c.run_driver = Some(Arc::clone(&driver) as Arc<dyn RunDriver>);
    })
    .await;
    let mut client = omp_client(&harness, "omp-1").await;

    let task = client
        .call(
            2,
            "task/upsert",
            json!({ "ownerClientInstanceId": "omp-1", "revision": 1 }),
        )
        .await;
    let task_id = task["result"]["taskId"].as_str().unwrap().to_string();
    let worker = client
        .call(
            3,
            "worker/create",
            json!({ "fingerprint": "sha256:f", "adapter": "fake", "model": "m" }),
        )
        .await;
    let worker_id = worker["result"]["workerId"].as_str().unwrap().to_string();
    let submit = client
        .call(
            4,
            "run/submit",
            json!({ "taskId": task_id, "workerId": worker_id }),
        )
        .await;
    let run_id = submit["result"]["runId"].as_str().unwrap().to_string();

    let send = client
        .call(
            5,
            "message/send",
            json!({
                "runId": run_id,
                "senderWorkerId": worker_id,
                "taskId": task_id,
                "kind": "question",
                "payload": "verbatim payload text"
            }),
        )
        .await;
    assert!(send.get("error").is_none(), "message/send failed: {send:?}");

    let follow_ups = driver.follow_ups.lock();
    assert_eq!(
        follow_ups.len(),
        1,
        "send_follow_up must be called exactly once"
    );
    let (recorded_run, recorded_task, recorded_worker, recorded_payload) = &follow_ups[0];
    assert_eq!(recorded_run.to_string(), run_id);
    assert_eq!(recorded_task.to_string(), task_id);
    assert_eq!(recorded_worker.to_string(), worker_id);
    assert_eq!(recorded_payload, "verbatim payload text");
}

// --------------------------------------------------------------- sequence

#[tokio::test]
async fn every_mutation_returns_a_strictly_increasing_sequence() {
    let harness = Harness::start(|_| {}).await;
    let mut client = omp_client(&harness, "omp-1").await;

    let first = client
        .call(
            2,
            "task/upsert",
            json!({ "ownerClientInstanceId": "omp-1", "revision": 1 }),
        )
        .await;
    let second = client
        .call(
            3,
            "worker/create",
            json!({ "fingerprint": "sha256:f", "adapter": "fake", "model": "m" }),
        )
        .await;

    let seq1 = first["result"]["sequence"].as_u64().unwrap();
    let seq2 = second["result"]["sequence"].as_u64().unwrap();
    assert!(
        seq2 > seq1,
        "sequence numbers must strictly increase: {seq1} then {seq2}"
    );
}

// ----------------------------------------------------------- role gating

#[tokio::test]
async fn display_principal_cannot_call_orchestration_mutation_methods() {
    let harness = Harness::start(|_| {}).await;
    let mut client = Client::connect(&harness.socket).await;
    client.initialize("display", "display-1", None).await;

    let attempt = client
        .call(
            2,
            "task/upsert",
            json!({ "ownerClientInstanceId": "omp-1", "revision": 1 }),
        )
        .await;
    assert_eq!(
        attempt["error"]["code"], -32601,
        "display must get METHOD_NOT_FOUND: {attempt:?}"
    );
}

// ------------------------------------------------------------- reconcile

#[tokio::test]
async fn reconcile_omp_rebinds_task_ownership_on_matching_revision() {
    let harness = Harness::start(|_| {}).await;
    let mut first_client = omp_client(&harness, "omp-1").await;

    let created = first_client
        .call(
            2,
            "task/upsert",
            json!({ "ownerClientInstanceId": "omp-1", "revision": 7 }),
        )
        .await;
    let task_id = created["result"]["taskId"].as_str().unwrap().to_string();

    // A second OMP instance connects and cannot mutate the task without reconciling.
    let mut second_client = omp_client(&harness, "omp-2").await;
    let reconcile = second_client
        .call(
            2,
            "reconcile/omp",
            json!({ "taskId": task_id, "revision": 7 }),
        )
        .await;
    assert!(
        reconcile.get("error").is_none(),
        "reconcile/omp failed: {reconcile:?}"
    );
    assert_eq!(reconcile["result"]["newOwnerClientInstanceId"], "omp-2");

    let get = second_client
        .call(3, "task/get", json!({ "taskId": task_id }))
        .await;
    assert_eq!(get["result"]["ownerClientInstanceId"], "omp-2");
}

#[tokio::test]
async fn reconcile_omp_rejects_mismatched_revision() {
    let harness = Harness::start(|_| {}).await;
    let mut first_client = omp_client(&harness, "omp-1").await;

    let created = first_client
        .call(
            2,
            "task/upsert",
            json!({ "ownerClientInstanceId": "omp-1", "revision": 7 }),
        )
        .await;
    let task_id = created["result"]["taskId"].as_str().unwrap().to_string();

    let mut second_client = omp_client(&harness, "omp-2").await;
    let reconcile = second_client
        .call(
            2,
            "reconcile/omp",
            json!({ "taskId": task_id, "revision": 99 }),
        )
        .await;
    assert_eq!(
        reconcile["error"]["code"], -32602,
        "mismatched revision must be INVALID_PARAMS: {reconcile:?}"
    );
}
#[tokio::test]
async fn workspace_acquire_returns_lease_for_valid_run() {
    let harness = Harness::start(|c| {
        c.run_driver = Some(Arc::new(FakeRunDriver));
    })
    .await;
    let mut client = omp_client(&harness, "omp-1").await;

    // Create a task, worker, and run to get a run_id
    let task = client
        .call(
            2,
            "task/upsert",
            json!({ "ownerClientInstanceId": "omp-1", "revision": 1 }),
        )
        .await;
    let task_id = task["result"]["taskId"].as_str().unwrap().to_string();

    let worker = client
        .call(
            3,
            "worker/create",
            json!({ "fingerprint": "sha256:f", "adapter": "fake", "model": "m" }),
        )
        .await;
    let worker_id = worker["result"]["workerId"].as_str().unwrap().to_string();

    let submit = client
        .call(
            4,
            "run/submit",
            json!({ "taskId": task_id, "workerId": worker_id }),
        )
        .await;
    assert!(
        submit.get("error").is_none(),
        "run/submit failed: {submit:?}"
    );

    let run_id = submit["result"]["runId"].as_str().unwrap().to_string();

    // Now acquire a workspace for that run
    let acquire = client
        .call(
            5,
            "workspace/acquire",
            json!({
                "runId": run_id,
                "mode": "readOnly",
                "requestedIsolation": "shared"
            }),
        )
        .await;

    assert!(
        acquire.get("error").is_none(),
        "workspace/acquire failed: {acquire:?}"
    );
    assert!(
        acquire["result"]["leaseId"].as_str().is_some(),
        "leaseId missing in response: {acquire:?}"
    );
    assert_eq!(
        acquire["result"]["runId"].as_str().unwrap(),
        &run_id,
        "runId mismatch in response"
    );
}

/// An unrecognized `workspaceMode` is rejected rather than silently
/// downgraded to the shared repository. The silent fallback was the real
/// hazard: a typo (`"isolatd"`) would run a write-capable agent directly
/// against the user's working tree while the caller believed it was
/// isolated.
#[tokio::test]
async fn run_submit_rejects_an_unrecognized_workspace_mode() {
    let harness = Harness::start(|c| {
        c.run_driver = Some(Arc::new(FakeRunDriver));
    })
    .await;
    let mut client = omp_client(&harness, "omp-1").await;

    let task = client
        .call(
            2,
            "task/upsert",
            json!({ "ownerClientInstanceId": "omp-1", "revision": 1 }),
        )
        .await;
    let task_id = task["result"]["taskId"].as_str().unwrap().to_string();
    let worker = client
        .call(
            3,
            "worker/create",
            json!({ "fingerprint": "sha256:f", "adapter": "fake", "model": "m" }),
        )
        .await;
    let worker_id = worker["result"]["workerId"].as_str().unwrap().to_string();

    let submit = client
        .call(
            4,
            "run/submit",
            json!({ "taskId": task_id, "workerId": worker_id, "workspaceMode": "isolatd" }),
        )
        .await;

    let message = submit["error"]["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("workspaceMode"),
        "a typo'd mode must be refused by name, got: {submit:?}"
    );
    assert_eq!(
        submit["error"]["code"].as_i64(),
        Some(i64::from(batman_protocol::error_code::INVALID_PARAMS)),
        "an unrecognized mode is the caller's error: {submit:?}"
    );

    // `shared` remains accepted and is the documented default's spelling.
    let shared = client
        .call(
            5,
            "run/submit",
            json!({ "taskId": task_id, "workerId": worker_id, "workspaceMode": "shared" }),
        )
        .await;
    assert!(
        shared.get("error").is_none(),
        "shared must remain accepted: {shared:?}"
    );
}
// ---------------------------------------------------------------- item 33: real adapter cancel

/// Locates the `fake-worker` binary, building it if necessary. Each
/// `tests/*.rs` file is a separate compilation unit, so this cannot be
/// shared with `tests/supervisor.rs`'s or `tests/omp_rpc_adapter.rs`'s own
/// copies of this same helper.
fn fake_worker_path() -> PathBuf {
    static PATH: std::sync::LazyLock<PathBuf> = std::sync::LazyLock::new(build_fake_worker_once);
    PATH.clone()
}

fn build_fake_worker_once() -> PathBuf {
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("parent of runtime crate");
    let status = std::process::Command::new(env!("CARGO"))
        .args(["build", "--quiet", "-p", "fake-worker"])
        .current_dir(workspace_root)
        .status()
        .expect("cargo build -p fake-worker must be runnable");
    assert!(status.success(), "cargo build -p fake-worker failed");
    let target_dir = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace_root.join("target"));
    let profile_dir = std::env::var("PROFILE").unwrap_or_else(|_| "debug".to_string());
    let binary = target_dir.join(profile_dir).join("fake-worker");
    assert!(
        binary.is_file(),
        "expected fake-worker binary at {}",
        binary.display()
    );
    binary
}

/// Collects every `AdapterEvent` a real adapter emits, so the test can
/// read back the OS pid `OmpRpcAdapter::start` reports via
/// `AdapterEventPayload::ProcessStarted` -- the adapter itself exposes no
/// pid accessor, this is the only observable path to it.
#[derive(Default)]
struct TestSink {
    events: tokio::sync::Mutex<Vec<AdapterEvent>>,
}

impl TestSink {
    async fn process_started_pid(&self) -> Option<u32> {
        self.events
            .lock()
            .await
            .iter()
            .find_map(|event| match &event.payload {
                AdapterEventPayload::ProcessStarted { pid } => Some(*pid),
                _ => None,
            })
    }
}

impl AdapterEventSink for TestSink {
    fn emit(&self, event: AdapterEvent) -> batman_runtime::adapter::AdapterFuture<'_, u64> {
        Box::pin(async move {
            let mut events = self.events.lock().await;
            events.push(event);
            Ok(events.len() as u64)
        })
    }
}

/// A `RunDriver` that constructs a real `OmpRpcAdapter` (via
/// `OmpRpcAdapter::with_binary`, pointed at the `fake-worker` fixture) and
/// stores it, delegating `running_adapter`/`cancel_run` to the adapter's
/// own methods exactly as `AdapterRegistry` does -- exercising the real
/// `Adapter::cancel` implementation (`run_pump`'s
/// `client.process_mut().terminate().await`), not a hand-rolled stand-in.
struct RealAdapterRunDriver {
    adapter: parking_lot::Mutex<Option<Arc<OmpRpcAdapter>>>,
    sink: Arc<TestSink>,
}

impl Default for RealAdapterRunDriver {
    fn default() -> Self {
        Self {
            adapter: parking_lot::Mutex::new(None),
            sink: Arc::new(TestSink::default()),
        }
    }
}

impl RealAdapterRunDriver {
    /// The fake-worker's real OS pid, once `OmpRpcAdapter::start` has
    /// emitted `ProcessStarted` (always true by the time `start` returns).
    async fn pid(&self) -> Option<u32> {
        self.sink.process_started_pid().await
    }
}

impl RunDriver for RealAdapterRunDriver {
    fn start(&self, ctx: RunDriverContext) -> AdapterFuture<'static, Result<(), String>> {
        let adapter = Arc::new(OmpRpcAdapter::with_binary(
            fake_worker_path().to_string_lossy().into_owned(),
            WorkerProfile {
                id: ProfileId::new(),
                adapter: "ompRpc".to_string(),
                model: "lm-studio/x".to_string(),
                permission_envelope: serde_json::json!({}),
                startup_options: StartupOptions::OmpRpc(OmpRpcStartupOptions {
                    profile: None,
                    host_tools: None,
                }),
                environment_allowlist: Vec::new(),
                source: "test".to_string(),
            },
            OmpRpcAdapterOptions::default(),
            None,
        ));
        *self.adapter.lock() = Some(Arc::clone(&adapter));
        let sink = Arc::clone(&self.sink) as Arc<dyn AdapterEventSink>;
        Box::pin(async move {
            adapter
                .start(
                    StartSpec {
                        run_id: ctx.run_id,
                        task_id: ctx.task_id,
                        worker_id: ctx.worker_id,
                        prompt: ctx.prompt.clone().unwrap_or_default(),
                        resume: None,
                    },
                    sink,
                )
                .await
                .map_err(|e| e.to_string())
        })
    }

    fn send_follow_up(
        &self,
        _run_id: RunId,
        _task_id: TaskId,
        _worker_id: WorkerId,
        _prompt: String,
    ) -> AdapterFuture<'static, Result<(), String>> {
        Box::pin(async { Err("not supported".to_string()) })
    }

    fn running_adapter(&self, _run_id: RunId) -> Option<Arc<dyn Adapter>> {
        self.adapter
            .lock()
            .as_ref()
            .map(|a| Arc::clone(a) as Arc<dyn Adapter>)
    }

    fn cancel_run(
        &self,
        _run_id: RunId,
        scope: CancelScope,
    ) -> AdapterFuture<'static, Result<(), String>> {
        let adapter = Arc::clone(self.adapter.lock().as_ref().unwrap());
        Box::pin(async move { adapter.cancel(scope).await.map_err(|e| e.to_string()) })
    }
}

/// Closes item 33's remaining gap: proves `run/cancel` reaches the *real*
/// `AdapterRegistry`-equivalent chain -- `RunDriver::cancel_run` ->
/// `OmpRpcAdapter::cancel()` -> `run_pump`'s
/// `client.process_mut().terminate().await` -- and the OS-level vendor
/// subprocess actually dies, not merely that the run's database state
/// becomes `"cancelled"`.
///
/// Uses `OmpRpcAdapter::with_binary` pointed at the `fake-worker` fixture
/// (its `--mode rpc` argv, always sent verbatim by `OmpRpcAdapter::start`,
/// is aliased to fake-worker's `omp-rpc-host-tool` mode -- see
/// `fake-worker/src/main.rs`'s `Mode::OmpRpcHostTool`), so this exercises
/// the adapter's real, production `cancel()` implementation end to end.
///
/// This does not itself prove SIGKILL escalation (fake-worker's
/// `omp-rpc-host-tool` mode does not ignore SIGINT/SIGTERM, so
/// `ManagedProcess::terminate` is expected to succeed on its first
/// signal) -- that coverage remains `supervisor.rs`'s `ignore-term` test.
#[tokio::test]
async fn run_cancel_reaches_real_omprpc_adapter_and_kills_process() {
    use nix::sys::signal::kill;
    use nix::unistd::Pid;

    let driver = Arc::new(RealAdapterRunDriver::default());

    let harness = Harness::start(|c| {
        c.run_driver = Some(Arc::clone(&driver) as Arc<dyn RunDriver>);
    })
    .await;
    let mut client = omp_client(&harness, "omp-1").await;

    let task = client
        .call(
            2,
            "task/upsert",
            json!({ "ownerClientInstanceId": "omp-1", "revision": 1 }),
        )
        .await;
    let task_id = task["result"]["taskId"].as_str().unwrap().to_string();

    let worker = client
        .call(
            3,
            "worker/create",
            json!({ "fingerprint": "sha256:f", "adapter": "fake", "model": "m" }),
        )
        .await;
    let worker_id = worker["result"]["workerId"].as_str().unwrap().to_string();

    let submit = client
        .call(
            4,
            "run/submit",
            json!({ "taskId": task_id, "workerId": worker_id }),
        )
        .await;
    assert!(
        submit.get("error").is_none(),
        "run/submit failed: {submit:?}"
    );
    let run_id = submit["result"]["runId"].as_str().unwrap().to_string();

    // OmpRpcAdapter::start emits ProcessStarted (carrying the fake-worker's
    // real OS pid) synchronously, before start() ever returns -- so it must
    // already be recorded by the time run/submit's RPC response arrives.
    let pid = driver
        .pid()
        .await
        .expect("OmpRpcAdapter must have emitted ProcessStarted with a real pid");
    let os_pid = Pid::from_raw(pid as i32);
    assert!(
        kill(os_pid, None).is_ok(),
        "fake-worker process (pid {pid}) must be alive right after run/submit"
    );

    let cancel = client
        .call(5, "run/cancel", json!({ "runId": run_id }))
        .await;
    assert!(
        cancel.get("error").is_none(),
        "run/cancel failed: {cancel:?}"
    );

    // OmpRpcAdapter::cancel() only queues Outbound::Terminate on
    // run_pump's channel and returns immediately -- it does not itself
    // await ManagedProcess::terminate() completing. Poll for the process
    // to actually die, bounded well past escalation's worst case at
    // production EscalationTimings::default() (5s + 5s).
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        if kill(os_pid, None).is_err() {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "process (pid {pid}) must be dead after run/cancel reaches the real \
             OmpRpcAdapter::cancel() -> run_pump's ManagedProcess::terminate()"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

// -------------------------------------------------------- artifact isolation

/// Regression test for R10: artifact APIs must scope results by the
/// caller's task ownership. Two OMP-extension clients connecting to the
/// same daemon, each owning a different task, should never see each
/// other's artifacts.
#[tokio::test]
async fn artifact_isolation_enforces_task_ownership_scoping() {
    let store = Arc::new(ArtifactStore::new());

    let harness = Harness::start(|c| {
        c.run_driver = Some(Arc::new(FakeRunDriver));
        c.artifact_store = Some(Arc::clone(&store));
    })
    .await;

    // Client A creates a task, worker, and run
    let mut client_a = omp_client(&harness, "omp-A").await;
    let task_a = client_a
        .call(
            10,
            "task/upsert",
            json!({ "ownerClientInstanceId": "omp-A", "revision": 1 }),
        )
        .await;
    let task_a_id = task_a["result"]["taskId"].as_str().unwrap().to_string();

    let worker_a = client_a
        .call(
            11,
            "worker/create",
            json!({ "fingerprint": "sha256:a", "adapter": "fake", "model": "m" }),
        )
        .await;
    let worker_a_id = worker_a["result"]["workerId"].as_str().unwrap().to_string();

    let submit_a = client_a
        .call(
            12,
            "run/submit",
            json!({ "taskId": task_a_id, "workerId": worker_a_id }),
        )
        .await;
    assert!(
        submit_a.get("error").is_none(),
        "submit A failed: {submit_a:?}"
    );
    let run_a_id = submit_a["result"]["runId"].as_str().unwrap().to_string();

    // Client B creates a task, worker, and run
    let mut client_b = omp_client(&harness, "omp-B").await;
    let task_b = client_b
        .call(
            20,
            "task/upsert",
            json!({ "ownerClientInstanceId": "omp-B", "revision": 1 }),
        )
        .await;
    let task_b_id = task_b["result"]["taskId"].as_str().unwrap().to_string();

    let worker_b = client_b
        .call(
            21,
            "worker/create",
            json!({ "fingerprint": "sha256:b", "adapter": "fake", "model": "m" }),
        )
        .await;
    let worker_b_id = worker_b["result"]["workerId"].as_str().unwrap().to_string();

    let submit_b = client_b
        .call(
            22,
            "run/submit",
            json!({ "taskId": task_b_id, "workerId": worker_b_id }),
        )
        .await;
    assert!(
        submit_b.get("error").is_none(),
        "submit B failed: {submit_b:?}"
    );
    let run_b_id = submit_b["result"]["runId"].as_str().unwrap().to_string();

    // Seed artifacts for each run
    let artifact_a = seed_artifact(&store, "artifact-from-A\n", Some(run_a_id.clone())).await;
    let artifact_b = seed_artifact(&store, "artifact-from-B\n", Some(run_b_id.clone())).await;

    // Client A lists artifacts — should only see their own
    let list_a = client_a.call(30, "artifact/list", json!({})).await;
    let artifacts_a: Vec<String> = list_a["result"]["artifacts"]
        .as_array()
        .expect("artifacts is an array")
        .iter()
        .filter_map(|a| {
            a.get("artifactId")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect();
    assert!(
        artifacts_a.contains(&artifact_a.to_string()),
        "Client A should see their own artifact, got: {artifacts_a:?}"
    );
    assert!(
        !artifacts_a.contains(&artifact_b.to_string()),
        "Client A should NOT see Client B's artifact, got: {artifacts_a:?}"
    );

    // Client B lists artifacts — should only see their own
    let list_b = client_b.call(31, "artifact/list", json!({})).await;
    let artifacts_b: Vec<String> = list_b["result"]["artifacts"]
        .as_array()
        .expect("artifacts is an array")
        .iter()
        .filter_map(|a| {
            a.get("artifactId")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect();
    assert!(
        artifacts_b.contains(&artifact_b.to_string()),
        "Client B should see their own artifact, got: {artifacts_b:?}"
    );
    assert!(
        !artifacts_b.contains(&artifact_a.to_string()),
        "Client B should NOT see Client A's artifact, got: {artifacts_b:?}"
    );

    // Client A tries to fetch B's artifact — should fail
    let fetch_cross = client_a
        .call(
            32,
            "artifact/fetch",
            json!({ "artifactId": artifact_b.to_string() }),
        )
        .await;
    assert!(
        fetch_cross.get("error").is_some(),
        "Client A should NOT be able to fetch Client B's artifact, got: {fetch_cross:?}"
    );
}

/// Seeds an artifact in the store with the given body and run_id.
async fn seed_artifact(
    store: &ArtifactStore,
    body: &str,
    run_id: Option<String>,
) -> batman_protocol::ArtifactId {
    let content = body.as_bytes().to_vec();
    let mut hasher = Sha256::new();
    hasher.update(&content);
    let artifact = batman_protocol::Artifact {
        artifact_id: batman_protocol::ArtifactId::new(),
        kind: batman_protocol::ArtifactKind::Patch,
        sha256: format!("{:x}", hasher.finalize()),
        byte_length: content.len() as u64,
        media_type: "application/x-git-diff".to_string(),
        storage_path: format!("patches/{}.patch", content.len()),
        run_id,
    };
    store.store(artifact, content).await.unwrap()
}
