//! Integration tests for the Codex `app-server` worker adapter: schema
//! compatibility against the real installed `codex-cli` binary, fixture
//! normalization of a realistic thread/turn transcript (text, tool,
//! usage, and artifact events, all correlated to one run, with hidden
//! `reasoning` content dropped before it ever reaches an event), and
//! approval-request normalization from a dedicated fixture.
//!
//! `crates/runtime/src/adapter/mod.rs` does not yet declare `mod codex;`
//! (Task 4 develops in parallel with three sibling adapter tasks; the
//! orchestrator wires every adapter's `mod`/`pub use` in one pass after
//! all four land -- see this adapter's final summary for the exact two
//! lines). Until then, this test file pulls the adapter's source in
//! directly via `#[path]` and provides tiny shim modules so its internal
//! `crate::adapter::*`/`crate::supervisor::*` paths -- which will resolve
//! against `batman_runtime`'s own module tree once wired -- resolve here
//! against `batman_runtime`'s public re-exports instead. Once wired, this
//! `#[path]`/shim scaffold becomes dead weight the orchestrator should
//! delete in the same pass (the adapter module will then live at
//! `batman_runtime::adapter::codex` and this file should switch to a
//! plain `use batman_runtime::adapter::CodexAdapter;`).

mod adapter {
    pub use batman_runtime::adapter::*;
}
mod supervisor {
    pub use batman_runtime::supervisor::*;
}

#[path = "../src/adapter/codex/mod.rs"]
mod codex;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use batman_protocol::{ContentClass, RunId, TaskId, WorkerId};
use batman_runtime::adapter::{
    Adapter, AdapterEvent, AdapterEventPayload, AdapterEventSink, AdapterFuture,
    CodexStartupOptions,
};
use serde_json::Value;

use codex::normalize;
use codex::schema::{SchemaManifest, verify_against_installed_binary};

// --------------------------------------------------------------- fixtures

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/adapters/codex")
        .join(name)
}

fn read_jsonl(name: &str) -> Vec<Value> {
    let raw = std::fs::read_to_string(fixture_path(name))
        .unwrap_or_else(|e| panic!("failed to read fixture {name}: {e}"));
    raw.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str(line).unwrap_or_else(|e| panic!("bad fixture line {line:?}: {e}"))
        })
        .collect()
}

// ------------------------------------------------------------ recording sink

/// An in-memory [`AdapterEventSink`] that records every emitted event, so
/// fixture tests can assert on correlation and payload shape without a
/// real domain/journal/broadcast stack.
#[derive(Default)]
struct RecordingSink {
    events: Mutex<Vec<AdapterEvent>>,
}

impl AdapterEventSink for RecordingSink {
    fn emit(&self, event: AdapterEvent) -> AdapterFuture<'_, u64> {
        Box::pin(async move {
            let mut events = self
                .events
                .lock()
                .expect("recording sink mutex never poisoned");
            events.push(event);
            Ok(events.len() as u64)
        })
    }
}

// --------------------------------------------------------------- Step 1

#[test]
fn schema_manifest_required_surface_is_present_on_the_installed_binary() {
    let manifest = SchemaManifest::load(&fixture_path("schema-version.json"))
        .expect("committed schema-version.json manifest must parse");
    verify_against_installed_binary(&manifest, "codex")
        .expect("installed codex-cli 0.145.0 app-server schema must still cover this adapter's required surface");
}

