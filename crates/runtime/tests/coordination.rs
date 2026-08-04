//! Integration tests for the audited coordination broker.
//!
//! Drives the real [`batman_runtime::ipc::Server`] over a Unix domain
//! socket with a real [`batman_runtime::coordination::ScopeTokenStore`]
//! wired as the worker-MCP credential verifier, exercising bounds, reply
//! visibility, task-ownership, rate limiting, and scope-token ancestry.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use batman_protocol::{ProjectId, RunId, TaskId, Timestamp, WorkerId};
use batman_runtime::coordination::{
    CoordinationBroker, ScopeBinding, ScopeTokenStore, ScopeTokenVerifier, VendorProcessIdentity,
    mcp_protocol,
};
use batman_runtime::db::DatabaseHandle;
use batman_runtime::ipc::{PeerCredentialReader, PeerCredentials, Server, ServerConfig};
use batman_runtime::paths::RuntimePaths;
use batman_runtime::service::FakeRunDriver;
use batman_runtime::workspace::LeaseService;
use nix::unistd::Uid;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::net::unix::OwnedWriteHalf;
use tokio::sync::{broadcast, oneshot};
use tokio::task::JoinHandle;

// ------------------------------------------------------------------ fakes

/// Reports a fixed uid and the *current test process's own pid* as the
/// peer -- letting these tests exercise the real
/// [`batman_runtime::coordination::SystemPidAncestryChecker`] end to end:
/// minting a token bound to this same pid as the "vendor process" makes
/// every connection trivially its own ancestor.
struct FakeReader {
    uid: Option<u32>,
    pid: i32,
}

impl PeerCredentialReader for FakeReader {
    fn read(&self, _stream: &UnixStream) -> PeerCredentials {
        PeerCredentials {
            uid: self.uid,
            pid: Some(self.pid),
        }
    }
}

fn current_uid() -> u32 {
    Uid::current().as_raw()
}

fn self_pid() -> i32 {
    std::process::id() as i32
}

fn matching_reader() -> Arc<dyn PeerCredentialReader> {
    Arc::new(FakeReader {
        uid: Some(current_uid()),
        pid: self_pid(),
    })
}

// --------------------------------------------------------------- harness

struct Harness {
    socket: PathBuf,
    owned_dir: PathBuf,
    database: PathBuf,
    project_id: ProjectId,
    scope_token_store: Arc<ScopeTokenStore>,
    _state: tempfile::TempDir,
    _repo: tempfile::TempDir,
    shutdown: Option<oneshot::Sender<()>>,
    join: Option<JoinHandle<()>>,
}

impl Harness {
    async fn start() -> Self {
        let state = tempfile::Builder::new()
            .prefix("bat-co-s-")
            .tempdir_in("/tmp")
            .unwrap();
        let repo = tempfile::Builder::new()
            .prefix("bat-co-r-")
            .tempdir_in("/tmp")
            .unwrap();
        std::fs::create_dir(repo.path().join(".git")).unwrap();

        let paths = RuntimePaths::resolve(state.path(), repo.path()).unwrap();
        let db = Arc::new(DatabaseHandle::start(paths.database.clone()).await.unwrap());

        let scope_token_store = Arc::new(ScopeTokenStore::new());

        let config = ServerConfig {
            credential_reader: matching_reader(),
            worker_verifier: Arc::new(ScopeTokenVerifier::new(scope_token_store.clone())),
            run_driver: Some(Arc::new(FakeRunDriver)),
            ..Default::default()
        };

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

        for _ in 0..50 {
            if UnixStream::connect(&socket).await.is_ok() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        let owned_dir = std::fs::canonicalize(repo.path()).unwrap();

        Self {
            socket,
            owned_dir,
            database: paths.database.clone(),
            project_id: paths.project_id,
            scope_token_store,
            _state: state,
            _repo: repo,
            shutdown: Some(shutdown_tx),
            join: Some(join),
        }
    }

    /// A fresh, standalone [`CoordinationBroker`] against this harness's
    /// own database file -- for tests that call broker methods (here,
    /// [`CoordinationBroker::execute_tool_call`]) directly, in-process,
    /// with no socket connection at all. SQLite's WAL mode makes a
    /// second handle onto the same file safe alongside the server's own.
    async fn broker(&self) -> CoordinationBroker {
        let db = Arc::new(DatabaseHandle::start(self.database.clone()).await.unwrap());
        let (events_tx, _events_rx) = broadcast::channel(16);
        let lease_service = Arc::new(
            LeaseService::open_in_memory(self.project_id).expect("in-memory lease service"),
        );
        CoordinationBroker::new(db, self.project_id, events_tx, lease_service)
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

    async fn call(&mut self, id: i64, method: &str, params: Value) -> Value {
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))
            .await;
        self.recv().await
    }
}

