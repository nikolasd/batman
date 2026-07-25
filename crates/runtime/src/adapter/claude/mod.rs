//! `ClaudeAdapter`: a thin protocol adapter over the installed `claude`
//! CLI's `stream-json` mode. Not a Claude Code re-implementation --
//! [`command`] builds the argv/stdin frames, [`protocol`] types the raw
//! wire shapes, and [`normalize`] turns them into [`AdapterEventPayload`]s
//! (see that module's doc for the thinking-block/approval-lifecycle
//! discipline).
//!
//! # Concurrency
//! Once [`ClaudeAdapter::start`]/[`ClaudeAdapter::resume`] spawns the
//! vendor process, a single background task owns the
//! [`ManagedProcess`] exclusively (its `write_stdin`/`next_stdout_frame`
//! both require `&mut self`, so no other caller may touch it directly).
//! [`ClaudeAdapter::send`]/[`ClaudeAdapter::cancel`]/[`ClaudeAdapter::dispose`]
//! talk to that task through an internal [`SessionCommand`] channel
//! instead. [`ClaudeAdapter::snapshot`] reads a small `Arc<Mutex<..>>` of
//! session facts (vendor session id, pending approvals, last usage) the
//! background task updates as it normalizes frames.
//!
//! # What is/isn't exercised by the default test run
//! `probe()` is exercised for real against the installed CLI (`claude
//! --version`, `claude auth status`) -- never a model call. `start()`/
//! `resume()`/`send()` are real, complete implementations, but actually
//! running one would write a real prompt to a real `claude -p` process's
//! stdin, which *would* invoke the model the moment the CLI reads it --
//! so the default `claude_adapter.rs` suite never calls them past their
//! own pre-start guard clauses. The optional, `#[ignore]`d
//! `claude_live.rs` end-to-end test (gated on `BATMAN_LIVE_CLAUDE=1`) is
//! what actually exercises the spawn+stdin+reader-task path.

pub mod command;
pub mod normalize;
pub mod protocol;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};

use batman_protocol::{RunId, TaskId, WorkerId};
use batman_runtime::adapter::mcp_config::{
    AdapterMcpConfig, coordination_mcp_config_document, coordination_mcp_env,
};
use batman_runtime::adapter::{
    Adapter, AdapterCapabilities, AdapterError, AdapterEvent, AdapterEventPayload,
    AdapterEventSink, AdapterFuture, AdapterMessage, AdapterSnapshot, ApprovalsCapability,
    CancelScope, ClaudeStartupOptions, DurabilityCapability, NativeViewCapability,
    NestedCapability, ProbeResult, ProtocolKind, ResumeCapability, StartSpec, SteeringCapability,
    UsageCapability, VendorSessionRef, WorkspaceControlCapability,
};
use batman_runtime::coordination::ScopeTokenStore;
use batman_runtime::supervisor::{EnvironmentPolicy, ManagedProcess, SpawnSpec, Supervisor};
use tokio::sync::{Mutex as TokioMutex, mpsc, oneshot};
use uuid::Uuid;

use normalize::{ClaudeEvent, ClaudeNormalizer};

/// A pending vendor-observed `PermissionRequest` hook, tracked only for
/// `snapshot()` to report -- see the crate-level doc.
#[derive(Debug, Clone)]
struct PendingApproval {
    hook_name: String,
}

/// Facts about the running (or most recently run) session, updated by the
/// background reader task and read synchronously by `snapshot()`.
#[derive(Debug, Default)]
struct SharedSessionInfo {
    vendor_session_id: Option<String>,
    pending_approvals: HashMap<String, PendingApproval>,
    last_usage: Option<serde_json::Value>,
    /// Path of the `--mcp-config` temp file written for the current
    /// session, if worker MCP tools were injected. Taken (set to
    /// `None`) by whichever of `run_session`'s post-loop cleanup or
    /// `dispose()` runs first, so the file is deleted exactly once.
    mcp_config_path: Option<PathBuf>,
}

/// A message the background session task acts on.
enum SessionCommand {
    WriteStdin(Vec<u8>),
    Terminate(oneshot::Sender<()>),
}

