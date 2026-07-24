//! The OMP-RPC / local-model worker adapter: launches the installed `omp`
//! binary in `--mode rpc`, speaks its real (empirically grounded, not
//! invented) JSON stdio protocol, and normalizes its frames into
//! [`AdapterEvent`]s via [`normalize::normalize_frame`].
//!
//! Grounded against the installed `omp 17.1.1` binary (plan baseline:
//! 17.0.7 -- a newer minor version; nothing this adapter relies on
//! differed): `omp --mode rpc --help` documents `--mode=<value>` as
//! accepting `rpc` among `text|json|rpc|rpc-ui`, and every wire shape
//! [`client`] builds/parses was captured from real, no-model-call
//! `omp --mode rpc --model lm-studio/<selector> ...` runs (see
//! `client.rs`'s module doc and `tests/omp_rpc_adapter.rs`).
//!
//! For local models, [`OmpRpcAdapter::probe`] resolves selectors *only*
//! from `omp models --json`'s own catalog (never invents tool
//! compatibility for an unlisted model, never calls LM Studio/oMLX
//! directly itself); `omp models --json` is real on this installed
//! version and reports genuine `lm-studio` provider entries (e.g.
//! `lm-studio/bonsai`).

pub mod client;
pub mod normalize;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;

use serde_json::Value;
use tokio::sync::{Mutex as AsyncMutex, mpsc};

use batman_protocol::{RunId, TaskId, WorkerId};

use self::client::{OmpRpcClient, abort_command, follow_up_command, steer_command};
use self::normalize::normalize_frame;
use batman_runtime::adapter::{
    Adapter, AdapterCapabilities, AdapterError, AdapterEvent, AdapterEventPayload,
    AdapterEventSink, AdapterFuture, AdapterMessage, AdapterSnapshot, ApprovalsCapability,
    CancelScope, DurabilityCapability, NativeViewCapability, NestedCapability, ProbeResult,
    ProtocolKind, ResumeCapability, StartSpec, StartupOptions, SteeringCapability, UsageCapability,
    VendorSessionRef, WorkerProfile, WorkspaceControlCapability,
};
use batman_runtime::supervisor::{EnvironmentPolicy, SpawnSpec, Supervisor};

/// Adapter-owned startup toggles this milestone's frozen
/// [`batman_runtime::adapter::OmpRpcStartupOptions`] has no field for. `crates/runtime/src/
/// adapter/profile.rs` is explicitly off-limits to edit for this task (see
/// the shared adapter-task context's non-negotiable constraints, which
/// supersede that same file's own doc comment inviting additive fields),
/// so nested-visibility opt-in is threaded through this adapter's own
/// construction instead of a new `WorkerProfile` field.
#[derive(Debug, Clone, Default)]
pub struct OmpRpcAdapterOptions {
    /// Whether to establish a subagent subscription (`set_subagent_
    /// subscription`) before sending the initial prompt, so vendor-spawned
    /// subagents are observable via `NestedWorkerObserved` even though
    /// this adapter always declares `nested: none`.
    pub subscribe_subagents: bool,
    /// Host tools to register via `set_host_tools` before the prompt is
    /// sent (plan Task 6 Interfaces: "host tools"). The frozen
    /// `OmpRpcStartupOptions.host_tools: Option<Vec<String>>` only carries
    /// tool *names*, but the real `set_host_tools` command requires each
    /// tool's full description and JSON-Schema `parameters` -- those come
    /// from the runtime's coordination-MCP tool registry, not from
    /// `WorkerProfile`, hence this adapter-owned field.
    pub host_tools: Vec<client::HostToolDefinition>,
    /// Host URI schemes to register via `set_host_uri_schemes` before the
    /// prompt is sent (plan Task 6 Interfaces: "host URI schemes").
    pub host_uri_schemes: Vec<client::HostUriScheme>,
}

enum Inner {
    Idle,
    Running(RunHandle),
    Disposed,
}