async fn omp_client(harness: &Harness, instance_id: &str) -> Client {
    let mut client = Client::connect(&harness.socket).await;
    client
        .send(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "client": { "name": "@satori/batman", "version": "0.1.0" },
                "supported": { "min": { "major": 1, "minor": 0 }, "max": { "major": 1, "minor": 0 } },
                "repository": { "canonicalPath": harness.owned_dir, "vcsRoot": harness.owned_dir },
                "auth": { "role": "ompExtension", "instanceId": instance_id, "agentDirectory": harness.owned_dir },
                "capabilities": { "eventReplay": true, "maxFrameBytes": 1048576 },
                "lastSequence": null
            }
        }))
        .await;
    let init = client.recv().await;
    assert!(init.get("error").is_none(), "initialize failed: {init:?}");
    client
}

async fn worker_client(harness: &Harness, token: &str) -> Client {
    let mut client = Client::connect(&harness.socket).await;
    client
        .send(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "client": { "name": "worker", "version": "0.1.0" },
                "supported": { "min": { "major": 1, "minor": 0 }, "max": { "major": 1, "minor": 0 } },
                "repository": { "canonicalPath": harness.owned_dir, "vcsRoot": harness.owned_dir },
                "auth": { "role": "workerMcp", "instanceId": "worker-1", "scopeToken": token },
                "capabilities": { "eventReplay": true, "maxFrameBytes": 1048576 },
                "lastSequence": null
            }
        }))
        .await;
    let init = client.recv().await;
    assert!(
        init.get("error").is_none(),
        "worker initialize failed: {init:?}"
    );
    client
}

/// Sets up a task+worker+run and mints a live scope token bound to it,
/// returning the token plus the run/task/worker ids.
async fn seed_scoped_run(harness: &Harness, omp: &mut Client) -> (String, RunId, TaskId, WorkerId) {
    let task = omp
        .call(
            2,
            "task/upsert",
            json!({ "ownerClientInstanceId": "omp-1", "revision": 1 }),
        )
        .await;
    let task_id = TaskId::parse(task["result"]["taskId"].as_str().unwrap()).unwrap();

    let worker = omp
        .call(
            3,
            "worker/create",
            json!({ "fingerprint": "sha256:f", "adapter": "fake", "model": "m" }),
        )
        .await;
    let worker_id = WorkerId::parse(worker["result"]["workerId"].as_str().unwrap()).unwrap();

    let submit = omp
        .call(
            4,
            "run/submit",
            json!({ "taskId": task_id.to_string(), "workerId": worker_id.to_string() }),
        )
        .await;
    let run_id = RunId::parse(submit["result"]["runId"].as_str().unwrap()).unwrap();

    let token = harness.scope_token_store.mint(ScopeBinding {
        project_id: ProjectId::new(),
        task_id,
        worker_id,
        run_id,
        vendor_process: VendorProcessIdentity { pid: self_pid() },
        expires_at: Timestamp::parse("2099-01-01T00:00:00Z").unwrap(),
    });

    (token, run_id, task_id, worker_id)
}

// ------------------------------------------------------- execute_tool_call

fn bound_scope(run_id: RunId, task_id: TaskId, worker_id: WorkerId) -> mcp_protocol::BoundScope {
    mcp_protocol::BoundScope {
        run_id,
        task_id,
        worker_id,
    }
}

