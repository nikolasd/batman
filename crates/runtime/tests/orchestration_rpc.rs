//! Integration tests for the orchestration JSON-RPC methods.
//!
//! Drives the real [`batman_runtime::ipc::Server`] over a Unix domain
//! socket, exercising `task/upsert|get`, `worker/create|list|get`,
//! `run/submit|list|get|retry|cancel`, `message/send|list`,
//! `approval/list|decide`, and `reconcile/omp`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use batman_protocol::{RunId, TaskId, WorkerId};
use batman_runtime::db::DatabaseHandle;
use batman_runtime::ipc::{PeerCredentialReader, PeerCredentials, Server, ServerConfig};
use batman_runtime::paths::RuntimePaths;
use batman_runtime::adapter::{Adapter, CancelScope};
use batman_runtime::service::{AdapterFuture, FakeRunDriver, RunDriver, RunDriverContext};
use nix::unistd::Uid;
use serde_json::{Value, json};
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

    fn cancel_run(&self, _run_id: RunId, _scope: CancelScope) -> AdapterFuture<'static, Result<(), String>> {
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

    fn cancel_run(&self, run_id: RunId, scope: CancelScope) -> AdapterFuture<'static, Result<(), String>> {
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
    assert!(submit.get("error").is_none(), "run/submit failed: {submit:?}");

    let run_id = submit["result"]["runId"].as_str().unwrap().to_string();

    // Now cancel the run
    let cancel = client
        .call(
            5,
            "run/cancel",
            json!({ "runId": run_id }),
        )
        .await;
    assert!(cancel.get("error").is_none(), "run/cancel failed: {cancel:?}");

    // Verify cancel_run was called with CancelScope::Worker
    let calls = driver.cancel_calls();
    assert_eq!(calls.len(), 1, "expected exactly one cancel_run call");
    let (called_run_id, scope) = &calls[0];
    assert_eq!(called_run_id.to_string(), run_id, "cancel_run called with wrong run_id");
    assert_eq!(*scope, CancelScope::Worker, "cancel_run called with wrong CancelScope");
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
                "client": { "name": "@satori/batman", "version": "0.1.0" },
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
    assert!(submit.get("error").is_none(), "run/submit failed: {submit:?}");

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

    assert!(acquire.get("error").is_none(), "workspace/acquire failed: {acquire:?}");
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