#[test]
fn fixture_thread_turn_transcript_normalizes_to_correlated_events() {
    let lines = read_jsonl("thread-turn.jsonl");
    let run_id = RunId::new();
    let task_id = TaskId::new();
    let worker_id = WorkerId::new();

    let mut payloads = Vec::new();
    for line in &lines {
        let method = line
            .get("method")
            .and_then(Value::as_str)
            .expect("fixture line has method");
        let params = line.get("params").cloned().unwrap_or(Value::Null);
        if let Some(payload) = normalize::notification_to_event(method, &params) {
            payloads.push(payload);
        }
    }

    let events: Vec<AdapterEvent> = payloads
        .into_iter()
        .map(|payload| AdapterEvent {
            run_id,
            task_id,
            worker_id,
            payload,
        })
        .collect();

    // Every event this turn produced correlates to the same run/task/worker.
    assert!(
        !events.is_empty(),
        "the fixture transcript must normalize to at least one event"
    );
    for event in &events {
        assert_eq!(event.run_id, run_id);
        assert_eq!(event.task_id, task_id);
        assert_eq!(event.worker_id, worker_id);
    }

    let has = |pred: fn(&AdapterEventPayload) -> bool| events.iter().any(|e| pred(&e.payload));

    assert!(
        has(|p| matches!(p, AdapterEventPayload::MessageChunk { .. })),
        "expected at least one MessageChunk (item/agentMessage/delta)"
    );
    assert!(
        has(|p| matches!(p, AdapterEventPayload::MessageFinal { role, .. } if role == "assistant")),
        "expected a MessageFinal for the completed agentMessage item"
    );
    assert!(
        has(
            |p| matches!(p, AdapterEventPayload::ToolStarted { name, .. } if name == "commandExecution")
        ),
        "expected a ToolStarted for the commandExecution item"
    );
    assert!(
        has(|p| matches!(p, AdapterEventPayload::ToolResult { ok: true, .. })),
        "expected a successful ToolResult for the completed commandExecution item"
    );
    assert!(
        has(|p| matches!(
            p,
            AdapterEventPayload::UsageReported {
                input_tokens: 1200,
                output_tokens: 180,
                ..
            }
        )),
        "expected UsageReported from thread/tokenUsage/updated matching the fixture's token counts"
    );
    assert!(
        has(
            |p| matches!(p, AdapterEventPayload::ArtifactProduced { artifact_kind, .. } if artifact_kind == "fileChange")
        ),
        "expected ArtifactProduced for the completed fileChange item"
    );

    // The `reasoning` item's hidden chain-of-thought must never surface as
    // a MessageChunk/MessageFinal (or in any other visible text field).
    for event in &events {
        if let AdapterEventPayload::MessageChunk { text, .. }
        | AdapterEventPayload::MessageFinal { text, .. } = &event.payload
        {
            assert_eq!(text.class, ContentClass::Visible);
            assert!(!text.value.contains("chain of thought"));
        }
    }
}

#[test]
fn fixture_approvals_normalize_to_pending_approvals_not_sink_events() {
    let lines = read_jsonl("approval.jsonl");
    assert_eq!(lines.len(), 2);

    let mut kinds = Vec::new();
    for line in &lines {
        let id = line
            .get("id")
            .cloned()
            .expect("approval fixture line has id");
        let method = line
            .get("method")
            .and_then(Value::as_str)
            .expect("fixture line has method");
        let params = line.get("params").cloned().unwrap_or(Value::Null);
        let approval = normalize::server_request_to_pending_approval(&id, method, &params)
            .unwrap_or_else(|| panic!("expected {method} to normalize to a pending approval"));
        assert_eq!(approval.request_id, id);
        assert!(!approval.call_id.is_empty());
        kinds.push(approval.kind);
    }
    assert_eq!(kinds, vec!["execCommand", "applyPatch"]);
}

#[test]
fn decision_mapping_matches_the_verified_review_decision_shape() {
    assert_eq!(
        normalize::decision_to_review_decision("approve").unwrap(),
        Value::String("approved".to_string())
    );
    let denied = normalize::decision_to_review_decision("deny").unwrap();
    assert!(denied.get("denied").is_some());
    assert!(normalize::decision_to_review_decision("nonsense").is_err());
}

// --------------------------------------------------------------- Step 3/4

#[tokio::test]
async fn capabilities_match_the_verified_protocol_surface() {
    let adapter = codex::CodexAdapter::new(
        std::env::temp_dir(),
        CodexStartupOptions::default(),
        Vec::new(),
    );
    let caps = adapter.capabilities();
    assert_eq!(
        caps.protocol,
        batman_runtime::adapter::ProtocolKind::Structured
    );
    assert_eq!(
        caps.resume,
        batman_runtime::adapter::ResumeCapability::Session
    );
    assert_eq!(
        caps.steering,
        batman_runtime::adapter::SteeringCapability::ActiveTurn
    );
    assert_eq!(
        caps.approvals,
        batman_runtime::adapter::ApprovalsCapability::Controllable
    );
    assert_eq!(
        caps.usage,
        batman_runtime::adapter::UsageCapability::PerTurn
    );
    assert_eq!(caps.nested, batman_runtime::adapter::NestedCapability::None);
    assert_eq!(
        caps.workspace_control,
        batman_runtime::adapter::WorkspaceControlCapability::Write
    );
    assert_eq!(
        caps.durability,
        batman_runtime::adapter::DurabilityCapability::VendorResumable
    );
}