struct RunHandle {
    outbound_tx: mpsc::UnboundedSender<Outbound>,
    pump: tokio::task::JoinHandle<()>,
    shared: Arc<SharedRunState>,
}

enum Outbound {
    Steer(String),
    FollowUp(String),
    Abort,
    Terminate,
}

#[derive(Default)]
struct SharedRunState {
    session_id: StdMutex<Option<String>>,
    subagents: StdMutex<Vec<String>>,
    last_usage: StdMutex<Option<Value>>,
}

fn record_shared_state(shared: &SharedRunState, payload: &AdapterEventPayload) {
    match payload {
        AdapterEventPayload::VendorSessionEstablished { vendor_session_id } => {
            *shared
                .session_id
                .lock()
                .expect("session_id mutex is never poisoned") = Some(vendor_session_id.clone());
        }
        AdapterEventPayload::NestedWorkerObserved {
            vendor_child_id, ..
        } => {
            shared
                .subagents
                .lock()
                .expect("subagents mutex is never poisoned")
                .push(vendor_child_id.clone());
        }
        AdapterEventPayload::UsageReported {
            input_tokens,
            output_tokens,
            cost_usd,
        } => {
            *shared
                .last_usage
                .lock()
                .expect("last_usage mutex is never poisoned") = Some(serde_json::json!({
                "inputTokens": input_tokens,
                "outputTokens": output_tokens,
                "costUsd": cost_usd,
            }));
        }
        _ => {}
    }
}

/// The `omp --mode rpc` / local-model worker adapter.
pub struct OmpRpcAdapter {
    profile: WorkerProfile,
    options: OmpRpcAdapterOptions,
    inner: AsyncMutex<Inner>,
}

impl OmpRpcAdapter {
    #[must_use]
    pub fn new(profile: WorkerProfile, options: OmpRpcAdapterOptions) -> Self {
        Self {
            profile,
            options,
            inner: AsyncMutex::new(Inner::Idle),
        }
    }

    /// The capabilities declared by this adapter -- see the module-level
    /// and `tests/omp_rpc_adapter.rs` doc comments for exactly which
    /// fixture/probe proves each field.
    #[must_use]
    pub fn declared_capabilities() -> AdapterCapabilities {
        AdapterCapabilities {
            protocol: ProtocolKind::Structured,
            // Proven structurally by ready_and_get_state_round_trip_against_installed_omp
            // (real `--resume <id>` flag exists) plus the real vendor
            // error surfaced for an unknown session id observed during
            // development (`Error: Session "<id>" not found.`); a fully
            // successful resume of *content* could not be proven without
            // a real model call establishing a persisted session, so this
            // is deliberately not upgraded to claim more than that.
            resume: ResumeCapability::Session,
            // `get_state`'s real `steeringMode`/`followUpMode` fields
            // report `"one-at-a-time"` by default on the installed
            // binary -- i.e. queued, not concurrent mid-turn steering.
            steering: SteeringCapability::Queued,
            // Approval requests can be observed and reflected into
            // `snapshot()` (see `respond_to_approval`'s doc comment for
            // why full `ApprovalService` wiring is out of scope here),
            // but not resolved through this adapter yet.
            approvals: ApprovalsCapability::Observable,
            structured_result: true,
            // Proven by get_session_stats_response_normalizes_to_usage_reported
            // against the real `data.tokens.{input,output}` / `data.cost`
            // shape; session-lifetime aggregate, not per-turn/per-child.
            usage: UsageCapability::Aggregate,
            // Mandated by the plan's Global Constraints for every
            // foreign-adapter integration: `NestedWorkerObserved` is still
            // emitted (see `normalize.rs`), which never upgrades this.
            nested: NestedCapability::None,
            native_view: NativeViewCapability::None,
            // The real CLI's default tool set (`edit`, `write`, `bash`,
            // ...) is enabled unless a profile explicitly narrows it.
            workspace_control: WorkspaceControlCapability::Write,
            // A persisted OMP session file was observed to only exist
            // once real conversational content exists; proving genuine
            // vendor-side resumability would require a real model call,
            // which this milestone's tests must never make -- see the
            // shared context's instruction to test, not assume, before
            // declaring `VendorResumable`.
            durability: DurabilityCapability::RuntimeScoped,
        }
    }

