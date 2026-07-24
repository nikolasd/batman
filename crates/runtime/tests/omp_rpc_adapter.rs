//! Integration tests for the OMP-RPC / local-model worker adapter.
//!
//! Grounded against the real installed `omp 17.1.1` binary (`omp
//! --version`, `omp --mode rpc --help`, `omp models --json`, and direct
//! no-model-call RPC probes captured during development -- see the wire
//! shapes reproduced in `fixtures/adapters/omp-rpc/*.jsonl`, which mirror
//! frames this adapter actually observed from the real binary:
//! `{"type":"ready","protocolVersion":1,...}` and
//! `{"type":"response","id":...,"command":"get_state","success":true,
//! "data":{"sessionId":...,"sessionFile":...}}` were captured verbatim
//! from `omp --mode rpc --model lm-studio/<id> --session-dir <dir>`
//! without ever sending a prompt (zero model calls).
//!
//! Per the shared adapter contract, fixture-driven tests here feed
//! static, recorded-looking JSONL through `normalize.rs` directly rather
//! than spawning `fake-worker` (whose `omp-rpc` mode predates this
//! adapter's real wire-shape grounding and does not attempt to match it).
#[path = "../src/adapter/omp_rpc/mod.rs"]
mod omp_rpc;

use std::path::PathBuf;

use batman_runtime::adapter::AdapterEventPayload;
use batman_runtime::supervisor::{EnvironmentPolicy, SpawnSpec, Supervisor};
use omp_rpc::client::{
    self, OmpRpcClient, abort_command, follow_up_command, get_session_stats_command,
    get_state_command, prompt_command, set_subagent_subscription_command, steer_command,
};
use omp_rpc::normalize::{PROMPT_ACCEPTED_MARKER, PROMPT_COMPLETED_MARKER, normalize_frame};
use serde_json::Value;

// ------------------------------------------------------------- fixtures

fn load_fixture(name: &str) -> Vec<String> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/adapters/omp-rpc")
        .join(name);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading fixture {}: {e}", path.display()));
    text.lines().map(str::to_string).collect()
}

/// Mirrors exactly the recovery discipline `OmpRpcClient` applies to real
/// process stdout: a line that fails to parse as JSON is skipped, never
/// fatal, and normalization continues with the next line.
fn normalize_fixture_lines(lines: &[String]) -> Vec<AdapterEventPayload> {
    let mut events = Vec::new();
    let mut malformed_lines_skipped = 0usize;
    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Value>(line) {
            Ok(frame) => events.extend(normalize_frame(&frame)),
            Err(_) => malformed_lines_skipped += 1,
        }
    }
    assert!(
        malformed_lines_skipped > 0 || !lines.iter().any(|l| l == "this-is-not-json"),
        "expected the fixture's malformed line to be counted as skipped, not silently absent"
    );
    events
}

// --------------------------------------------------------------- Step 1

#[test]
fn prompt_acceptance_is_distinguishable_from_and_precedes_turn_completion() {
    let lines = load_fixture("turn.jsonl");
    let events = normalize_fixture_lines(&lines);

    let accepted_index = events
        .iter()
        .position(|e| matches!(e, AdapterEventPayload::MessageChunk { text, .. } if text.value == PROMPT_ACCEPTED_MARKER));
    let completed_index = events
        .iter()
        .position(|e| matches!(e, AdapterEventPayload::MessageFinal { text, .. } if text.value == PROMPT_COMPLETED_MARKER));

    let accepted_index = accepted_index.expect("prompt acceptance event must be emitted");
    let completed_index = completed_index.expect("turn completion event must be emitted");
    assert!(
        accepted_index < completed_index,
        "prompt acceptance ({accepted_index}) must precede turn completion ({completed_index})"
    );
    // The two are genuinely distinguishable: different payload variants
    // (MessageChunk vs MessageFinal), not merely different text.
    assert!(matches!(
        events[accepted_index],
        AdapterEventPayload::MessageChunk { .. }
    ));
    assert!(matches!(
        events[completed_index],
        AdapterEventPayload::MessageFinal { .. }
    ));
}

#[test]
fn malformed_json_line_is_skipped_not_fatal() {
    let lines = load_fixture("turn.jsonl");
    assert!(
        lines
            .iter()
            .any(|l| serde_json::from_str::<Value>(l).is_err()),
        "turn.jsonl must contain at least one genuinely malformed line to exercise recovery"
    );
    // normalize_fixture_lines itself asserts the malformed line was
    // skipped (not propagated as a panic/error) and processing continued:
    // reaching this point without panicking IS the proof.
    let events = normalize_fixture_lines(&lines);
    assert!(
        !events.is_empty(),
        "valid frames after the malformed line must still normalize"
    );
}

