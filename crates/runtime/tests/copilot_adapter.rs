//! Integration tests for the version-gated Copilot ACP adapter.
//!
//! Every test here is a genuine no-model-call structured-protocol check:
//! pure fixture/negotiation/normalization assertions, or a real
//! `copilot --acp` handshake (`initialize`/`session/list`) that never
//! sends a `session/prompt`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use batman_runtime::adapter::copilot::CopilotAdapter;
use batman_runtime::adapter::copilot::client::{
    CopilotAcpClient, CopilotClientEvent, parse_initialize_response,
};
use batman_runtime::adapter::copilot::compatibility::{
    COPILOT_MAX_ACP_PROTOCOL_VERSION, COPILOT_MIN_ACP_PROTOCOL_VERSION,
    copilot_acp_protocol_version_supported, copilot_cli_version_known,
};
use batman_runtime::adapter::copilot::normalize::copilot_normalize_session_update;
use batman_runtime::adapter::{
    Adapter, AdapterCapabilities, AdapterErrorCode, ApprovalsCapability, DurabilityCapability,
    NativeViewCapability, NestedCapability, ProtocolKind, ResumeCapability, SteeringCapability,
    UsageCapability, WorkspaceControlCapability,
};
use serde_json::Value;
use tokio::time::timeout;

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/adapters/copilot")
        .join(name)
}

fn load_json_fixture(name: &str) -> Value {
    let path = fixture_path(name);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading fixture {}: {e}", path.display()));
    serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("parsing fixture {}: {e}", path.display()))
}

fn load_jsonl_fixture(name: &str) -> Vec<Value> {
    let path = fixture_path(name);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading fixture {}: {e}", path.display()));
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str(line).unwrap_or_else(|e| panic!("parsing {name} line: {e}"))
        })
        .collect()
}

fn real_copilot_binary() -> Option<PathBuf> {
    let output = Command::new("which").arg("copilot").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let path = String::from_utf8(output.stdout).ok()?.trim().to_string();
    if path.is_empty() {
        None
    } else {
        Some(PathBuf::from(path))
    }
}

// ------------------------------------------------------- compatibility.rs

#[test]
fn known_cli_version_is_exact_match_against_both_empirically_verified_versions() {
    // 1.0.73 was installed when this work started; the CLI's own
    // background auto-updater (`copilot update`) updated it to 1.0.75
    // mid-development, and 1.0.75 was reprobed and confirmed to
    // negotiate the same ACP v1 shape (see `compatibility.rs`'s module
    // doc). Both are exact-match known versions; nothing else is.
    assert!(copilot_cli_version_known("1.0.73"));
    assert!(copilot_cli_version_known("1.0.75"));
    assert!(!copilot_cli_version_known("1.0.74"));
    assert!(!copilot_cli_version_known("1.0.7"));
    assert!(!copilot_cli_version_known(""));
}

#[test]
fn only_acp_protocol_v1_is_supported() {
    assert_eq!(COPILOT_MIN_ACP_PROTOCOL_VERSION, 1);
    assert_eq!(COPILOT_MAX_ACP_PROTOCOL_VERSION, 1);
    assert!(copilot_acp_protocol_version_supported(1));
    assert!(!copilot_acp_protocol_version_supported(0));
    assert!(!copilot_acp_protocol_version_supported(2));
}

// ---------------------------------------------------- initialize negotiation

#[test]
fn real_1_0_73_fixture_negotiates_protocol_v1_and_v1_field_names() {
    let response = load_json_fixture("initialize-v1.json");
    let result = response
        .get("result")
        .expect("fixture is a JSON-RPC response with a result");
    let negotiated =
        parse_initialize_response(result).expect("a v1-shaped initialize response parses");

    assert_eq!(negotiated.protocol_version, 1);
    assert_eq!(negotiated.agent_version.as_deref(), Some("1.0.73"));
    // v1 field names read directly off the real observed response:
    // `agentCapabilities.loadSession`, `.mcpCapabilities.{http,sse}`,
    // `.promptCapabilities.{image,embeddedContext}`,
    // `.sessionCapabilities.list` -- never v2 names (`tools`, etc.).
    assert!(negotiated.load_session);
    assert!(negotiated.session_list);
    assert!(negotiated.mcp_http);
    assert!(negotiated.mcp_sse);
    assert!(negotiated.image);
    assert!(negotiated.embedded_context);
}

#[test]
fn an_unsupported_negotiated_protocol_version_is_refused_as_incompatible() {
    let mut response = load_json_fixture("initialize-v1.json");
    response["result"]["protocolVersion"] = Value::from(2);
    let result = response.get("result").unwrap();

    let error =
        parse_initialize_response(result).expect_err("an unsupported protocol version must fail");
    assert_eq!(error.error_code(), AdapterErrorCode::IncompatibleVersion);
}

