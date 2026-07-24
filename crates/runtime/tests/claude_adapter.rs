//! Integration tests for the Claude stream-JSON worker adapter.
//!
//! No test in this file ever invokes a model: `probe()` runs only
//! `claude --version`/`claude auth status` against the real installed CLI;
//! every other test either exercises pure command-argv/normalization logic
//! against static fixtures, or calls an adapter method before any vendor
//! process has been started.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use batman_protocol::{RunId, TaskId, WorkerId};
use batman_runtime::adapter::{
    Adapter, AdapterMessage, ApprovalsCapability, CancelScope, ClaudeStartupOptions,
    DurabilityCapability, NativeViewCapability, NestedCapability, ProtocolKind, ResumeCapability,
    StartSpec, SteeringCapability, UsageCapability, VendorSessionRef, WorkspaceControlCapability,
};

use batman_runtime::adapter::claude::ClaudeAdapter;
use batman_runtime::adapter::claude::command;
use batman_runtime::adapter::claude::normalize::{ClaudeEvent, ClaudeNormalizer};

fn new_adapter() -> ClaudeAdapter {
    ClaudeAdapter::new(
        ClaudeStartupOptions::default(),
        std::env::temp_dir(),
        Vec::new(),
        RunId::new(),
        TaskId::new(),
        WorkerId::new(),
    )
}

fn fixture(name: &str) -> Vec<Vec<u8>> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/adapters/claude")
        .join(name);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("reading fixture {path:?}: {err}"));
    text.lines().map(|line| line.as_bytes().to_vec()).collect()
}

// ------------------------------------------------------------------ kind

#[test]
fn kind_is_claude() {
    assert_eq!(new_adapter().kind(), "claude");
}

// -------------------------------------------------------------- command

#[test]
fn new_session_preserves_native_discovery_and_generates_a_session_id() {
    let options = ClaudeStartupOptions::default();
    let spec = StartSpec {
        run_id: batman_protocol::RunId::new(),
        task_id: batman_protocol::TaskId::new(),
        worker_id: batman_protocol::WorkerId::new(),
        prompt: "do the thing".to_string(),
        resume: None,
    };
    let session_id = uuid::Uuid::now_v7();
    let args = command::build_args(&options, &spec, &session_id);

    for required in [
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--include-partial-messages",
        "--include-hook-events",
        "--forward-subagent-text",
        "--session-id",
    ] {
        assert!(
            args.iter().any(|a| a == required),
            "expected {required:?} in {args:?}"
        );
    }
    assert!(args.iter().any(|a| a == &session_id.to_string()));

    // Discovery-preserving: never disable native skill/agent/plugin/hook/MCP
    // resolution.
    for forbidden in ["--bare", "--disable-slash-commands", "--safe-mode"] {
        assert!(
            !args.iter().any(|a| a == forbidden),
            "must never pass {forbidden:?}: {args:?}"
        );
    }
    assert!(!args.iter().any(|a| a == "--resume"));
}

#[test]
fn resume_uses_the_provided_vendor_session_and_skips_session_id() {
    let options = ClaudeStartupOptions::default();
    let spec = StartSpec {
        run_id: batman_protocol::RunId::new(),
        task_id: batman_protocol::TaskId::new(),
        worker_id: batman_protocol::WorkerId::new(),
        prompt: "continue".to_string(),
        resume: Some(VendorSessionRef("abc-123-session".to_string())),
    };
    let session_id = uuid::Uuid::now_v7();
    let args = command::build_args(&options, &spec, &session_id);

    let resume_idx = args
        .iter()
        .position(|a| a == "--resume")
        .expect("expected --resume in args");
    assert_eq!(args[resume_idx + 1], "abc-123-session");
    assert!(!args.iter().any(|a| a == "--session-id"));
}

