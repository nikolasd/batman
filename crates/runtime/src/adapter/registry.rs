//! The adapter registry: implements [`crate::service::RunDriver`] by
//! resolving a run's immutable worker profile, gating start on
//! conformance-derived effective capabilities through an injected
//! [`AdapterAuthorization`], constructing the matching [`Adapter`], and
//! owning it for the run's lifetime in a run-indexed table.
//!
//! # Scope boundary (documented, not silently omitted)
//! [`RunDriverContext::prompt`] carries the task's initial content (closed
//! as part of the M2/M3 gap-closure milestone): `run_one` passes
//! `ctx.prompt.clone().unwrap_or_default()` into [`StartSpec::prompt`], so
//! `run/submit`'s optional `RunSpec::prompt` now reaches the adapter at
//! start time. Delivering a *later* follow-up to an already-running
//! adapter instance is a separate seam (`RunDriver::send_follow_up`,
//! implemented below and invoked from `OrchestrationService::message_send`)
//! rather than a second `start()` call. Claude/Codex/Copilot adapters
//! constructed here now receive worker-coordination MCP config too
//! (closed alongside the prompt gap): `AdapterRegistry::new` accepts an
//! `Option<AdapterMcpConfig>`, built by `lifecycle::serve()` from a
//! resolved `batcave` binary path, state dir, and repository root, and
//! threaded into every Claude/Codex/Copilot adapter this registry
//! constructs. OMP-RPC's in-process host-tool bridge instead needs a
//! `CoordinationBroker`, supplied after construction via
//! [`AdapterRegistry::set_broker`] (see that method's own doc comment
//! for why it cannot be a constructor argument).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::Mutex;

use batman_protocol::{RunId, TaskId, WorkerId};

use super::capability::{AdapterCapabilities, NestedCapability};
use super::event_sink::DomainAdapterEventSink;
use super::mcp_config::AdapterMcpConfig;
use super::profile::{StartupOptions, WorkerProfile};
use super::r#trait::{Adapter, AdapterMessage, StartSpec};
use crate::conformance;
use crate::coordination::CoordinationBroker;
use crate::domain::DomainRepository;
use crate::service::{AdapterFuture as RunDriverFuture, RunDriver, RunDriverContext};
use crate::adapter::CancelScope;
/// adapter, given `effective_capabilities` -- always the conformance-
/// filtered set, never the adapter's raw declared claims. Production
/// construction of [`AdapterRegistry`] requires a real implementation;
/// tests inject an allow/deny fixture (see [`FixtureAuthorization`]).
pub trait AdapterAuthorization: Send + Sync {
    /// # Errors
    /// Returns a human-readable denial reason. The run is never started
    /// when this returns `Err`.
    fn authorize(
        &self,
        profile: &WorkerProfile,
        effective_capabilities: &AdapterCapabilities,
    ) -> Result<(), String>;
}

/// A deterministic allow/deny fixture for tests. Production callers must
/// supply a real policy, per the plan's "do not ship a permissive
/// production authorization implementation."
pub struct FixtureAuthorization {
    pub allow: bool,
}

impl AdapterAuthorization for FixtureAuthorization {
    fn authorize(
        &self,
        _profile: &WorkerProfile,
        _effective_capabilities: &AdapterCapabilities,
    ) -> Result<(), String> {
        if self.allow {
            Ok(())
        } else {
            Err("denied by fixture authorization".to_string())
        }
    }
}

/// The production [`AdapterAuthorization`]: denies every worker unless the
/// development override is explicitly set. Replaced by the Hardening plan's
/// `PolicyEvaluator`, which owns model/adapter allowlists and ceilings.
pub struct DenyByDefaultAuthorization {
    dev_override: bool,
}

impl DenyByDefaultAuthorization {
    /// Reads `BATMAN_DEV_ALLOW_ALL_WORKERS` once, at construction.
    #[must_use]
    pub fn from_env() -> Self {
        let dev_override = std::env::var("BATMAN_DEV_ALLOW_ALL_WORKERS").as_deref() == Ok("1");
        if dev_override {
            tracing::warn!(
                code = "dev_authorization_override",
                "BATMAN_DEV_ALLOW_ALL_WORKERS=1 is set; all workers are authorized. \
                 This is a development override and must not be used in production."
            );
        }
        Self { dev_override }
    }
}