#[test]
fn local_only_prompt_completes_via_agent_invoked_false_without_agent_end() {
    let lines = load_fixture("turn.jsonl");
    assert!(
        !lines.iter().any(|l| l.contains("\"agent_end\"")),
        "turn.jsonl must be the local-only (no subagent invoked) fixture"
    );
    let events = normalize_fixture_lines(&lines);
    let completions = events
        .iter()
        .filter(|e| matches!(e, AdapterEventPayload::MessageFinal { text, .. } if text.value == PROMPT_COMPLETED_MARKER))
        .count();
    assert_eq!(
        completions, 1,
        "exactly one completion must be derived from data.agentInvoked:false"
    );
}

#[test]
fn get_state_response_establishes_vendor_session_from_real_session_id_field() {
    let lines = load_fixture("turn.jsonl");
    let events = normalize_fixture_lines(&lines);
    let established = events.iter().find_map(|e| match e {
        AdapterEventPayload::VendorSessionEstablished { vendor_session_id } => {
            Some(vendor_session_id.clone())
        }
        _ => None,
    });
    assert_eq!(
        established.as_deref(),
        Some("019f9652-7aac-7000-a8e1-db0d90064c58"),
        "vendor session id must be taken from the real omp get_state response's data.sessionId field"
    );
}

#[test]
fn get_session_stats_response_normalizes_to_usage_reported() {
    let lines = load_fixture("turn.jsonl");
    let events = normalize_fixture_lines(&lines);
    let usage = events.iter().find_map(|e| match e {
        AdapterEventPayload::UsageReported {
            input_tokens,
            output_tokens,
            cost_usd,
        } => Some((*input_tokens, *output_tokens, *cost_usd)),
        _ => None,
    });
    assert_eq!(usage, Some((42, 7, Some(0.0007))));
}

#[test]
fn subagent_subscription_is_established_before_the_prompt_command_when_nested_visibility_requested()
{
    // Pure command-sequencing check: the adapter's startup command order,
    // not a live process. `subscribe_subagents: true` mirrors a caller
    // requesting nested visibility.
    let commands = client::build_startup_commands(true, "review this diff");
    let subscription_index = commands
        .iter()
        .position(|(command, _)| command == "set_subagent_subscription")
        .expect("subagent subscription command must be sent when nested visibility is requested");
    let prompt_index = commands
        .iter()
        .position(|(command, _)| command == "prompt")
        .expect("prompt command must be sent");
    assert!(
        subscription_index < prompt_index,
        "subagent subscription must be established before work begins"
    );
}

#[test]
fn subagent_subscription_is_omitted_when_nested_visibility_is_not_requested() {
    let commands = client::build_startup_commands(false, "review this diff");
    assert!(
        !commands
            .iter()
            .any(|(command, _)| command == "set_subagent_subscription"),
        "must never send a subscription command the caller did not request"
    );
}

#[test]
fn subagents_fixture_observes_nested_worker_without_upgrading_declared_capability() {
    let lines = load_fixture("subagents.jsonl");
    let events = normalize_fixture_lines(&lines);

    let nested = events.iter().find_map(|e| match e {
        AdapterEventPayload::NestedWorkerObserved {
            vendor_child_id,
            vendor_parent_ref,
        } => Some((vendor_child_id.clone(), vendor_parent_ref.clone())),
        _ => None,
    });
    assert_eq!(
        nested,
        Some(("sub-1".to_string(), "main".to_string())),
        "a vendor-reported subagent must normalize to NestedWorkerObserved"
    );

    // agent_end must still complete the turn even though a subagent ran.
    let agent_end_completion = events
        .iter()
        .any(|e| matches!(e, AdapterEventPayload::MessageFinal { text, .. } if text.value == PROMPT_COMPLETED_MARKER));
    assert!(
        agent_end_completion,
        "agent_end must complete the agent-invoked turn"
    );

    // Tool lifecycle around the subagent's work must also normalize.
    assert!(
        events
            .iter()
            .any(|e| matches!(e, AdapterEventPayload::ToolStarted { name, .. } if name == "grep"))
    );
    assert!(events.iter().any(
        |e| matches!(e, AdapterEventPayload::ToolResult { name, ok, .. } if name == "grep" && *ok)
    ));
}

#[test]
fn agent_invoked_prompt_defers_completion_to_a_later_agent_end_frame() {
    let lines = load_fixture("subagents.jsonl");
    let events = normalize_fixture_lines(&lines);
    let accepted_index = events
        .iter()
        .position(|e| matches!(e, AdapterEventPayload::MessageChunk { text, .. } if text.value == PROMPT_ACCEPTED_MARKER))
        .expect("prompt acceptance must still be emitted for an agent-invoked prompt");
    let completed_index = events
        .iter()
        .position(|e| matches!(e, AdapterEventPayload::MessageFinal { text, .. } if text.value == PROMPT_COMPLETED_MARKER))
        .expect("agent_end must eventually complete the turn");
    // The subagent + tool events must land strictly between acceptance
    // and completion -- proving completion genuinely waited for agent_end
    // rather than firing immediately alongside acceptance.
    let nested_index = events
        .iter()
        .position(|e| matches!(e, AdapterEventPayload::NestedWorkerObserved { .. }))
        .expect("nested worker must be observed");
    assert!(accepted_index < nested_index);
    assert!(nested_index < completed_index);
}