/// State guarded by `ClaudeAdapter::state`: the channel to the
/// background session task, if one is running, plus the shared facts it
/// updates.
#[derive(Default)]
struct ClaudeSessionState {
    commands: Option<mpsc::Sender<SessionCommand>>,
    shared: Arc<StdMutex<SharedSessionInfo>>,
}

/// A thin protocol adapter over the installed `claude` CLI's
/// `stream-json` mode. See the module doc for the concurrency model.
pub struct ClaudeAdapter {
    startup_options: ClaudeStartupOptions,
    cwd: PathBuf,
    environment_allowlist: Vec<String>,
    /// Bound to this adapter instance at construction (not read from
    /// `StartSpec`), so `resume()` -- which carries no `StartSpec` at
    /// all -- has a correlation to stamp on its `AdapterEvent`s even
    /// from a *fresh* instance (e.g. after a genuine runtime restart),
    /// not only when resuming on the same instance that previously
    /// called `start()`.
    run_id: RunId,
    task_id: TaskId,
    worker_id: WorkerId,
    supervisor: Supervisor,
    state: TokioMutex<ClaudeSessionState>,
    /// `None` for a caller that never asked for worker MCP tools
    /// (every pre-existing test/call site); `Some` injects the
    /// coordination MCP server into every spawned session.
    mcp: Option<AdapterMcpConfig>,
}

impl ClaudeAdapter {
    /// `cwd` is the workspace directory the supervised `claude` process
    /// runs in; `environment_allowlist` names extra environment variables
    /// (beyond `EnvironmentPolicy::baseline()`) the process may inherit.
    /// Workspace assignment itself is a later milestone's concern (see
    /// the shared adapter context) -- this adapter is handed an already-
    /// resolved `cwd` rather than deriving one from a `WorkerProfile`.
    /// `run_id`/`task_id`/`worker_id` identify the one run this adapter
    /// instance is scoped to; `start()` uses the (matching) ids on its
    /// own `StartSpec` instead, but `resume()` has no `StartSpec` to
    /// read them from, so they are bound here unconditionally. `mcp` is
    /// `None` for a caller that never asked for worker MCP tools.
    #[must_use]
    pub fn new(
        startup_options: ClaudeStartupOptions,
        cwd: PathBuf,
        environment_allowlist: Vec<String>,
        run_id: RunId,
        task_id: TaskId,
        worker_id: WorkerId,
        mcp: Option<AdapterMcpConfig>,
    ) -> Self {
        Self {
            startup_options,
            cwd,
            environment_allowlist,
            run_id,
            task_id,
            worker_id,
            supervisor: Supervisor::new(),
            state: TokioMutex::new(ClaudeSessionState::default()),
            mcp,
        }
    }

    fn spawn_env(&self) -> HashMap<String, String> {
        EnvironmentPolicy::baseline()
            .build(&std::env::vars().collect(), &self.environment_allowlist)
    }

    /// Runs a short-lived, no-model-call probe subcommand (`--version` or
    /// `auth status`) to completion and returns its stdout as text.
    async fn run_probe_command(&self, args: &[&str]) -> Result<String, AdapterError> {
        let spawn_spec = SpawnSpec {
            program: PathBuf::from("claude"),
            args: args.iter().map(ToString::to_string).collect(),
            env: self.spawn_env(),
            ..SpawnSpec::minimal()
        };
        let mut process = self
            .supervisor
            .spawn(spawn_spec)
            .await
            .map_err(|err| AdapterError::process(self.kind(), "probe", err.to_string()))?;

        let mut output = Vec::new();
        while let Some(frame) = process.next_stdout_frame().await {
            output.extend_from_slice(&frame);
            output.push(b'\n');
        }
        process.wait().await.ok();

        String::from_utf8(output)
            .map_err(|err| AdapterError::protocol(self.kind(), "probe", err.to_string()))
    }