#[tokio::test]
async fn execute_tool_call_fulfills_every_tool_against_the_real_broker() {
    let harness = Harness::start().await;
    let mut omp = omp_client(&harness, "omp-1").await;
    let (_token, run_id, task_id, worker_id) = seed_scoped_run(&harness, &mut omp).await;
    // A second worker on the same task, so batman_peers has someone to see.
    let peer_worker = omp
        .call(
            6,
            "worker/create",
            json!({ "fingerprint": "sha256:g", "adapter": "fake", "model": "m" }),
        )
        .await;
    let peer_worker_id = peer_worker["result"]["workerId"]
        .as_str()
        .unwrap()
        .to_string();
    omp.call(
        7,
        "run/submit",
        json!({ "taskId": task_id.to_string(), "workerId": peer_worker_id }),
    )
    .await;

    let broker = harness.broker().await;
    let scope = bound_scope(run_id, task_id, worker_id);

    let task = broker
        .execute_tool_call("batman_task", &json!({}), scope)
        .await;
    assert_eq!(task["isError"], false, "{task:?}");
    assert_eq!(task["structuredContent"]["taskId"], task_id.to_string());

    let peers = broker
        .execute_tool_call("batman_peers", &json!({}), scope)
        .await;
    assert_eq!(peers["isError"], false, "{peers:?}");
    assert_eq!(
        peers["structuredContent"]["peers"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    let send = broker
        .execute_tool_call(
            "batman_send",
            &json!({ "kind": "peerMessage", "payload": "hi peer" }),
            scope,
        )
        .await;
    assert_eq!(send["isError"], false, "{send:?}");
    assert_eq!(send["structuredContent"]["deliveryState"], "sent");

    let request_child = broker
        .execute_tool_call(
            "batman_request_child",
            &json!({ "reason": "need help" }),
            scope,
        )
        .await;
    assert_eq!(request_child["isError"], false, "{request_child:?}");
    assert!(request_child["structuredContent"]["sequence"].is_number());

    let publish = broker
        .execute_tool_call(
            "batman_publish_artifact",
            &json!({ "artifactRef": "artifact://abc", "description": "the diff" }),
            scope,
        )
        .await;
    assert_eq!(publish["isError"], false, "{publish:?}");
    assert_eq!(
        publish["structuredContent"]["artifactRef"],
        "artifact://abc"
    );

    let blocked = broker
        .execute_tool_call(
            "batman_report_blocked",
            &json!({ "reason": "waiting" }),
            scope,
        )
        .await;
    assert_eq!(blocked["isError"], false, "{blocked:?}");
    assert_eq!(blocked["structuredContent"]["deliveryState"], "sent");

    let policy = broker
        .execute_tool_call(
            "batman_ask_policy",
            &json!({ "question": "may I write here?" }),
            scope,
        )
        .await;
    assert_eq!(policy["isError"], false, "{policy:?}");
    assert_eq!(policy["structuredContent"]["deliveryState"], "sent");
}

#[tokio::test]
async fn execute_tool_call_rejects_a_smuggled_sender_worker_id_and_journals_nothing() {
    let harness = Harness::start().await;
    let mut omp = omp_client(&harness, "omp-1").await;
    let (_token, run_id, task_id, worker_id) = seed_scoped_run(&harness, &mut omp).await;
    let broker = harness.broker().await;
    let scope = bound_scope(run_id, task_id, worker_id);

    let spoofed = WorkerId::new();
    let result = broker
        .execute_tool_call(
            "batman_send",
            &json!({
                "kind": "peerMessage",
                "payload": "hi",
                "senderWorkerId": spoofed.to_string(),
            }),
            scope,
        )
        .await;
    assert_eq!(result["isError"], true, "{result:?}");

    let replay = omp
        .call(5, "events/replay", json!({ "afterSequence": 0 }))
        .await;
    let events = replay["result"]
        .as_array()
        .expect("events/replay returns an array");
    assert!(
        !events.iter().any(|e| e["event"]["type"] == "messageEvent"),
        "a rejected call must never journal any message at all: {events:?}"
    );
}

#[tokio::test]
async fn execute_tool_call_reports_an_unknown_tool_as_a_tool_error_not_a_panic() {
    let harness = Harness::start().await;
    let mut omp = omp_client(&harness, "omp-1").await;
    let (_token, run_id, task_id, worker_id) = seed_scoped_run(&harness, &mut omp).await;
    let broker = harness.broker().await;
    let scope = bound_scope(run_id, task_id, worker_id);

    let result = broker
        .execute_tool_call("not_a_real_tool", &json!({}), scope)
        .await;
    assert_eq!(result["isError"], true, "{result:?}");
    assert!(
        result["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("unknown tool")
    );
}

// ------------------------------------------------------------- send bounds

#[tokio::test]
async fn coordination_send_requires_sender_task_and_payload() {
    let harness = Harness::start().await;
    let mut omp = omp_client(&harness, "omp-1").await;
    let (token, run_id, _task_id, _worker_id) = seed_scoped_run(&harness, &mut omp).await;
    let mut worker = worker_client(&harness, &token).await;

    let missing_sender = worker
        .call(2, "coordination/send", json!({ "runId": run_id.to_string(), "taskId": "x", "kind": "question", "payload": "hi" }))
        .await;
    assert_eq!(
        missing_sender["error"]["code"], -32602,
        "{missing_sender:?}"
    );
}

#[tokio::test]
async fn coordination_send_rejects_a_sender_worker_id_outside_the_authenticated_scope() {
    let harness = Harness::start().await;
    let mut omp = omp_client(&harness, "omp-1").await;
    let (token, run_id, task_id, _worker_id) = seed_scoped_run(&harness, &mut omp).await;
    let mut worker = worker_client(&harness, &token).await;

    // A worker connection authenticated for this run's real worker must
    // never be able to claim a *different* worker's identity when it
    // sends -- that would let a scoped connection impersonate a peer.
    let spoofed_sender = WorkerId::new();
    let result = worker
        .call(
            2,
            "coordination/send",
            json!({
                "runId": run_id.to_string(),
                "senderWorkerId": spoofed_sender.to_string(),
                "taskId": task_id.to_string(),
                "kind": "question",
                "payload": "hi",
            }),
        )
        .await;
    assert_eq!(result["error"]["code"], -32602, "{result:?}");
    assert!(
        result["error"]["message"]
            .as_str()
            .unwrap()
            .contains("authenticated scope"),
        "{result:?}"
    );

    // The rejected send must never have been journaled: replaying every
    // event since the beginning finds no message from the spoofed sender.
    let replay = omp
        .call(5, "events/replay", json!({ "afterSequence": 0 }))
        .await;
    let events = replay["result"]
        .as_array()
        .expect("events/replay returns an array");
    assert!(
        !events
            .iter()
            .any(|e| e["event"]["payload"]["senderWorkerId"] == spoofed_sender.to_string()),
        "a rejected spoofed send must never reach the journal: {events:?}"
    );
}

#[tokio::test]
async fn coordination_send_rejects_a_payload_over_64_kib() {
    let harness = Harness::start().await;
    let mut omp = omp_client(&harness, "omp-1").await;
    let (token, run_id, task_id, worker_id) = seed_scoped_run(&harness, &mut omp).await;
    let mut worker = worker_client(&harness, &token).await;

    let huge_payload = "x".repeat(64 * 1024 + 1);
    let result = worker
        .call(
            2,
            "coordination/send",
            json!({
                "runId": run_id.to_string(),
                "senderWorkerId": worker_id.to_string(),
                "taskId": task_id.to_string(),
                "kind": "question",
                "payload": huge_payload,
            }),
        )
        .await;
    assert_eq!(result["error"]["code"], -32602, "{result:?}");
}

#[tokio::test]
async fn coordination_send_accepts_a_payload_at_the_limit() {
    let harness = Harness::start().await;
    let mut omp = omp_client(&harness, "omp-1").await;
    let (token, run_id, task_id, worker_id) = seed_scoped_run(&harness, &mut omp).await;
    let mut worker = worker_client(&harness, &token).await;

    let max_payload = "x".repeat(64 * 1024);
    let result = worker
        .call(
            2,
            "coordination/send",
            json!({
                "runId": run_id.to_string(),
                "senderWorkerId": worker_id.to_string(),
                "taskId": task_id.to_string(),
                "kind": "question",
                "payload": max_payload,
            }),
        )
        .await;
    assert!(result.get("error").is_none(), "{result:?}");
    assert_eq!(result["result"]["deliveryState"], "sent");
}

#[tokio::test]
async fn coordination_send_rejects_a_task_unrelated_to_the_run() {
    let harness = Harness::start().await;
    let mut omp = omp_client(&harness, "omp-1").await;
    let (token, run_id, _task_id, worker_id) = seed_scoped_run(&harness, &mut omp).await;
    let mut worker = worker_client(&harness, &token).await;

    let unrelated_task_id = "018f0000-0000-7000-8000-000000000000";
    let result = worker
        .call(
            2,
            "coordination/send",
            json!({
                "runId": run_id.to_string(),
                "senderWorkerId": worker_id.to_string(),
                "taskId": unrelated_task_id,
                "kind": "question",
                "payload": "hi",
            }),
        )
        .await;
    assert_eq!(result["error"]["code"], -32602, "{result:?}");
    assert!(
        result["error"]["message"]
            .as_str()
            .unwrap()
            .contains("cannot address")
    );
}

#[tokio::test]
async fn coordination_methods_reject_a_run_that_has_already_settled() {
    let harness = Harness::start().await;
    let mut omp = omp_client(&harness, "omp-1").await;
    let (token, run_id, task_id, worker_id) = seed_scoped_run(&harness, &mut omp).await;
    let mut worker = worker_client(&harness, &token).await;

    // Settle the run to a terminal state -- the scope token itself is
    // still technically live (nothing has revoked it yet), but every
    // worker-safe operation must reject it regardless: settlement, not
    // revocation, is what actually ends this run's ability to act.
    let cancel = omp
        .call(5, "run/cancel", json!({ "runId": run_id.to_string() }))
        .await;
    assert!(
        cancel.get("error").is_none(),
        "run/cancel failed: {cancel:?}"
    );

    let task_call = worker
        .call(
            6,
            "coordination/task",
            json!({ "runId": run_id.to_string() }),
        )
        .await;
    assert_eq!(task_call["error"]["code"], -32602, "{task_call:?}");
    assert!(
        task_call["error"]["message"]
            .as_str()
            .unwrap()
            .contains("already settled")
    );

    let send_call = worker
        .call(
            7,
            "coordination/send",
            json!({
                "runId": run_id.to_string(),
                "senderWorkerId": worker_id.to_string(),
                "taskId": task_id.to_string(),
                "kind": "peerMessage",
                "payload": "too late",
            }),
        )
        .await;
    assert_eq!(send_call["error"]["code"], -32602, "{send_call:?}");

    // Nothing from either rejected call reached the journal.
    let replay = omp
        .call(8, "events/replay", json!({ "afterSequence": 0 }))
        .await;
    let events = replay["result"]
        .as_array()
        .expect("events/replay returns an array");
    assert!(
        !events.iter().any(|e| e["event"]["type"] == "messageEvent"),
        "a call rejected for run settlement must never journal a message: {events:?}"
    );
}

#[tokio::test]
async fn coordination_send_reply_to_must_reference_a_visible_prior_message() {
    let harness = Harness::start().await;
    let mut omp = omp_client(&harness, "omp-1").await;
    let (token, run_id, task_id, worker_id) = seed_scoped_run(&harness, &mut omp).await;
    let mut worker = worker_client(&harness, &token).await;

    let fake_message_id = "018f0000-0000-7000-8000-000000000001";
    let result = worker
        .call(
            2,
            "coordination/send",
            json!({
                "runId": run_id.to_string(),
                "senderWorkerId": worker_id.to_string(),
                "taskId": task_id.to_string(),
                "kind": "answer",
                "payload": "42",
                "replyTo": fake_message_id,
            }),
        )
        .await;
    assert_eq!(result["error"]["code"], -32602, "{result:?}");

    // The real prior message: send once, then reply to it -- must succeed.
    let first = worker
        .call(
            3,
            "coordination/send",
            json!({
                "runId": run_id.to_string(),
                "senderWorkerId": worker_id.to_string(),
                "taskId": task_id.to_string(),
                "kind": "question",
                "payload": "what is the meaning of life?",
            }),
        )
        .await;
    let first_message_id = first["result"]["messageId"].as_str().unwrap();

    let reply = worker
        .call(
            4,
            "coordination/send",
            json!({
                "runId": run_id.to_string(),
                "senderWorkerId": worker_id.to_string(),
                "taskId": task_id.to_string(),
                "kind": "answer",
                "payload": "42",
                "replyTo": first_message_id,
            }),
        )
        .await;
    assert!(reply.get("error").is_none(), "{reply:?}");
}

// -------------------------------------------------------------- rate limit

#[tokio::test]
async fn coordination_send_rate_limits_after_30_messages_per_minute() {
    let harness = Harness::start().await;
    let mut omp = omp_client(&harness, "omp-1").await;
    let (token, run_id, task_id, worker_id) = seed_scoped_run(&harness, &mut omp).await;
    let mut worker = worker_client(&harness, &token).await;

    let mut last = json!(null);
    for i in 0..31 {
        last = worker
            .call(
                2 + i,
                "coordination/send",
                json!({
                    "runId": run_id.to_string(),
                    "senderWorkerId": worker_id.to_string(),
                    "taskId": task_id.to_string(),
                    "kind": "peerMessage",
                    "payload": format!("message {i}"),
                }),
            )
            .await;
    }
    assert_eq!(
        last["error"]["code"], -32006,
        "expected RATE_LIMITED: {last:?}"
    );
}

// ---------------------------------------------------------- scope tokens

#[tokio::test]
async fn worker_mcp_with_an_unknown_token_is_rejected() {
    let harness = Harness::start().await;
    let mut client = Client::connect(&harness.socket).await;
    client
        .send(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "client": { "name": "worker", "version": "0.1.0" },
                "supported": { "min": { "major": 1, "minor": 0 }, "max": { "major": 1, "minor": 0 } },
                "repository": { "canonicalPath": harness.owned_dir, "vcsRoot": harness.owned_dir },
                "auth": { "role": "workerMcp", "instanceId": "worker-1", "scopeToken": "not-a-real-token" },
                "capabilities": { "eventReplay": true, "maxFrameBytes": 1048576 },
                "lastSequence": null
            }
        }))
        .await;
    let response = client.recv().await;
    assert!(response.get("error").is_some(), "{response:?}");
}