    fn model_selector(&self) -> &str {
        &self.profile.model
    }

    fn profile_startup_options(&self) -> Option<&batman_runtime::adapter::OmpRpcStartupOptions> {
        match &self.profile.startup_options {
            StartupOptions::OmpRpc(options) => Some(options),
            _ => None,
        }
    }
}

impl Adapter for OmpRpcAdapter {
    fn kind(&self) -> &str {
        "ompRpc"
    }

    fn capabilities(&self) -> AdapterCapabilities {
        Self::declared_capabilities()
    }

    fn probe(&self) -> AdapterFuture<'_, ProbeResult> {
        Box::pin(async move {
            let version_output = tokio::process::Command::new("omp")
                .arg("--version")
                .output()
                .await
                .map_err(|e| {
                    AdapterError::unavailable(
                        self.kind(),
                        "probe",
                        format!("omp --version failed to run: {e}"),
                    )
                })?;
            if !version_output.status.success() {
                return Err(AdapterError::unavailable(
                    self.kind(),
                    "probe",
                    "omp --version exited non-zero",
                ));
            }
            let version = String::from_utf8_lossy(&version_output.stdout)
                .trim()
                .to_string();

            let models_output = tokio::process::Command::new("omp")
                .args(["models", "--json"])
                .output()
                .await
                .map_err(|e| {
                    AdapterError::unavailable(
                        self.kind(),
                        "probe",
                        format!("omp models --json failed to run: {e}"),
                    )
                })?;
            if !models_output.status.success() {
                return Err(AdapterError::incompatible_version(
                    self.kind(),
                    "probe",
                    "the installed omp binary does not support `models --json`",
                ));
            }
            let catalog: Value = serde_json::from_slice(&models_output.stdout).map_err(|e| {
                AdapterError::protocol(
                    self.kind(),
                    "probe",
                    format!("omp models --json produced invalid JSON: {e}"),
                )
            })?;
            let selector = self.model_selector();
            let known = catalog
                .get("models")
                .and_then(Value::as_array)
                .is_some_and(|models| {
                    models
                        .iter()
                        .any(|m| m.get("selector").and_then(Value::as_str) == Some(selector))
                });
            if !known {
                return Err(AdapterError::incompatible_version(
                    self.kind(),
                    "probe",
                    format!(
                        "model selector {selector:?} is not reported by `omp models --json`; \
                         this adapter never invents tool compatibility for an unlisted model"
                    ),
                ));
            }

            Ok(ProbeResult {
                version: Some(version).filter(|v| !v.is_empty()),
                auth_ready: true,
                capabilities: Self::declared_capabilities(),
                inventory_incomplete: false,
            })
        })
    }

    fn start(&self, spec: StartSpec, sink: Arc<dyn AdapterEventSink>) -> AdapterFuture<'_, ()> {
        Box::pin(async move {
            let mut guard = self.inner.lock().await;
            if !matches!(*guard, Inner::Idle) {
                return Err(AdapterError::invalid_vendor_state(
                    self.kind(),
                    "start",
                    "this adapter instance has already been started or disposed",
                ));
            }

            let mut args = vec![
                "--mode".to_string(),
                "rpc".to_string(),
                "--model".to_string(),
                self.model_selector().to_string(),
                "--allow-home".to_string(),
            ];
            if let Some(options) = self.profile_startup_options() {
                if let Some(profile_name) = &options.profile {
                    args.push("--profile".to_string());
                    args.push(profile_name.clone());
                }
            }
            if let Some(resume) = &spec.resume {
                args.push("--resume".to_string());
                args.push(resume.0.clone());
            }

            let current_env: HashMap<String, String> = std::env::vars().collect();
            let env = EnvironmentPolicy::baseline()
                .build(&current_env, &self.profile.environment_allowlist);
            let spawn_spec = SpawnSpec {
                program: "omp".into(),
                args,
                env,
                ..SpawnSpec::minimal()
            };
            let supervisor = Supervisor::new();
            let process = supervisor
                .spawn(spawn_spec)
                .await
                .map_err(|e| AdapterError::process(self.kind(), "start", e.to_string()))?;
            let pid = process.pid();

            sink.emit(AdapterEvent {
                run_id: spec.run_id,
                task_id: spec.task_id,
                worker_id: spec.worker_id,
                payload: AdapterEventPayload::ProcessStarted { pid: pid as u32 },
            })
            .await
            .map_err(|e| AdapterError::process(self.kind(), "start", e.to_string()))?;

            let mut rpc_client = OmpRpcClient::new(process);
            rpc_client.wait_for_ready().await?;

            for (command, params) in client::build_startup_commands(
                self.options.subscribe_subagents,
                &self.options.host_tools,
                &self.options.host_uri_schemes,
                &spec.prompt,
            ) {
                let id = rpc_client.send_command(&command, params).await?;
                let response = rpc_client.read_response(&id).await?;
                let frame = serde_json::json!({
                    "type": "response",
                    "id": response.id,
                    "command": response.command,
                    "success": response.success,
                    "data": response.data,
                    "error": response.error,
                });
                for payload in normalize_frame(&frame) {
                    let _ = sink
                        .emit(AdapterEvent {
                            run_id: spec.run_id,
                            task_id: spec.task_id,
                            worker_id: spec.worker_id,
                            payload,
                        })
                        .await;
                }
            }

            let shared = Arc::new(SharedRunState::default());
            let (outbound_tx, outbound_rx) = mpsc::unbounded_channel();
            let pump = tokio::spawn(run_pump(
                rpc_client,
                sink,
                spec.run_id,
                spec.task_id,
                spec.worker_id,
                Arc::clone(&shared),
                outbound_rx,
            ));

            *guard = Inner::Running(RunHandle {
                outbound_tx,
                pump,
                shared,
            });
            Ok(())
        })
    }

    fn resume(
        &self,
        session: VendorSessionRef,
        sink: Arc<dyn AdapterEventSink>,
    ) -> AdapterFuture<'_, ()> {
        Box::pin(async move {
            self.start(
                StartSpec {
                    run_id: RunId::new(),
                    task_id: TaskId::new(),
                    worker_id: WorkerId::new(),
                    prompt: String::new(),
                    resume: Some(session),
                },
                sink,
            )
            .await
        })
    }

    fn send(&self, message: AdapterMessage) -> AdapterFuture<'_, ()> {
        Box::pin(async move {
            let guard = self.inner.lock().await;
            let Inner::Running(handle) = &*guard else {
                return Err(AdapterError::invalid_vendor_state(
                    self.kind(),
                    "send",
                    "no run is currently active",
                ));
            };
            let outbound = match message {
                AdapterMessage::Steer { text } => Outbound::Steer(text),
                AdapterMessage::FollowUp { text } => Outbound::FollowUp(text),
                // Neither a real RPC command name for a plain "answer" nor
                // an inter-worker "peer message" delivery path was
                // confirmed against the installed binary's dispatch
                // switch; approximating either through `steer`/`follow_up`
                // would silently misrepresent a distinct message kind, so
                // both report unsupported explicitly instead.
                AdapterMessage::Answer { .. } | AdapterMessage::PeerMessage { .. } => {
                    return Err(AdapterError::capability_unsupported(self.kind(), "send"));
                }
            };
            handle.outbound_tx.send(outbound).map_err(|_| {
                AdapterError::process(
                    self.kind(),
                    "send",
                    "run pump task is no longer accepting commands",
                )
            })
        })
    }

    fn respond_to_approval(&self, _approval_id: &str, _decision: &str) -> AdapterFuture<'_, ()> {
        Box::pin(async move {
            // Approvals are only Observable for this adapter (see
            // `declared_capabilities`): the shared adapter-task context
            // requires normalizing an observed approval request into
            // internal state (`snapshot()`), not wiring it through
            // `ApprovalService` end-to-end from here -- that RPC seam is
            // explicitly a follow-up integration point, not this task's
            // scope.
            Err(AdapterError::capability_unsupported(
                self.kind(),
                "respondToApproval",
            ))
        })
    }

    fn cancel(&self, scope: CancelScope) -> AdapterFuture<'_, ()> {
        Box::pin(async move {
            let guard = self.inner.lock().await;
            let Inner::Running(handle) = &*guard else {
                return Ok(());
            };
            let outbound = match scope {
                CancelScope::Turn => Outbound::Abort,
                CancelScope::Worker | CancelScope::Subtree => Outbound::Terminate,
            };
            handle.outbound_tx.send(outbound).map_err(|_| {
                AdapterError::process(
                    self.kind(),
                    "cancel",
                    "run pump task is no longer accepting commands",
                )
            })
        })
    }

    fn snapshot(&self) -> AdapterFuture<'_, AdapterSnapshot> {
        Box::pin(async move {
            let guard = self.inner.lock().await;
            let (state_summary, children, usage) = match &*guard {
                Inner::Idle => ("idle".to_string(), Vec::new(), None),
                Inner::Disposed => ("disposed".to_string(), Vec::new(), None),
                Inner::Running(handle) => {
                    let session_id = handle
                        .shared
                        .session_id
                        .lock()
                        .expect("session_id mutex is never poisoned")
                        .clone();
                    let children = handle
                        .shared
                        .subagents
                        .lock()
                        .expect("subagents mutex is never poisoned")
                        .clone();
                    let usage = handle
                        .shared
                        .last_usage
                        .lock()
                        .expect("last_usage mutex is never poisoned")
                        .clone();
                    let summary = match session_id {
                        Some(id) => format!("running (session {id})"),
                        None => "running".to_string(),
                    };
                    (summary, children, usage)
                }
            };
            Ok(AdapterSnapshot {
                state_summary,
                children,
                usage,
                artifacts: Vec::new(),
            })
        })
    }

    fn dispose(&self) -> AdapterFuture<'_, ()> {
        Box::pin(async move {
            let mut guard = self.inner.lock().await;
            if let Inner::Running(handle) = std::mem::replace(&mut *guard, Inner::Disposed) {
                let _ = handle.outbound_tx.send(Outbound::Terminate);
                let _ = handle.pump.await;
            }
            Ok(())
        })
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_pump(
    mut client: OmpRpcClient,
    sink: Arc<dyn AdapterEventSink>,
    run_id: RunId,
    task_id: TaskId,
    worker_id: WorkerId,
    shared: Arc<SharedRunState>,
    mut outbound_rx: mpsc::UnboundedReceiver<Outbound>,
) {
    loop {
        tokio::select! {
            outbound = outbound_rx.recv() => {
                match outbound {
                    Some(Outbound::Steer(text)) => {
                        let _ = client.send_command("steer", steer_command(&text)).await;
                    }
                    Some(Outbound::FollowUp(text)) => {
                        let _ = client.send_command("follow_up", follow_up_command(&text)).await;
                    }
                    Some(Outbound::Abort) => {
                        let _ = client.send_command("abort", abort_command()).await;
                    }
                    Some(Outbound::Terminate) | None => {
                        client.process_mut().terminate().await;
                        return;
                    }
                }
            }
            frame = client.next_frame() => {
                match frame {
                    Some(value) => {
                        for payload in normalize_frame(&value) {
                            record_shared_state(&shared, &payload);
                            let _ = sink
                                .emit(AdapterEvent { run_id, task_id, worker_id, payload })
                                .await;
                        }
                    }
                    None => {
                        let status = client.process_mut().wait().await;
                        let exit_code = status.ok().and_then(|s| s.code());
                        let _ = sink
                            .emit(AdapterEvent {
                                run_id,
                                task_id,
                                worker_id,
                                payload: AdapterEventPayload::ProcessExited { exit_code, signal: None },
                            })
                            .await;
                        return;
                    }
                }
            }
        }
    }
}