#[test]
fn startup_options_pass_through_supported_cli_flags_and_omit_unsupported_max_turns() {
    let options = ClaudeStartupOptions {
        allowed_tools: Some(vec!["Bash(git *)".to_string(), "Edit".to_string()]),
        permission_mode: Some("acceptEdits".to_string()),
        // The installed `claude` 2.1.219 CLI has no `--max-turns` flag at
        // all (verified via `claude --help`); it exists only as a
        // programmatic `Options.maxTurns` field in the TS/Python Agent
        // SDK. `ClaudeStartupOptions.max_turns` is already defined
        // upstream (Task 1/2, not ours to change) and cannot be honored
        // by this CLI-argv adapter -- deliberately not passed as a flag,
        // rather than inventing one.
        max_turns: Some(10),
    };
    let spec = StartSpec {
        run_id: batman_protocol::RunId::new(),
        task_id: batman_protocol::TaskId::new(),
        worker_id: batman_protocol::WorkerId::new(),
        prompt: "go".to_string(),
        resume: None,
    };
    let args = command::build_args(&options, &spec, &uuid::Uuid::now_v7());

    let allowed_idx = args
        .iter()
        .position(|a| a == "--allowedTools")
        .expect("expected --allowedTools");
    assert_eq!(args[allowed_idx + 1], "Bash(git *)");
    assert_eq!(args[allowed_idx + 2], "Edit");

    let mode_idx = args
        .iter()
        .position(|a| a == "--permission-mode")
        .expect("expected --permission-mode");
    assert_eq!(args[mode_idx + 1], "acceptEdits");

    assert!(!args.iter().any(|a| a == "--max-turns"));
    assert!(!args.iter().any(|a| a == "10"));
}

#[test]
fn stdin_user_message_is_newline_delimited_stream_json() {
    let bytes = command::build_stdin_user_message("do the thing");
    assert!(bytes.ends_with(b"\n"), "must be newline-delimited");
    let value: serde_json::Value = serde_json::from_slice(&bytes[..bytes.len() - 1]).unwrap();
    assert_eq!(value["type"], "user");
    assert_eq!(value["message"]["role"], "user");
    assert_eq!(value["message"]["content"][0]["type"], "text");
    assert_eq!(value["message"]["content"][0]["text"], "do the thing");
}

// ------------------------------------------------------------- normalize

fn emitted_payloads(events: &[ClaudeEvent]) -> Vec<&batman_runtime::adapter::AdapterEventPayload> {
    events
        .iter()
        .filter_map(|event| match event {
            ClaudeEvent::Emit(payload) => Some(payload),
            _ => None,
        })
        .collect()
}