    /// Starts (new session) or resumes (existing vendor session) the
    /// supervised `claude` process and hands it off to a background
    /// reader/writer task. Shared by `start`/`resume`.
    async fn spawn_session(
        &self,
        run_id: RunId,
        task_id: TaskId,
        worker_id: WorkerId,
        spec_resume: Option<VendorSessionRef>,
        initial_stdin: Option<Vec<u8>>,
        sink: Arc<dyn AdapterEventSink>,
    ) -> Result<(), AdapterError> {
        let mut state = self.state.lock().await;
        if state.commands.is_some() {
            return Err(AdapterError::invalid_vendor_state(
                self.kind(),
                "start",
                "a vendor process is already running for this adapter instance",
            ));
        }

        let start_spec = StartSpec {
            run_id,
            task_id,
            worker_id,
            prompt: String::new(),
            resume: spec_resume,
        };
        let session_id = Uuid::now_v7();
        let mut args = command::build_args(&self.startup_options, &start_spec, &session_id);
        let mut env = self.spawn_env();

        // Worker MCP tool injection, additive alongside every native
        // discovery flag `command::build_args` already produced above --
        // see `build_mcp_injection`'s doc. Left completely untouched (no
        // env addition, no extra arg) when `self.mcp` is `None`.
        let mcp_injection = match &self.mcp {
            Some(mcp) => {
                let injection = build_mcp_injection(mcp, run_id)
                    .map_err(|err| AdapterError::process(self.kind(), "start", err.to_string()))?;
                env.extend(injection.extra_env.clone());
                args.extend(injection.extra_args.clone());
                Some(injection)
            }
            None => None,
        };

        let spawn_spec = SpawnSpec {
            program: PathBuf::from("claude"),
            args,
            cwd: self.cwd.clone(),
            env,
            ..SpawnSpec::minimal()
        };
        let mut process = match self.supervisor.spawn(spawn_spec).await {
            Ok(process) => process,
            Err(err) => {
                if let Some(injection) = &mcp_injection {
                    let _ = std::fs::remove_file(&injection.config_path);
                }
                return Err(AdapterError::process(self.kind(), "start", err.to_string()));
            }
        };

        let pid = process.pid();

        // The token cannot be verified until it is bound to the vendor's
        // real pid, known only now. On failure the vendor process must
        // never be left running with a scope token that never went
        // live -- terminate it and report an error rather than proceed.
        if let (Some(mcp), Some(injection)) = (&self.mcp, &mcp_injection) {
            if let Err(err) = mcp.activate(
                injection.token.clone(),
                run_id,
                task_id,
                worker_id,
                pid,
                AdapterMcpConfig::default_expiry(),
            ) {
                process.terminate().await;
                let _ = std::fs::remove_file(&injection.config_path);
                return Err(AdapterError::process(
                    self.kind(),
                    "start",
                    format!("failed to activate worker MCP scope token: {err}"),
                ));
            }
        }

        sink.emit(AdapterEvent {
            run_id,
            task_id,
            worker_id,
            payload: AdapterEventPayload::ProcessStarted { pid: pid as u32 },
        })
        .await?;

        if let Some(bytes) = initial_stdin {
            process
                .write_stdin(&bytes)
                .await
                .map_err(|err| AdapterError::process(self.kind(), "start", err.to_string()))?;
        }

        let shared = Arc::new(StdMutex::new(SharedSessionInfo {
            mcp_config_path: mcp_injection
                .as_ref()
                .map(|injection| injection.config_path.clone()),
            ..SharedSessionInfo::default()
        }));
        let (commands_tx, commands_rx) = mpsc::channel(16);
        let kind = self.kind().to_string();
        let task_shared = shared.clone();
        let scope_tokens = self.mcp.as_ref().map(|mcp| mcp.scope_tokens.clone());
        tokio::spawn(run_session(
            process,
            commands_rx,
            ClaudeNormalizer::new(),
            sink,
            (run_id, task_id, worker_id),
            task_shared,
            kind,
            scope_tokens,
        ));

        state.commands = Some(commands_tx);
        state.shared = shared;
        Ok(())
    }
}

/// Everything one spawn needs to inject worker MCP tools into the
/// vendor process's argv/env, produced by [`build_mcp_injection`]: the
/// reserved (not yet activated) scope token, the argv suffix appended
/// after `command::build_args`'s output, the environment addition
/// merged into `spawn_env()`'s output, and the path of the
/// `--mcp-config` file just written -- tracked so the caller can clean
/// it up on any later failure, and so `run_session`/`dispose` can
/// delete it once the session ends.
pub struct McpInjection {
    pub token: String,
    pub config_path: PathBuf,
    pub extra_args: Vec<String>,
    pub extra_env: HashMap<String, String>,
}