#[tokio::test]
async fn probe_reports_the_installed_codex_version_without_a_model_call() {
    let adapter = codex::CodexAdapter::with_binary(
        "codex",
        std::env::temp_dir(),
        CodexStartupOptions::default(),
        Vec::new(),
    );
    let probe = adapter
        .probe()
        .await
        .expect("probe against the installed codex-cli must succeed");
    assert!(
        probe
            .version
            .as_deref()
            .unwrap_or_default()
            .contains("codex-cli")
    );
}
#[tokio::test]
async fn real_transport_completes_initialize_and_thread_start_with_zero_model_calls() {
    // Exercises `CodexRpcClient` directly against a real spawned
    // `codex app-server` process: the `initialize` request, the
    // `initialized` notification, and a bare `thread/start` (session
    // creation only, no `input`/turn). None of these three methods ever
    // reach the model -- Codex only calls out to the model once a turn
    // actually starts with input (`turn/start`), which this test
    // deliberately never issues.
    let current_env: std::collections::HashMap<String, String> = std::env::vars().collect();
    let env = supervisor::EnvironmentPolicy::baseline().build(&current_env, &[]);
    let spec = supervisor::SpawnSpec {
        program: PathBuf::from("codex"),
        args: vec!["app-server".to_string()],
        cwd: std::env::temp_dir(),
        env,
        ..supervisor::SpawnSpec::minimal()
    };
    let process = supervisor::Supervisor::new()
        .spawn(spec)
        .await
        .expect("spawning the real installed codex app-server must succeed");
    let (client, _inbound_rx) = codex::client::CodexRpcClient::spawn(process);

    let init = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        client.call(
            "initialize",
            serde_json::json!({
                "clientInfo": {"name": "@satori/batman", "version": "0.0.0-test"},
                "capabilities": {"experimentalApi": true}
            }),
        ),
    )
    .await
    .expect("initialize must not hang")
    .expect("initialize must succeed against the real installed binary");
    assert!(
        init.get("userAgent").is_some(),
        "InitializeResponse must carry userAgent"
    );

    client
        .notify("initialized", serde_json::json!({}))
        .expect("initialized notification must send");

    let thread = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        client.call(
            "thread/start",
            serde_json::json!({"cwd": std::env::temp_dir().to_string_lossy()}),
        ),
    )
    .await
    .expect("thread/start must not hang")
    .expect("thread/start must succeed against the real installed binary");
    assert!(
        thread.get("thread").and_then(|t| t.get("id")).is_some(),
        "ThreadStartResponse must carry thread.id"
    );

    client
        .terminate()
        .await
        .expect("terminating the real app-server process must succeed");
    // `shutdown` is a separate, idempotent hard-stop escape hatch (abort
    // the driver task outright, without a graceful process wait) --
    // exercised here to prove it never panics once the driver has
    // already exited on its own.
    client.shutdown();
}

/// Exercises the full [`Adapter::start`] lifecycle -- including
/// `turn/start`, which genuinely does invoke the model once Codex begins
/// working the turn -- against a real authenticated Codex account.
/// **Never run this in CI or by an agent**: it is gated behind
/// `BATMAN_LIVE_CODEX=1` and `#[ignore]`d by default specifically because
/// it is the one path in this test file that is not free of model calls.
/// A human wanting to exercise it locally: `BATMAN_LIVE_CODEX=1 cargo test
/// -p batman-runtime --test codex_adapter -- --ignored
/// live_start_actually_runs_a_turn_against_a_real_model`.
#[tokio::test]
#[ignore = "invokes a real model turn against an authenticated Codex account; human-run only, see doc comment"]
async fn live_start_actually_runs_a_turn_against_a_real_model() {
    assert_eq!(
        std::env::var("BATMAN_LIVE_CODEX").as_deref(),
        Ok("1"),
        "set BATMAN_LIVE_CODEX=1 to opt into this live, model-invoking test"
    );
    let adapter = codex::CodexAdapter::new(
        std::env::temp_dir(),
        CodexStartupOptions::default(),
        Vec::new(),
    );
    let sink: Arc<dyn AdapterEventSink> = Arc::new(RecordingSink::default());
    let spec = batman_runtime::adapter::StartSpec {
        run_id: RunId::new(),
        task_id: TaskId::new(),
        worker_id: WorkerId::new(),
        prompt: "reply with exactly the word done".to_string(),
        resume: None,
    };
    adapter
        .start(spec, sink)
        .await
        .expect("live start must succeed");
    adapter.dispose().await.expect("dispose must succeed");
}