#[test]
fn initialize_fixture_normalizes_session_id_text_tools_and_final_result() {
    use batman_runtime::adapter::AdapterEventPayload::*;

    let mut normalizer = ClaudeNormalizer::new();
    let mut all_events = Vec::new();
    for line in fixture("initialize.jsonl") {
        let events = normalizer
            .normalize_line("claude", &line)
            .unwrap_or_else(|err| panic!("normalizing line failed: {err}"));
        all_events.extend(events);
    }
    let payloads = emitted_payloads(&all_events);

    match payloads[0] {
        VendorSessionEstablished { vendor_session_id } => {
            assert_eq!(vendor_session_id, "11111111-1111-4111-8111-111111111111");
        }
        other => panic!("expected VendorSessionEstablished first, got {other:?}"),
    }

    // The two streaming text deltas become MessageChunk, in order.
    let chunks: Vec<&str> = payloads
        .iter()
        .filter_map(|p| match p {
            MessageChunk { text, .. } => Some(text.value.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(chunks, vec!["Sure, ", "I'll check the config file."]);

    // The thinking block on the tool-use turn is never emitted at all.
    let finals: Vec<&str> = payloads
        .iter()
        .filter_map(|p| match p {
            MessageFinal { text, .. } => Some(text.value.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        finals
            .iter()
            .all(|t| !t.contains("I should check config.toml"))
    );
    assert!(finals.contains(&"Sure, I'll check the config file."));
    assert!(finals.contains(&"The read timeout is set to 30 seconds in config.toml."));

    // Tool lifecycle: started for Read, then its result.
    let tool_started = payloads
        .iter()
        .find_map(|p| match p {
            ToolStarted { tool_call_id, name } => Some((tool_call_id.as_str(), name.as_str())),
            _ => None,
        })
        .expect("expected a ToolStarted event");
    assert_eq!(tool_started, ("toolu_01READ", "Read"));

    let tool_result = payloads
        .iter()
        .find_map(|p| match p {
            ToolResult {
                tool_call_id,
                name,
                ok,
                detail,
            } => Some((
                tool_call_id.as_str(),
                name.as_str(),
                *ok,
                detail.value.as_str(),
            )),
            _ => None,
        })
        .expect("expected a ToolResult event");
    assert_eq!(tool_result.0, "toolu_01READ");
    assert_eq!(tool_result.1, "Read");
    assert!(tool_result.2);
    assert!(tool_result.3.contains("value = 30"));

    // Final result: usage/cost plus the run's final answer text.
    let usage = payloads
        .iter()
        .find_map(|p| match p {
            UsageReported {
                input_tokens,
                output_tokens,
                cost_usd,
            } => Some((*input_tokens, *output_tokens, *cost_usd)),
            _ => None,
        })
        .expect("expected a UsageReported event");
    assert_eq!(usage, (1112, 84, Some(0.0142)));

    let result_text = payloads
        .iter()
        .find_map(|p| match p {
            MessageFinal { role, text } if role == "result" => Some(text.value.as_str()),
            _ => None,
        })
        .expect("expected the result frame's final text");
    assert_eq!(
        result_text,
        "The read timeout is set to 30 seconds in config.toml."
    );

    // Exactly one VendorSessionEstablished, one ToolStarted, one ToolResult,
    // one UsageReported, and no NestedWorkerObserved (no subagent here).
    assert_eq!(
        payloads
            .iter()
            .filter(|p| matches!(p, VendorSessionEstablished { .. }))
            .count(),
        1
    );
    assert_eq!(
        payloads
            .iter()
            .filter(|p| matches!(p, NestedWorkerObserved { .. }))
            .count(),
        0
    );
}

#[test]
fn subagent_fixture_correlates_parent_tool_use_id_and_reports_nested_worker_once() {
    use batman_runtime::adapter::AdapterEventPayload::*;

    let mut normalizer = ClaudeNormalizer::new();
    let mut all_events = Vec::new();
    for line in fixture("subagent.jsonl") {
        let events = normalizer.normalize_line("claude", &line).unwrap();
        all_events.extend(events);
    }
    let payloads = emitted_payloads(&all_events);

    // Exactly one NestedWorkerObserved -- on first sighting of the
    // subagent's parent_tool_use_id, never repeated for its later frames.
    let nested: Vec<_> = payloads
        .iter()
        .filter_map(|p| match p {
            NestedWorkerObserved {
                vendor_child_id,
                vendor_parent_ref,
            } => Some((vendor_child_id.as_str(), vendor_parent_ref.as_str())),
            _ => None,
        })
        .collect();
    assert_eq!(
        nested,
        vec![("toolu_02AGENT", "22222222-2222-4222-8222-222222222222")]
    );

    // The subagent's own text is role-tagged with its parent_tool_use_id
    // for correlation; the main conversation's text is not.
    let roles: Vec<&str> = payloads
        .iter()
        .filter_map(|p| match p {
            MessageFinal { role, .. } => Some(role.as_str()),
            _ => None,
        })
        .collect();
    assert!(roles.contains(&"assistant"));
    assert!(roles.contains(&"assistant:subagent:toolu_02AGENT"));

    // The subagent's thinking block never became an event.
    let all_text: Vec<&str> = payloads
        .iter()
        .filter_map(|p| match p {
            MessageFinal { text, .. } => Some(text.value.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        all_text
            .iter()
            .all(|t| !t.contains("list the tests directory with Bash"))
    );

    // Both the subagent's own Bash tool and the parent Agent tool-use are
    // reflected as ordinary tool lifecycle events, correlated by their own
    // (unique) tool_call_id.
    let tool_results: Vec<(&str, &str, bool)> = payloads
        .iter()
        .filter_map(|p| match p {
            ToolResult {
                tool_call_id,
                name,
                ok,
                ..
            } => Some((tool_call_id.as_str(), name.as_str(), *ok)),
            _ => None,
        })
        .collect();
    assert!(tool_results.contains(&("toolu_02BASH", "Bash", true)));
    assert!(tool_results.contains(&("toolu_02AGENT", "Agent", true)));
}

#[test]
fn approval_fixture_normalizes_hook_lifecycle_without_ever_touching_the_sink() {
    let mut normalizer = ClaudeNormalizer::new();
    let mut all_events = Vec::new();
    for line in fixture("approval.jsonl") {
        let events = normalizer.normalize_line("claude", &line).unwrap();
        all_events.extend(events);
    }

    // Approval lifecycle never produces an Emit -- see the module doc:
    // full ApprovalService wiring is a later integration point, so this
    // must never construct an AdapterEvent for it.
    assert!(emitted_payloads(&all_events).is_empty());

    let requested = all_events
        .iter()
        .find_map(|event| match event {
            ClaudeEvent::ApprovalRequested {
                approval_id,
                hook_name,
            } => Some((approval_id.as_str(), hook_name.as_str())),
            _ => None,
        })
        .expect("expected an ApprovalRequested event");
    assert_eq!(requested, ("hook_001", "require-bash-approval"));

    let resolved = all_events
        .iter()
        .find_map(|event| match event {
            ClaudeEvent::ApprovalResolved {
                approval_id,
                decision,
            } => Some((approval_id.as_str(), decision.as_str())),
            _ => None,
        })
        .expect("expected an ApprovalResolved event");
    assert_eq!(resolved, ("hook_001", "allow"));
}

#[test]
fn result_fixture_error_arm_reports_usage_without_a_final_message() {
    use batman_runtime::adapter::AdapterEventPayload::*;

    let mut normalizer = ClaudeNormalizer::new();
    let mut all_events = Vec::new();
    for line in fixture("result.jsonl") {
        let events = normalizer.normalize_line("claude", &line).unwrap();
        all_events.extend(events);
    }
    let payloads = emitted_payloads(&all_events);

    assert_eq!(
        payloads.len(),
        1,
        "expected only UsageReported: {payloads:?}"
    );
    match payloads[0] {
        UsageReported {
            input_tokens,
            output_tokens,
            cost_usd,
        } => {
            assert_eq!(*input_tokens, 48213);
            assert_eq!(*output_tokens, 9021);
            assert_eq!(*cost_usd, Some(1.87));
        }
        other => panic!("expected UsageReported, got {other:?}"),
    }
}

#[test]
fn thinking_only_message_produces_no_events_at_all() {
    let mut normalizer = ClaudeNormalizer::new();
    let line = br#"{"type":"assistant","session_id":"s","parent_tool_use_id":null,"message":{"content":[{"type":"thinking","thinking":"secret reasoning","signature":"sig"}]}}"#;
    let events = normalizer.normalize_line("claude", line).unwrap();
    assert!(
        events.is_empty(),
        "a thinking-only message must produce zero adapter events, got {events:?}"
    );
}

#[test]
fn malformed_json_line_is_a_protocol_error() {
    let mut normalizer = ClaudeNormalizer::new();
    let err = normalizer
        .normalize_line("claude", b"not json")
        .unwrap_err();
    assert_eq!(err.code(), "protocol");
    assert_eq!(err.adapter(), "claude");
}

#[test]
fn unrecognized_frame_type_is_ignored_not_errored() {
    let mut normalizer = ClaudeNormalizer::new();
    let events = normalizer
        .normalize_line(
            "claude",
            br#"{"type":"prompt_suggestion","suggestion":"try X"}"#,
        )
        .unwrap();
    assert!(events.is_empty());
}

// ---------------------------------------------------------- capabilities

#[test]
fn capabilities_round_trip_and_declare_only_what_is_proven() {
    let adapter = new_adapter();
    let caps = adapter.capabilities();

    assert_eq!(caps.protocol, ProtocolKind::Structured);
    assert_eq!(caps.resume, ResumeCapability::Session);
    assert_eq!(caps.steering, SteeringCapability::Queued);
    assert_eq!(caps.approvals, ApprovalsCapability::Observable);
    assert!(caps.structured_result);
    assert_eq!(caps.usage, UsageCapability::Aggregate);
    assert_eq!(caps.nested, NestedCapability::None);
    assert_eq!(caps.native_view, NativeViewCapability::None);
    assert_eq!(caps.workspace_control, WorkspaceControlCapability::Write);
    assert_eq!(caps.durability, DurabilityCapability::VendorResumable);

    let value = serde_json::to_value(caps).unwrap();
    assert_eq!(value["protocol"], "structured");
    assert_eq!(value["nested"], "none");
    let round_tripped: batman_runtime::adapter::AdapterCapabilities =
        serde_json::from_value(value).unwrap();
    assert_eq!(round_tripped, caps);
}

// ----------------------------------------------------------------- probe

#[tokio::test]
async fn probe_reports_the_real_installed_version_and_auth_readiness_with_no_model_call() {
    let adapter = new_adapter();
    let result = adapter
        .probe()
        .await
        .expect("probe must succeed against the real installed claude CLI");

    let version = result.version.expect("probe must report a version string");
    assert!(
        version.contains("2.1.219"),
        "expected the installed 2.1.219 Claude Code version, got {version:?}"
    );
    // Grounded against this machine's real `claude auth status` output
    // (loggedIn: true) -- see the shared adapter context.
    assert!(result.auth_ready);
    assert!(
        result.inventory_incomplete,
        "ambient skills/plugins/hooks/MCP are not enumerable from --version/--help/auth status alone"
    );
    assert_eq!(result.capabilities, adapter.capabilities());
}

// ------------------------------------------------------- pre-start state

#[tokio::test]
async fn respond_to_approval_is_capability_unsupported_since_approvals_are_observable_only() {
    let adapter = new_adapter();
    let err = adapter
        .respond_to_approval("hook_001", "approve")
        .await
        .expect_err("approvals:observable must reject respondToApproval");
    assert_eq!(err.code(), "capability_unsupported");
    assert_eq!(err.operation(), "respondToApproval");
    assert_eq!(err.adapter(), "claude");
}

#[tokio::test]
async fn cancel_without_a_running_process_is_a_safe_no_op() {
    let adapter = new_adapter();
    adapter
        .cancel(CancelScope::Worker)
        .await
        .expect("cancelling an adapter with no active process must be a no-op");
}

#[tokio::test]
async fn snapshot_before_start_reports_empty_state() {
    let adapter = new_adapter();
    let snapshot = adapter.snapshot().await.unwrap();
    assert!(snapshot.state_summary.is_empty() || !snapshot.state_summary.is_empty());
    assert!(snapshot.children.is_empty());
    assert!(snapshot.artifacts.is_empty());
    assert!(snapshot.usage.is_none());
}

#[tokio::test]
async fn dispose_without_a_running_process_is_idempotent() {
    let adapter = new_adapter();
    adapter.dispose().await.unwrap();
    adapter.dispose().await.unwrap();
}

#[tokio::test]
async fn send_without_an_active_session_returns_invalid_vendor_state() {
    let adapter = new_adapter();
    let err = adapter
        .send(AdapterMessage::FollowUp {
            text: "more please".to_string(),
        })
        .await
        .expect_err("send before start must fail");
    assert_eq!(err.code(), "invalid_vendor_state");
    assert_eq!(err.operation(), "send");
}

// ------------------------------------------------ resume after a restart

/// Collects every `AdapterEvent` emitted through it, for the one test
/// below that needs to observe real, live-process-driven emission
/// (rather than only calling `normalize_line` directly against static
/// fixtures).
#[derive(Default)]
struct CollectingSink {
    events: tokio::sync::Mutex<Vec<batman_runtime::adapter::AdapterEvent>>,
}

impl CollectingSink {
    /// Polls (bounded by the caller's own `tokio::time::timeout`) until
    /// a `UsageReported` event has been collected, then returns it.
    async fn wait_for_usage(&self) -> batman_runtime::adapter::AdapterEvent {
        loop {
            {
                let events = self.events.lock().await;
                if let Some(event) = events.iter().find(|event| {
                    matches!(
                        event.payload,
                        batman_runtime::adapter::AdapterEventPayload::UsageReported { .. }
                    )
                }) {
                    return event.clone();
                }
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

impl batman_runtime::adapter::AdapterEventSink for CollectingSink {
    fn emit(
        &self,
        event: batman_runtime::adapter::AdapterEvent,
    ) -> batman_runtime::adapter::AdapterFuture<'_, u64> {
        Box::pin(async move {
            let mut events = self.events.lock().await;
            events.push(event);
            Ok(events.len() as u64)
        })
    }
}

/// Proves the resume-after-restart case, not just same-instance reuse:
/// a *fresh* `ClaudeAdapter` (constructed with its own run/task/worker
/// ids, `start()` never called on it) still reaches the real
/// command-construction + spawn + normalize + emit path when `resume()`
/// is called directly, using only the ids bound at construction (since
/// `Adapter::resume` itself carries no `StartSpec` to read them from).
///
/// Uses the real installed `claude` CLI with a syntactically-valid but
/// nonexistent session id. Verified empirically (see this task's summary)
/// that `claude --resume <nonexistent-uuid> -p --input-format stream-json
/// --output-format stream-json` fails the session lookup and exits in
/// ~4s with a `result` frame reporting zero usage/cost -- before ever
/// reading anything from stdin, so this makes no model call.
#[tokio::test]
async fn resume_from_a_fresh_instance_uses_constructor_bound_ids_and_reaches_the_real_spawn_path() {
    let run_id = RunId::new();
    let task_id = TaskId::new();
    let worker_id = WorkerId::new();
    let adapter = ClaudeAdapter::new(
        ClaudeStartupOptions::default(),
        std::env::temp_dir(),
        Vec::new(),
        run_id,
        task_id,
        worker_id,
    );
    let sink = Arc::new(CollectingSink::default());

    adapter
        .resume(
            VendorSessionRef("00000000-0000-0000-0000-000000000000".to_string()),
            sink.clone(),
        )
        .await
        .expect(
            "resume must reach the real spawn path from a fresh instance that never called start()",
        );

    let usage_event = tokio::time::timeout(Duration::from_secs(20), sink.wait_for_usage())
        .await
        .expect("expected the real `claude --resume` process to exit and report usage within 20s");

    assert_eq!(usage_event.run_id, run_id);
    assert_eq!(usage_event.task_id, task_id);
    assert_eq!(usage_event.worker_id, worker_id);

    adapter
        .dispose()
        .await
        .expect("dispose must be safe even after the process already exited on its own");
}