/// Reserves a worker-MCP scope token and writes the Claude `--mcp-config`
/// file (owner-only `0600` permissions; the file names only the
/// `coordination-mcp` command/args, never the token itself -- see
/// `mcp_config`'s module doc for why the token only ever belongs in the
/// vendor process's own environment). Factored out of `spawn_session` so
/// the exact argv/env/file shape is unit-testable without spawning a
/// real `claude` process.
pub fn build_mcp_injection(mcp: &AdapterMcpConfig, run_id: RunId) -> std::io::Result<McpInjection> {
    let context = mcp.launch_context(run_id);
    let token = mcp.reserve();
    let extra_env = coordination_mcp_env(&token);
    let config_path = std::env::temp_dir().join(format!("batman-mcp-{run_id}.json"));
    let document = coordination_mcp_config_document(&context);
    write_mcp_config_file(&config_path, &document)?;
    Ok(McpInjection {
        token,
        extra_args: vec![
            "--mcp-config".to_string(),
            config_path.display().to_string(),
        ],
        extra_env,
        config_path,
    })
}

/// Writes `document` as pretty JSON to `path`, creating it (or
/// truncating an existing one) with owner-only (`0600`) permissions from
/// the moment it is created -- never briefly world-readable.
fn write_mcp_config_file(path: &Path, document: &serde_json::Value) -> std::io::Result<()> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;

    let contents = serde_json::to_vec_pretty(document)
        .expect("an MCP config document is always representable as JSON");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(&contents)
}

/// The background task that exclusively owns one `ManagedProcess`:
/// normalizes+emits every stdout frame, and serializes stdin
/// writes/termination requested through `commands`.
async fn run_session(
    mut process: ManagedProcess,
    mut commands: mpsc::Receiver<SessionCommand>,
    mut normalizer: ClaudeNormalizer,
    sink: Arc<dyn AdapterEventSink>,
    ids: (RunId, TaskId, WorkerId),
    shared: Arc<StdMutex<SharedSessionInfo>>,
    kind: String,
    scope_tokens: Option<Arc<ScopeTokenStore>>,
) {
    let (run_id, task_id, worker_id) = ids;
    loop {
        tokio::select! {
            frame = process.next_stdout_frame() => {
                match frame {
                    Some(bytes) => {
                        if let Ok(events) = normalizer.normalize_line(&kind, &bytes) {
                            for event in events {
                                match event {
                                    ClaudeEvent::Emit(payload) => {
                                        if let AdapterEventPayload::VendorSessionEstablished { vendor_session_id } = &payload {
                                            shared.lock().expect("session info mutex is never poisoned").vendor_session_id = Some(vendor_session_id.clone());
                                        }
                                        if let AdapterEventPayload::UsageReported { input_tokens, output_tokens, cost_usd } = &payload {
                                            shared.lock().expect("session info mutex is never poisoned").last_usage = Some(serde_json::json!({
                                                "inputTokens": input_tokens,
                                                "outputTokens": output_tokens,
                                                "costUsd": cost_usd,
                                            }));
                                        }
                                        let _ = sink.emit(AdapterEvent { run_id, task_id, worker_id, payload }).await;
                                    }
                                    ClaudeEvent::ApprovalRequested { approval_id, hook_name } => {
                                        shared.lock().expect("session info mutex is never poisoned").pending_approvals.insert(approval_id, PendingApproval { hook_name });
                                    }
                                    ClaudeEvent::ApprovalResolved { approval_id, .. } => {
                                        shared.lock().expect("session info mutex is never poisoned").pending_approvals.remove(&approval_id);
                                    }
                                }
                            }
                        }
                        // A single malformed line never kills the whole
                        // session's stream -- it is simply skipped.
                    }
                    None => break,
                }
            }
            cmd = commands.recv() => {
                match cmd {
                    Some(SessionCommand::WriteStdin(bytes)) => {
                        let _ = process.write_stdin(&bytes).await;
                    }
                    Some(SessionCommand::Terminate(reply)) => {
                        process.terminate().await;
                        let _ = reply.send(());
                        break;
                    }
                    None => break,
                }
            }
        }
    }

    // Vendor-exit hook: the loop above only ever breaks once the
    // supervised process has exited (stdout closed, cancelled, or the
    // commands channel closed) -- revoke the scope token and delete the
    // `--mcp-config` temp file right here, regardless of which arm broke
    // the loop. `mcp_config_path.take()` (guarded by the same mutex
    // `dispose()` reads) ensures the file is deleted at most once even
    // if `dispose()` raced this same cleanup.
    if let Some(scope_tokens) = scope_tokens {
        scope_tokens.revoke_for_run(run_id);
    }
    let path_to_delete = shared
        .lock()
        .expect("session info mutex is never poisoned")
        .mcp_config_path
        .take();
    if let Some(path) = path_to_delete {
        let _ = std::fs::remove_file(path);
    }
}