impl AdapterAuthorization for DenyByDefaultAuthorization {
    fn authorize(
        &self,
        _profile: &WorkerProfile,
        _effective_capabilities: &AdapterCapabilities,
    ) -> Result<(), String> {
        if self.dev_override {
            Ok(())
        } else {
            Err("no production authorization policy is configured. Set \
                 BATMAN_DEV_ALLOW_ALL_WORKERS=1 for local development, or wait for the \
                 Hardening milestone's PolicyEvaluator."
                .to_string())
        }
    }
}

/// Why [`AdapterRegistry::start`] could not start (or continue driving)
/// a run. Always converted to a plain `String` at the [`RunDriver`]
/// boundary (that trait's own contract), but kept structured internally
/// so tests can assert on the exact rejection reason.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RegistryError {
    #[error("run {0} already has a running adapter instance")]
    DuplicateStart(RunId),
    #[error("worker has no resolved profile snapshot")]
    NoResolvedProfile,
    #[error("failed to read the resolved worker profile: {0}")]
    ProfileUnreadable(String),
    #[error("authorization denied: {0}")]
    AuthorizationDenied(String),
    #[error("no adapter is currently running for run {0}")]
    NoRunningAdapter(RunId),
}

impl From<RegistryError> for String {
    fn from(err: RegistryError) -> Self {
        err.to_string()
    }
}

/// Implements [`RunDriver`] against the four real worker adapters.
///
/// Always constructed behind an `Arc` in practice (exactly like every
/// other `RunDriver`, per `OrchestrationService`'s own
/// `run_driver: Option<Arc<dyn RunDriver>>` field) -- `Self::start`'s
/// `'static` future clones every field it needs out of `&self` rather
/// than borrowing it, so this requirement is never actually load-bearing
/// for soundness, only for the instance to still exist by the time a
/// caller awaits the future.
pub struct AdapterRegistry {
    authorization: Arc<dyn AdapterAuthorization>,
    /// The working directory every supervised vendor process is launched
    /// in. One registry instance serves one repository, exactly like one
    /// `batcave` daemon does.
    repo_root: PathBuf,
    /// Worker-coordination MCP launch config, given to every Claude/Codex/
    /// Copilot adapter this registry constructs so their supervised vendor
    /// processes can reach the `batman` coordination MCP server. `None`
    /// for callers (chiefly tests) that never asked for worker MCP tools.
    mcp: Option<AdapterMcpConfig>,
    /// The [`CoordinationBroker`] OMP-RPC adapters answer their in-process
    /// `host_tool_call` bridge against. Set once, after construction, via
    /// [`Self::set_broker`] -- unlike `mcp`, the real broker instance is
    /// owned by [`crate::ipc::Server`] and only exists after `Server::bind`
    /// returns, which happens *after* this registry must already be handed
    /// to [`crate::ipc::ServerConfig::run_driver`]. `None` until set (or
    /// permanently, for callers that never call the setter): OMP-RPC
    /// adapters constructed in that window get no broker, matching their
    /// existing `broker: None` behavior exactly.
    broker: Mutex<Option<Arc<CoordinationBroker>>>,
    running: Arc<Mutex<HashMap<RunId, Arc<dyn Adapter>>>>,
    /// Org security patterns for redaction.
    org_security_patterns: Vec<String>,
}