// ----------------------------------------------------- command builders

#[test]
fn prompt_command_uses_the_real_message_field_name() {
    // Grounded against the installed binary's own dispatch source:
    // `case "prompt": { const H = await kI1(A, E.message, ...) }`.
    let params = prompt_command("hello");
    assert_eq!(params.get("message").and_then(Value::as_str), Some("hello"));
}

#[test]
fn steer_and_follow_up_commands_use_the_real_message_field_name() {
    // `case "steer": { await A.steer(E.message, ...) }`,
    // `case "follow_up": { await A.followUp(E.message, ...) }`.
    assert_eq!(
        steer_command("stop and check tests first").get("message"),
        Some(&Value::String("stop and check tests first".to_string()))
    );
    assert_eq!(
        follow_up_command("also update the docs").get("message"),
        Some(&Value::String("also update the docs".to_string()))
    );
}

#[test]
fn abort_and_get_state_commands_carry_no_extra_params() {
    assert!(abort_command().is_empty());
    assert!(get_state_command().is_empty());
    assert!(get_session_stats_command().is_empty());
}

#[test]
fn set_subagent_subscription_command_carries_a_level() {
    let params = set_subagent_subscription_command("full");
    assert_eq!(params.get("level").and_then(Value::as_str), Some("full"));
}

// -------------------------------------------------- real installed CLI

/// Real, no-model-call probe against the installed `omp` binary: spawns
/// `omp --mode rpc` with a local (`lm-studio/...`) model selector actually
/// reported by `omp models --json`, waits for the real `{"type":"ready",
/// ...}` handshake frame, and completes a real `get_state` round trip.
/// Never sends a `prompt` command, so it never invokes a model backend --
/// `lm-studio`'s local server does not even need to be running for this
/// test to pass, exactly as observed manually against the installed
/// `omp 17.1.1` binary during development.
#[tokio::test]
async fn ready_and_get_state_round_trip_against_installed_omp() {
    let selector = match resolve_first_local_selector().await {
        Some(selector) => selector,
        None => {
            eprintln!(
                "skipping: `omp models --json` reported no local (lm-studio/omlx) selector on this machine"
            );
            return;
        }
    };

    let workdir = std::env::temp_dir().join(format!("omp-rpc-adapter-test-{}", std::process::id()));
    std::fs::create_dir_all(&workdir).expect("create scratch workdir");

    let env = EnvironmentPolicy::baseline().build(&std::env::vars().collect(), &[]);
    let spec = SpawnSpec {
        program: "omp".into(),
        args: vec![
            "--mode".into(),
            "rpc".into(),
            "--model".into(),
            selector,
            "--no-session".into(),
            "--allow-home".into(),
        ],
        cwd: workdir.clone(),
        env,
        ..SpawnSpec::minimal()
    };
    let supervisor = Supervisor::new();
    let process = supervisor
        .spawn(spec)
        .await
        .expect("spawning the installed omp binary must succeed");

    let mut client = OmpRpcClient::new(process);
    let ready = client
        .wait_for_ready()
        .await
        .expect("the installed omp binary must emit a ready handshake frame");
    assert_eq!(ready.get("type").and_then(Value::as_str), Some("ready"));
    assert!(
        ready.get("protocolVersion").is_some(),
        "real omp ready frame carries a protocolVersion field"
    );

    let id = client
        .send_command("get_state", get_state_command())
        .await
        .expect("writing get_state to the real process must succeed");
    let response = client
        .read_response(&id)
        .await
        .expect("reading the correlated get_state response must succeed");
    assert_eq!(response.command, "get_state");
    assert!(
        response.success,
        "get_state must succeed with no model call"
    );
    assert!(
        response.data.get("sessionId").is_some(),
        "real omp get_state response must carry a sessionId field"
    );

    client.process_mut().terminate().await;
    let _ = std::fs::remove_dir_all(&workdir);
}

async fn resolve_first_local_selector() -> Option<String> {
    let output = tokio::process::Command::new("omp")
        .args(["models", "--json"])
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let parsed: Value = serde_json::from_slice(&output.stdout).ok()?;
    parsed
        .get("models")?
        .as_array()?
        .iter()
        .find(|m| {
            matches!(
                m.get("provider").and_then(Value::as_str),
                Some("lm-studio") | Some("omlx")
            )
        })
        .and_then(|m| m.get("selector"))
        .and_then(Value::as_str)
        .map(str::to_string)
}