impl Adapter for ClaudeAdapter {
    fn kind(&self) -> &str {
        "claude"
    }

    fn capabilities(&self) -> AdapterCapabilities {
        AdapterCapabilities {
            protocol: ProtocolKind::Structured,
            // Proven by `command::build_args`'s resume test: a
            // `VendorSessionRef` becomes `--resume <id>`.
            resume: ResumeCapability::Session,
            // Proven by `command::build_stdin_user_message` plus
            // `send`'s pre-start guard: every follow-up/steer/answer/
            // peer message becomes another queued `user` turn on the
            // same `stream-json` stdin stream (see
            // <https://code.claude.com/docs/en/agent-sdk/typescript>'s
            // streaming-input docs) rather than replacing an in-flight
            // turn.
            steering: SteeringCapability::Queued,
            // The vendor CLI's `PermissionRequest` hook lifecycle is
            // observable (see `normalize`'s `ApprovalRequested`/
            // `ApprovalResolved` and the approval fixture test), but
            // resolving one end-to-end through `ApprovalService` is out
            // of this milestone's scope -- `respond_to_approval` always
            // returns `capability_unsupported`.
            approvals: ApprovalsCapability::Observable,
            // Proven by the `result.jsonl`/`initialize.jsonl` fixture
            // tests: the `result` frame's `result` text normalizes to a
            // structured `MessageFinal`.
            structured_result: true,
            // Only the final `result` frame's aggregate usage/cost is
            // normalized (never per-message `usage`), so `Aggregate`,
            // not `PerTurn`.
            usage: UsageCapability::Aggregate,
            // Mandated for every foreign adapter regardless of what the
            // vendor protocol itself observes -- see the shared context.
            nested: NestedCapability::None,
            native_view: NativeViewCapability::None,
            workspace_control: WorkspaceControlCapability::Write,
            // Claude persists sessions to disk independently of this
            // runtime (`--resume`/`--session-id`/`--continue`), proven at
            // the command-construction level by the resume test above.
            durability: DurabilityCapability::VendorResumable,
        }
    }