#[test]
fn a_response_missing_protocol_version_is_a_protocol_error() {
    let error = parse_initialize_response(&serde_json::json!({}))
        .expect_err("a response missing protocolVersion must fail");
    assert_eq!(error.error_code(), AdapterErrorCode::Protocol);
}

// --------------------------------------------------------------- no TCP ever

#[test]
fn copilot_acp_client_source_never_constructs_a_port_argument() {
    // Structural proof, not just a runtime debug_assert: the only place
    // the literal `--port` appears anywhere in client.rs is the defensive
    // guard that *refuses* it (`debug_assert!` comparing argv tokens
    // against it) -- whitelisted verbatim below and stripped before the
    // scan, so any other occurrence (a real construction site) fails
    // this test.
    let source = include_str!("../src/adapter/copilot/client.rs");
    let code_only: String = source
        .lines()
        .map(str::trim_start)
        .filter(|line| !line.starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    let guard_start = code_only
        .find("debug_assert!(")
        .expect("client.rs must contain the --port debug_assert guard");
    let guard_end = code_only[guard_start..]
        .find(");")
        .map(|offset| guard_start + offset + 2)
        .expect("the debug_assert guard must be terminated");
    let guard_block = &code_only[guard_start..guard_end];
    assert!(
        guard_block.contains("--port"),
        "the debug_assert block right after `--acp` argv construction must be the --port guard, found: {guard_block}"
    );
    let sanitized = format!("{}{}", &code_only[..guard_start], &code_only[guard_end..]);
    assert!(
        !sanitized.contains("--port"),
        "client.rs must never construct a --port argv token outside the whitelisted debug_assert guard"
    );
}

#[tokio::test]
async fn real_binary_port_zero_opens_no_tcp_listener() {
    let Some(copilot) = real_copilot_binary() else {
        eprintln!("skipping: `copilot` is not on PATH");
        return;
    };
    // Deliberately probes the REAL binary's own `--port` flag directly
    // (bypassing `CopilotAcpClient`, which never builds this flag) to
    // prove the installed 1.0.73 CLI itself opens no TCP listener for
    // it -- the empirical fact `client.rs`'s module doc relies on.
    let mut child = tokio::process::Command::new(&copilot)
        .args(["--acp", "--port", "0"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawning the real copilot binary");
    let pid = child
        .id()
        .expect("pid observable for a freshly spawned child");

    tokio::time::sleep(Duration::from_millis(750)).await;

    let lsof = Command::new("lsof")
        .args(["-a", "-p", &pid.to_string(), "-i", "TCP", "-sTCP:LISTEN"])
        .output();
    let _ = child.kill().await;
    let _ = child.wait().await;

    match lsof {
        Ok(output) => {
            let listing = String::from_utf8_lossy(&output.stdout);
            assert!(
                listing.trim().is_empty(),
                "the real copilot --acp --port 0 binary must open no TCP listener, found: {listing}"
            );
        }
        Err(e) => eprintln!("skipping listener assertion: lsof unavailable ({e})"),
    }
}

// -------------------------------------------------- normalize.rs (fixtures)

#[test]
fn session_updates_fixture_normalizes_every_variant_correctly() {
    let updates: Vec<Value> = load_jsonl_fixture("session-updates.jsonl")
        .into_iter()
        .map(|frame| frame["params"]["update"].clone())
        .collect();
    assert_eq!(updates.len(), 10);

    // 1. user_message_chunk -> visible MessageChunk{role: "user"}.
    let payloads = copilot_normalize_session_update(&updates[0]);
    assert_eq!(payloads.len(), 1);
    match &payloads[0] {
        batman_runtime::adapter::AdapterEventPayload::MessageChunk { role, text } => {
            assert_eq!(role, "user");
            assert_eq!(text.class, batman_protocol::ContentClass::Visible);
            assert_eq!(text.value, "Fix the failing assertion in adapter.rs");
        }
        other => panic!("expected MessageChunk, got {other:?}"),
    }

    // 2. agent_thought_chunk -> dropped before it ever becomes an event.
    assert!(copilot_normalize_session_update(&updates[1]).is_empty());

    // 3. agent_message_chunk -> visible MessageChunk{role: "assistant"}.
    let payloads = copilot_normalize_session_update(&updates[2]);
    match &payloads[0] {
        batman_runtime::adapter::AdapterEventPayload::MessageChunk { role, .. } => {
            assert_eq!(role, "assistant");
        }
        other => panic!("expected MessageChunk, got {other:?}"),
    }

    // 4. tool_call -> ToolStarted.
    let payloads = copilot_normalize_session_update(&updates[3]);
    match &payloads[0] {
        batman_runtime::adapter::AdapterEventPayload::ToolStarted { tool_call_id, name } => {
            assert_eq!(tool_call_id, "call-1");
            assert_eq!(name, "Read adapter.rs");
        }
        other => panic!("expected ToolStarted, got {other:?}"),
    }

    // 5. tool_call_update{status: completed} -> ToolResult{ok: true}.
    let payloads = copilot_normalize_session_update(&updates[4]);
    match &payloads[0] {
        batman_runtime::adapter::AdapterEventPayload::ToolResult { ok, detail, .. } => {
            assert!(ok);
            assert_eq!(detail.value, "fn main() {}");
        }
        other => panic!("expected ToolResult, got {other:?}"),
    }

    // 7. tool_call_update{status: in_progress} -> ToolProgress.
    let payloads = copilot_normalize_session_update(&updates[6]);
    match &payloads[0] {
        batman_runtime::adapter::AdapterEventPayload::ToolProgress { tool_call_id, .. } => {
            assert_eq!(tool_call_id, "call-2");
        }
        other => panic!("expected ToolProgress, got {other:?}"),
    }

    // 8. tool_call_update{status: completed, content: [diff]} -> ToolResult
    //    whose detail names only the path, never the old/new file text.
    let payloads = copilot_normalize_session_update(&updates[7]);
    match &payloads[0] {
        batman_runtime::adapter::AdapterEventPayload::ToolResult { ok, detail, .. } => {
            assert!(ok);
            assert_eq!(detail.value, "diff: /workspace/adapter.rs");
            assert!(!detail.value.contains("assert_eq"));
        }
        other => panic!("expected ToolResult, got {other:?}"),
    }

    // 10. tool_call_update{status: failed} -> ToolResult{ok: false}.
    let payloads = copilot_normalize_session_update(&updates[9]);
    match &payloads[0] {
        batman_runtime::adapter::AdapterEventPayload::ToolResult { ok, detail, .. } => {
            assert!(!ok);
            assert_eq!(detail.value, "permission denied");
        }
        other => panic!("expected ToolResult, got {other:?}"),
    }
}

#[test]
fn an_unrecognized_session_update_variant_normalizes_to_no_events() {
    let update = serde_json::json!({ "sessionUpdate": "plan", "entries": [] });
    assert!(copilot_normalize_session_update(&update).is_empty());
}

// ------------------------------------------------------------- capabilities

#[test]
fn declared_capabilities_match_exactly_what_this_adapter_tests() {
    let adapter = CopilotAdapter::new(
        PathBuf::from("copilot"),
        std::env::temp_dir(),
        batman_runtime::adapter::CopilotStartupOptions::default(),
        Vec::new(),
        batman_protocol::RunId::new(),
        batman_protocol::TaskId::new(),
        batman_protocol::WorkerId::new(),
    );
    let capabilities: AdapterCapabilities = adapter.capabilities();
    assert_eq!(capabilities.protocol, ProtocolKind::Structured);
    // Proven by `real_1_0_73_fixture_negotiates_protocol_v1_and_v1_field_names`
    // observing `agentCapabilities.loadSession: true`, and
    // `session_load`/`session/load` being exercised in `Adapter::resume`.
    assert_eq!(capabilities.resume, ResumeCapability::Session);
    // ACP v1 has no mid-turn steering distinct from a follow-up prompt
    // after a turn ends; never tested against a real turn (that would be
    // a model call), so declared `None`, not assumed.
    assert_eq!(capabilities.steering, SteeringCapability::None);
    // Proven by the `permission.jsonl`-driven
    // `respond_permission_answers_a_real_pending_request_over_the_wire`
    // test: this adapter both observes AND resolves a real
    // `session/request_permission` request.
    assert_eq!(capabilities.approvals, ApprovalsCapability::Controllable);
    assert!(!capabilities.structured_result);
    // ACP v1's `PromptResponse` carries only a `stopReason`, never a
    // token/cost usage object -- absent from the protocol, not merely
    // untested.
    assert_eq!(capabilities.usage, UsageCapability::None);
    assert_eq!(capabilities.nested, NestedCapability::None);
    assert_eq!(capabilities.native_view, NativeViewCapability::None);
    // Proven by the `session-updates.jsonl` `edit`-kind tool call
    // (`call-2`)'s `diff` content normalizing into a `ToolResult`.
    assert_eq!(
        capabilities.workspace_control,
        WorkspaceControlCapability::Write
    );
    // Proven by `agentCapabilities.loadSession: true` plus real
    // historical sessions returned from a live `session/list` probe.
    assert_eq!(
        capabilities.durability,
        DurabilityCapability::VendorResumable
    );
}

// -------------------------------------------------- real installed binary

#[tokio::test]
async fn real_binary_initialize_and_session_list_never_invoke_a_model() {
    let Some(copilot) = real_copilot_binary() else {
        eprintln!("skipping: `copilot` is not on PATH");
        return;
    };

    let client = timeout(
        Duration::from_secs(10),
        CopilotAcpClient::spawn(&copilot, Path::new("."), Vec::new(), HashMap::new()),
    )
    .await
    .expect("spawning the real copilot --acp binary did not hang")
    .expect("spawning the real copilot --acp binary");

    let negotiated = timeout(Duration::from_secs(10), client.initialize())
        .await
        .expect("initialize did not hang")
        .expect("a real handshake with the installed binary succeeds");
    assert_eq!(negotiated.protocol_version, 1);
    // The installed binary can auto-update itself between test runs
    // (observed 1.0.73 -> 1.0.75 mid-development); assert it is a
    // version this adapter has empirically verified rather than pinning
    // one exact string that will go stale.
    let observed_version = negotiated
        .agent_version
        .as_deref()
        .expect("agentInfo.version present");
    assert!(
        copilot_cli_version_known(observed_version),
        "installed copilot CLI {observed_version} is not in COPILOT_KNOWN_CLI_VERSIONS; \
         reprobe and add it after confirming it negotiates the same ACP v1 shape"
    );

    // A real, no-model-call structured probe: `session/list` only reads
    // Copilot's own persisted session metadata, never a prompt/model
    // call.
    let sessions = timeout(Duration::from_secs(10), client.session_list())
        .await
        .expect("session/list did not hang")
        .expect("session/list succeeds against an authenticated installed CLI");
    assert!(
        sessions.get("sessions").and_then(Value::as_array).is_some(),
        "session/list response must carry a `sessions` array, got: {sessions}"
    );

    client.shutdown().await;
}

// ------------------------------------------------------- permission flow

#[tokio::test]
async fn respond_permission_answers_a_real_pending_request_over_the_wire() {
    let fixture = load_jsonl_fixture("permission.jsonl");
    let request_line = serde_json::to_string(&fixture[0]).unwrap();
    let expected_response = fixture[1].clone();

    let output_dir =
        std::env::temp_dir().join(format!("copilot-adapter-test-{}", std::process::id()));
    std::fs::create_dir_all(&output_dir).unwrap();
    let output_path = output_dir.join("response.json");
    let _ = std::fs::remove_file(&output_path);

    // A fake ACP agent: emits the fixture's real `session/request_permission`
    // request immediately, then writes whatever this client answers with
    // to a file this test can inspect.
    let script = format!(
        "cat <<'ACPEOF'\n{request_line}\nACPEOF\nread -r resp\nprintf '%s' \"$resp\" > {}\n",
        output_path.display()
    );

    let client = CopilotAcpClient::spawn_with_raw_args(
        Path::new("/bin/sh"),
        Path::new("."),
        vec!["-c".to_string(), script],
        HashMap::new(),
    )
    .await
    .expect("spawning the fake ACP agent");

    let event = timeout(Duration::from_secs(5), client.next_event())
        .await
        .expect("the fake agent's permission request arrives promptly")
        .expect("the reader task is still alive");

    let (request_id, request) = match event {
        CopilotClientEvent::PermissionRequested {
            request_id,
            request,
        } => (request_id, request),
        other => panic!("expected PermissionRequested, got {other:?}"),
    };
    assert_eq!(request_id, 42);
    assert_eq!(request.session_id, "sess-1");
    assert_eq!(request.tool_call_id, "call-2");
    assert_eq!(request.options.len(), 2);
    assert_eq!(client.pending_permission_ids(), vec![42]);

    client
        .respond_permission(request_id, "allow-once")
        .expect("answering a real pending permission request");
    assert!(client.pending_permission_ids().is_empty());

    // Answering an already-answered request is an explicit error, never
    // a silent no-op.
    let error = client
        .respond_permission(request_id, "allow-once")
        .expect_err("responding twice to the same request must fail");
    assert_eq!(error.error_code(), AdapterErrorCode::InvalidVendorState);

    let written = timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(text) = std::fs::read_to_string(&output_path) {
                if !text.is_empty() {
                    return text;
                }
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the fake agent wrote a response before this test gave up");

    client.shutdown().await;

    let actual: Value =
        serde_json::from_str(&written).expect("the response this client sent is valid JSON");
    assert_eq!(actual, expected_response);

    let _ = std::fs::remove_dir_all(&output_dir);
}