#[tokio::test]
async fn a_restarted_mcp_descendant_may_reinitialize_with_the_same_token() {
    let harness = Harness::start().await;
    let mut omp = omp_client(&harness, "omp-1").await;
    let (token, _run_id, _task_id, _worker_id) = seed_scoped_run(&harness, &mut omp).await;

    // First subprocess initializes.
    let first = worker_client(&harness, &token).await;
    drop(first);

    // A "restarted" subprocess reinitializes with the same token while the
    // run, expiry, and vendor process remain live.
    let _second = worker_client(&harness, &token).await;
}

#[tokio::test]
async fn coordination_send_never_writes_the_raw_token_into_the_event_journal() {
    let harness = Harness::start().await;
    let mut omp = omp_client(&harness, "omp-1").await;
    let (token, run_id, task_id, worker_id) = seed_scoped_run(&harness, &mut omp).await;
    let mut worker = worker_client(&harness, &token).await;

    worker
        .call(
            2,
            "coordination/send",
            json!({
                "runId": run_id.to_string(),
                "senderWorkerId": worker_id.to_string(),
                "taskId": task_id.to_string(),
                "kind": "peerMessage",
                "payload": "hello peer",
            }),
        )
        .await;

    // Inspect the durable journal directly: the bearer token string must
    // never appear in any event row.
    let conn = rusqlite::Connection::open(&harness.database).unwrap();
    let mut stmt = conn.prepare("SELECT event_json FROM events").unwrap();
    let rows: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .filter_map(Result::ok)
        .collect();
    for row in &rows {
        assert!(
            !row.contains(&token),
            "token bytes leaked into the event journal: {row}"
        );
    }
}