impl AdapterRegistry {
    #[must_use]
    pub fn new(
        authorization: Arc<dyn AdapterAuthorization>,
        repo_root: PathBuf,
        mcp: Option<AdapterMcpConfig>,
        org_security_patterns: Vec<String>,
    ) -> Self {
        Self {
            authorization,
            repo_root,
            mcp,
            org_security_patterns,
            broker: Mutex::new(None),
            running: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Supplies the real [`CoordinationBroker`] for this registry's
    /// OMP-RPC adapters' in-process host-tool bridge, once the caller
    /// field's own doc comment for why this cannot be a constructor
    /// argument.
    pub fn set_broker(&self, broker: Arc<CoordinationBroker>) {
        *self.broker.lock() = Some(broker);
    }

    /// The adapter instance currently running for `run_id`, if any --
    /// exposed for tests and for the message-forwarding seam this
    /// module's own doc comment names as a follow-up.
    #[must_use]
    pub fn running_adapter(&self, run_id: RunId) -> Option<Arc<dyn Adapter>> {
        self.running.lock().get(&run_id).cloned()
    }

    /// How many adapters this registry is currently driving. Exposed for
    /// tests asserting an instance was actually inserted/removed.
    #[must_use]
    pub fn running_count(&self) -> usize {
        self.running.lock().len()
    }
}

impl RunDriver for AdapterRegistry {
    fn start(&self, ctx: RunDriverContext) -> RunDriverFuture<'static, Result<(), String>> {
        let authorization = Arc::clone(&self.authorization);
        let repo_root = self.repo_root.clone();
        let mcp = self.mcp.clone();
        let broker = self.broker.lock().clone();
        let org_security_patterns = self.org_security_patterns.clone();
        let running = Arc::clone(&self.running);

        Box::pin(async move {
            // Reserve the run-id slot atomically with the duplicate
            // check: nothing between "is it already running" and
            // "mark it running" can race a second concurrent `start`
            // for the same run past this point, since both hold the
            // same lock for the whole check-then-insert.
            {
                let mut guard = running.lock();
                if guard.contains_key(&ctx.run_id) {
                    return Err(RegistryError::DuplicateStart(ctx.run_id).into());
                }
                // A placeholder; overwritten with the real adapter once
                // constructed below. Never observable from outside this
                // function: readers only see it as "already running",
                // exactly the state a duplicate-start rejection wants.
                guard.insert(ctx.run_id, build_placeholder_adapter());
            }

            let events_tx = ctx.events_tx.clone();
            let run_id = ctx.run_id;
            match run_one(
                &ctx,
                &authorization,
                &repo_root,
                mcp,
                broker,
                org_security_patterns,
            )
            .await
            {
                Ok(adapter) => {
                    running.lock().insert(run_id, adapter);
                    // Evicts and disposes this run's adapter once its
                    // supervised process actually exits. This is the
                    // only terminal-settlement signal available without
                    // a new `RunDriver` method (see the module doc's
                    // scope-boundary note) -- it is carried on the very
                    // `events_tx` `RunDriverContext` already supplies,
                    // so it needs no additional wiring.
                    let running_for_watcher = Arc::clone(&running);
                    let mut events_rx = events_tx.subscribe();
                    tokio::spawn(async move {
                        while let Ok(envelope) = events_rx.recv().await {
                            if is_process_exited_for(&envelope.event, run_id) {
                                let evicted = running_for_watcher.lock().remove(&run_id);
                                if let Some(adapter) = evicted {
                                    let _ = adapter.dispose().await;
                                }
                                break;
                            }
                        }
                    });
                    Ok(())
                }
                Err(err) => {
                    // The reservation above must not leak on any failure
                    // path -- a rejected/failed start must be startable
                    // again.
                    running.lock().remove(&run_id);
                    Err(err)
                }
            }
        })
    }

    fn send_follow_up(
        &self,
        run_id: RunId,
        _task_id: TaskId,
        _worker_id: WorkerId,
        prompt: String,
    ) -> RunDriverFuture<'static, Result<(), String>> {
        let running = Arc::clone(&self.running);

        Box::pin(async move {
            let adapter = running.lock().get(&run_id).cloned().ok_or_else(|| {
                <RegistryError as Into<String>>::into(RegistryError::NoRunningAdapter(run_id))
            })?;

            adapter
                .send(AdapterMessage::FollowUp { text: prompt })
                .await
                .map_err(|err| err.to_string())
        })
    }

    fn running_adapter(&self, run_id: RunId) -> Option<Arc<dyn Adapter>> {
        let running = Arc::clone(&self.running);
        running.lock().get(&run_id).cloned()
    }

    fn cancel_run(&self, run_id: RunId, scope: CancelScope) -> RunDriverFuture<'static, Result<(), String>> {
        let running = Arc::clone(&self.running);

        Box::pin(async move {
            let adapter = running.lock().get(&run_id).cloned().ok_or_else(|| {
                RegistryError::NoRunningAdapter(run_id).to_string()
            })?;

            adapter.cancel(scope).await.map_err(|e| e.to_string())
        })
    }
}

fn is_process_exited_for(event: &batman_protocol::RuntimeEvent, run_id: RunId) -> bool {
    matches!(
        event,
        batman_protocol::RuntimeEvent::AdapterProcessEvent {
            kind: batman_protocol::RuntimeEventKind::AdapterProcessExited,
            run_id: event_run_id,
            ..
        } if *event_run_id == run_id
    )
}