    fn probe(&self) -> AdapterFuture<'_, ProbeResult> {
        Box::pin(async move {
            let version_output = self.run_probe_command(&["--version"]).await?;
            let version = version_output
                .split_whitespace()
                .next()
                .map(str::to_string)
                .filter(|s| !s.is_empty());

            let auth_output = self.run_probe_command(&["auth", "status"]).await?;
            let auth_ready = serde_json::from_str::<serde_json::Value>(&auth_output)
                .ok()
                .and_then(|value| value.get("loggedIn").and_then(serde_json::Value::as_bool))
                .unwrap_or(false);

            Ok(ProbeResult {
                version,
                auth_ready,
                capabilities: self.capabilities(),
                // Ambient skills/plugins/hooks/MCP servers are only
                // enumerable from a live `system/init` frame (a real
                // session), never from `--version`/`--help`/`auth
                // status` alone.
                inventory_incomplete: true,
            })
        })
    }

    fn start(&self, spec: StartSpec, sink: Arc<dyn AdapterEventSink>) -> AdapterFuture<'_, ()> {
        Box::pin(async move {
            let initial_stdin = command::build_stdin_user_message(&spec.prompt);
            self.spawn_session(
                spec.run_id,
                spec.task_id,
                spec.worker_id,
                spec.resume,
                Some(initial_stdin),
                sink,
            )
            .await
        })
    }

    fn resume(
        &self,
        session: VendorSessionRef,
        sink: Arc<dyn AdapterEventSink>,
    ) -> AdapterFuture<'_, ()> {
        Box::pin(async move {
            // `Adapter::resume` carries no `StartSpec`, so there is no
            // per-call `run_id`/`task_id`/`worker_id` to stamp on the
            // `AdapterEvent`s this resumed session emits -- unlike
            // `start()`. This adapter is bound to its run/task/worker at
            // construction (see `ClaudeAdapter::new`) precisely so this
            // path works from a *fresh* instance too (e.g. after a
            // genuine runtime restart), not only when resuming on the
            // same instance that previously called `start()`.
            self.spawn_session(
                self.run_id,
                self.task_id,
                self.worker_id,
                Some(session),
                None,
                sink,
            )
            .await
        })
    }

    fn send(&self, message: AdapterMessage) -> AdapterFuture<'_, ()> {
        Box::pin(async move {
            let text = match &message {
                AdapterMessage::Steer { text }
                | AdapterMessage::FollowUp { text }
                | AdapterMessage::Answer { text }
                | AdapterMessage::PeerMessage { text } => text.clone(),
            };
            let state = self.state.lock().await;
            let Some(commands) = state.commands.clone() else {
                return Err(AdapterError::invalid_vendor_state(
                    self.kind(),
                    "send",
                    "no active vendor session to send this message to",
                ));
            };
            drop(state);
            let bytes = command::build_stdin_user_message(&text);
            commands
                .send(SessionCommand::WriteStdin(bytes))
                .await
                .map_err(|_| {
                    AdapterError::invalid_vendor_state(
                        self.kind(),
                        "send",
                        "the vendor session's background task has already exited",
                    )
                })
        })
    }

    fn respond_to_approval(&self, _approval_id: &str, _decision: &str) -> AdapterFuture<'_, ()> {
        Box::pin(async move {
            Err(AdapterError::capability_unsupported(
                self.kind(),
                "respondToApproval",
            ))
        })
    }

    fn cancel(&self, _scope: CancelScope) -> AdapterFuture<'_, ()> {
        Box::pin(async move {
            let mut state = self.state.lock().await;
            let Some(commands) = state.commands.take() else {
                return Ok(());
            };
            let (reply_tx, reply_rx) = oneshot::channel();
            if commands
                .send(SessionCommand::Terminate(reply_tx))
                .await
                .is_ok()
            {
                let _ = reply_rx.await;
            }
            Ok(())
        })
    }

    fn snapshot(&self) -> AdapterFuture<'_, AdapterSnapshot> {
        Box::pin(async move {
            let state = self.state.lock().await;
            let shared = state
                .shared
                .lock()
                .expect("session info mutex is never poisoned");
            let mut state_summary = match &shared.vendor_session_id {
                Some(session_id) => format!("claude session {session_id}"),
                None => String::new(),
            };
            if !shared.pending_approvals.is_empty() {
                let hook_names: Vec<&str> = shared
                    .pending_approvals
                    .values()
                    .map(|approval| approval.hook_name.as_str())
                    .collect();
                if !state_summary.is_empty() {
                    state_summary.push_str(", ");
                }
                state_summary.push_str(&format!(
                    "{} pending approval(s): {}",
                    hook_names.len(),
                    hook_names.join(", ")
                ));
            }
            Ok(AdapterSnapshot {
                state_summary,
                children: Vec::new(),
                usage: shared.last_usage.clone(),
                artifacts: Vec::new(),
            })
        })
    }

    fn dispose(&self) -> AdapterFuture<'_, ()> {
        Box::pin(async move {
            let mut state = self.state.lock().await;
            if let Some(commands) = state.commands.take() {
                let (reply_tx, reply_rx) = oneshot::channel();
                if commands
                    .send(SessionCommand::Terminate(reply_tx))
                    .await
                    .is_ok()
                {
                    let _ = reply_rx.await;
                }
            }
            if let Some(mcp) = &self.mcp {
                mcp.scope_tokens.revoke_for_run(self.run_id);
                let path_to_delete = state
                    .shared
                    .lock()
                    .expect("session info mutex is never poisoned")
                    .mcp_config_path
                    .take();
                if let Some(path) = path_to_delete {
                    let _ = std::fs::remove_file(path);
                }
            }
            Ok(())
        })
    }
}