// --------------------------------------------------------- role gating

#[tokio::test]
async fn omp_extension_cannot_call_worker_safe_coordination_methods() {
    let harness = Harness::start().await;
    let mut omp = omp_client(&harness, "omp-1").await;
    let result = omp
        .call(
            2,
            "coordination/send",
            json!({ "runId": RunId::new().to_string() }),
        )
        .await;
    assert_eq!(result["error"]["code"], -32601, "{result:?}");
}

// ----------------------------------------------------- crash-window sweep

#[tokio::test]
async fn sweep_unacknowledged_as_unknown_settles_recorded_and_sent_messages() {
    let harness = Harness::start().await;
    let mut omp = omp_client(&harness, "omp-1").await;
    let (token, run_id, task_id, worker_id) = seed_scoped_run(&harness, &mut omp).await;
    let mut worker = worker_client(&harness, &token).await;

    let sent = worker
        .call(
            2,
            "coordination/send",
            json!({
                "runId": run_id.to_string(),
                "senderWorkerId": worker_id.to_string(),
                "taskId": task_id.to_string(),
                "kind": "peerMessage",
                "payload": "hello",
            }),
        )
        .await;
    assert_eq!(sent["result"]["deliveryState"], "sent");

    // Simulate crash recovery: a fresh broker over the same database sweeps
    // every non-terminal delivery state to `unknown`, never resending.
    let db = Arc::new(
        DatabaseHandle::start(harness.database.clone())
            .await
            .unwrap(),
    );
    let (events_tx, _events_rx) = broadcast::channel(64);
    let lease_service =
        Arc::new(LeaseService::open_in_memory(ProjectId::new()).expect("in-memory lease service"));
    let broker = CoordinationBroker::new(db, ProjectId::new(), events_tx, lease_service);
    let swept = broker.sweep_unacknowledged_as_unknown().await.unwrap();
    assert_eq!(swept, 1);

    let conn = rusqlite::Connection::open(&harness.database).unwrap();
    let state: String = conn
        .query_row("SELECT delivery_state FROM messages LIMIT 1", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(
        state, "unknown",
        "unacknowledged message must settle at unknown, not acknowledged"
    );
}