/// A never-started, immediately-idle placeholder occupying the run-id
/// reservation slot while the real adapter is constructed. Its `start`/
/// `resume`/`send`/etc. are never called; it exists only to make
/// `running.contains_key` true for the duration of construction.
fn build_placeholder_adapter() -> Arc<dyn Adapter> {
    Arc::new(super::OmpRpcAdapter::new(
        WorkerProfile {
            id: super::ProfileId::new(),
            adapter: "ompRpc".to_string(),
            model: String::new(),
            permission_envelope: serde_json::json!({}),
            startup_options: StartupOptions::OmpRpc(super::OmpRpcStartupOptions::default()),
            environment_allowlist: Vec::new(),
            source: "registry-placeholder".to_string(),
        },
        super::OmpRpcAdapterOptions::default(),
        None,
    ))
}

async fn run_one(
    ctx: &RunDriverContext,
    authorization: &Arc<dyn AdapterAuthorization>,
    repo_root: &std::path::Path,
    mcp: Option<AdapterMcpConfig>,
    broker: Option<Arc<CoordinationBroker>>,
    org_security_patterns: Vec<String>,
) -> Result<Arc<dyn Adapter>, String> {
    let profile = resolve_profile(ctx).await.map_err(String::from)?;

    // Handle TerminalDegraded specially (it has no adapter kind)
    let effective_capabilities = if profile.adapter_kind().is_none() {
        // TerminalDegraded uses the terminal adapter with degraded capabilities
        // We need to extract the backend from the startup options
        if let StartupOptions::TerminalDegraded(opts) = &profile.startup_options() {
            super::terminal::TerminalAdapter::new(opts.backend.clone()).capabilities()
        } else {
            return Err("TerminalDegraded profile has no startup options".to_string());
        }
    } else {
        let Some(kind) = profile.adapter_kind() else {
            return Err("no adapter kind".to_string());
        };
        conformance::run_fixture_conformance(kind)
            .await
            .effective_capabilities
    };
    authorization
        .authorize(&profile, &effective_capabilities)
        .map_err(RegistryError::AuthorizationDenied)
        .map_err(String::from)?;

    // Use the workspace path from the context (isolated worktree or copy)
    // when available; fall back to the repository root.
    let cwd = ctx.workspace_path.as_deref().unwrap_or(repo_root);

    let adapter = build_adapter(
        &profile,
        cwd,
        ctx.run_id,
        ctx.task_id,
        ctx.worker_id,
        mcp,
        broker,
    )
    .map_err(String::from)?;
    let sink = Arc::new(DomainAdapterEventSink::new(
        Arc::clone(&ctx.db),
        ctx.project_id,
        ctx.events_tx.clone(),
        org_security_patterns,
        effective_capabilities.nested != NestedCapability::Managed,
        Arc::clone(&ctx.violation_service),
    ));
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
        .map_err(|err| err.to_string())?;
    Ok(adapter)
}

async fn resolve_profile(ctx: &RunDriverContext) -> Result<WorkerProfile, RegistryError> {
    let db = Arc::clone(&ctx.db);
    let project_id = ctx.project_id;
    let worker_id = ctx.worker_id;
    let snapshot = db
        .run_domain_op(Box::new(move |conn| {
            let repo = DomainRepository::new(conn, project_id);
            let snapshot = repo.resolved_profile_snapshot(worker_id)?;
            Ok(serde_json::json!({ "snapshot": snapshot }))
        }))
        .await
        .map_err(|err| RegistryError::ProfileUnreadable(err.to_string()))?;
    let snapshot = snapshot
        .get("snapshot")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    let Some(snapshot) = snapshot else {
        return Err(RegistryError::NoResolvedProfile);
    };
    serde_json::from_str(&snapshot).map_err(|err| RegistryError::ProfileUnreadable(err.to_string()))
}

fn build_adapter(
    profile: &WorkerProfile,
    repo_root: &std::path::Path,
    run_id: RunId,
    task_id: TaskId,
    worker_id: WorkerId,
    mcp: Option<AdapterMcpConfig>,
    broker: Option<Arc<CoordinationBroker>>,
) -> Result<Arc<dyn Adapter>, RegistryError> {
    let adapter: Arc<dyn Adapter> = match &profile.startup_options {
        StartupOptions::Claude(options) => Arc::new(super::ClaudeAdapter::new(
            options.clone(),
            repo_root.to_path_buf(),
            profile.environment_allowlist.clone(),
            run_id,
            task_id,
            worker_id,
            mcp,
        )),
        StartupOptions::Codex(options) => Arc::new(super::CodexAdapter::new(
            repo_root.to_path_buf(),
            options.clone(),
            profile.environment_allowlist.clone(),
            mcp,
        )),
        StartupOptions::Copilot(options) => Arc::new(super::CopilotAdapter::new(
            PathBuf::from("copilot"),
            repo_root.to_path_buf(),
            options.clone(),
            profile.environment_allowlist.clone(),
            run_id,
            task_id,
            worker_id,
            mcp,
        )),
        StartupOptions::OmpRpc(_) => Arc::new(super::OmpRpcAdapter::new(
            profile.clone(),
            super::OmpRpcAdapterOptions::default(),
            broker,
        )),
        StartupOptions::TerminalDegraded(opts) => {
            Arc::new(super::terminal::TerminalAdapter::new(opts.backend.clone()))
                as Arc<dyn super::r#trait::Adapter>
        }
    };
    Ok(adapter)
}

#[cfg(test)]
mod build_adapter_tests {
    //! Unit tests for the private [`build_adapter`] function, reachable
    //! only from inside this crate (an external integration test crate
    //! cannot call it). These deliberately never call `.start()` on the
    //! returned adapter -- for Claude/Codex/Copilot that would spawn a
    //! real vendor CLI and, for Claude specifically, immediately send a
    //! real (billed) model turn (see `ClaudeAdapter::start`'s own
    //! `build_stdin_user_message` call) -- so they can only prove that
    //! `build_adapter` accepts and threads an `Option<AdapterMcpConfig>`/
    //! `Option<Arc<CoordinationBroker>>` through to construction without
    //! erroring, not that the constructed adapter's own `start()` later
    //! *uses* it correctly. That mechanism is proven separately and
    //! thoroughly, with zero process spawn, by each adapter's own
    //! dedicated test suite (e.g. `tests/claude_adapter.rs`'s
    //! `mcp_injection_appends_mcp_config_after_native_discovery_args...`
    //! and `mcp_injection_env_carries_only_the_scope_token`).
    use super::*;
    use crate::adapter::profile::{
        ClaudeStartupOptions, CodexStartupOptions, CopilotStartupOptions,
    };
    use crate::coordination::ScopeTokenStore;

    fn mcp_config() -> AdapterMcpConfig {
        AdapterMcpConfig {
            scope_tokens: Arc::new(ScopeTokenStore::new()),
            project_id: batman_protocol::ProjectId::new(),
            batcave_path: PathBuf::from("/opt/batman/bin/batcave"),
            state_dir: std::env::temp_dir(),
            repository: std::env::temp_dir(),
        }
    }

    fn profile(startup_options: StartupOptions) -> WorkerProfile {
        WorkerProfile {
            id: super::super::profile::ProfileId::new(),
            adapter: "test".to_string(),
            model: "test-model".to_string(),
            permission_envelope: serde_json::json!({}),
            startup_options,
            environment_allowlist: Vec::new(),
            source: "test".to_string(),
        }
    }

    #[test]
    fn claude_branch_accepts_some_mcp_config() {
        let profile = profile(StartupOptions::Claude(ClaudeStartupOptions::default()));
        let result = build_adapter(
            &profile,
            std::path::Path::new("/tmp"),
            RunId::new(),
            TaskId::new(),
            WorkerId::new(),
            Some(mcp_config()),
            None,
        );
        assert!(
            result.is_ok(),
            "Claude branch must accept Some(mcp): {}",
            result.err().map(|e| e.to_string()).unwrap_or_default()
        );
    }

    #[test]
    fn codex_branch_accepts_some_mcp_config() {
        let profile = profile(StartupOptions::Codex(CodexStartupOptions::default()));
        let result = build_adapter(
            &profile,
            std::path::Path::new("/tmp"),
            RunId::new(),
            TaskId::new(),
            WorkerId::new(),
            Some(mcp_config()),
            None,
        );
        assert!(
            result.is_ok(),
            "Codex branch must accept Some(mcp): {}",
            result.err().map(|e| e.to_string()).unwrap_or_default()
        );
    }

    #[test]
    fn copilot_branch_accepts_some_mcp_config() {
        let profile = profile(StartupOptions::Copilot(CopilotStartupOptions::default()));
        let result = build_adapter(
            &profile,
            std::path::Path::new("/tmp"),
            RunId::new(),
            TaskId::new(),
            WorkerId::new(),
            Some(mcp_config()),
            None,
        );
        assert!(
            result.is_ok(),
            "Copilot branch must accept Some(mcp): {}",
            result.err().map(|e| e.to_string()).unwrap_or_default()
        );
    }
}
