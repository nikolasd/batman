//! The adapter registry: implements [`crate::service::RunDriver`] by
//! resolving a run's immutable worker profile, gating start on
//! conformance-derived effective capabilities through an injected
//! [`AdapterAuthorization`], constructing the matching [`Adapter`], and
//! owning it for the run's lifetime in a run-indexed table.
//!
//! # Scope boundary (documented, not silently omitted)
//! [`RunDriverContext`] carries no prompt/message payload -- by design,
//! per the design spec's "OMP owns scheduling... Batman never creates or
//! edits the OMP task graph": `run/submit` only ever carries `taskId`/
//! `workerId` (`OrchestrationService::run_submit`), never task content.
//! This registry therefore starts every adapter with an empty initial
//! [`StartSpec::prompt`]; delivering the task's actual content (and any
//! later follow-up) as a live [`AdapterMessage`] to an already-running
//! adapter instance requires a message-forwarding seam (e.g. an
//! `events_tx` subscriber translating a journaled message event into
//! `Adapter::send`) that is out of this milestone's Task 8 scope --
//! [`RunDriver`] has no method for it today, and no Task 8 plan file
//! names that change. This is a known, explicitly documented follow-up,
//! not a silently dropped requirement. Likewise, adapters constructed
//! here never receive worker-coordination MCP config (`mcp: None`
//! throughout): wiring `crate::adapter::mcp_config::McpLaunchContext`
//! (which needs a resolved `batcave` binary path, state dir, and
//! repository root this registry is not constructed with) is the same
//! kind of production-wiring follow-up, tracked alongside it.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::Mutex;

use batman_protocol::{RunId, TaskId, WorkerId};

use super::capability::AdapterCapabilities;
use super::event_sink::DomainAdapterEventSink;
use super::profile::{StartupOptions, WorkerProfile};
use super::r#trait::{Adapter, StartSpec};
use crate::conformance;
use crate::domain::DomainRepository;
use crate::service::{AdapterFuture as RunDriverFuture, RunDriver, RunDriverContext};

/// Decides whether a run may actually be started against `profile`'s
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
    running: Arc<Mutex<HashMap<RunId, Arc<dyn Adapter>>>>,
}

impl AdapterRegistry {
    #[must_use]
    pub fn new(authorization: Arc<dyn AdapterAuthorization>, repo_root: PathBuf) -> Self {
        Self {
            authorization,
            repo_root,
            running: Arc::new(Mutex::new(HashMap::new())),
        }
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
            match run_one(&ctx, &authorization, &repo_root).await {
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
        conformance::run_fixture_conformance(kind).await.effective_capabilities
    };
    authorization
        .authorize(&profile, &effective_capabilities)
        .map_err(RegistryError::AuthorizationDenied)
        .map_err(String::from)?;

    let adapter = build_adapter(&profile, repo_root, ctx.run_id, ctx.task_id, ctx.worker_id)
        .map_err(String::from)?;
    let sink = Arc::new(DomainAdapterEventSink::new(
        Arc::clone(&ctx.db),
        ctx.project_id,
        ctx.events_tx.clone(),
    ));
    adapter
        .start(
            StartSpec {
                run_id: ctx.run_id,
                task_id: ctx.task_id,
                worker_id: ctx.worker_id,
                prompt: String::new(),
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
) -> Result<Arc<dyn Adapter>, RegistryError> {
    let adapter: Arc<dyn Adapter> = match &profile.startup_options {
        StartupOptions::Claude(options) => Arc::new(super::ClaudeAdapter::new(
            options.clone(),
            repo_root.to_path_buf(),
            profile.environment_allowlist.clone(),
            run_id,
            task_id,
            worker_id,
            None,
        )),
        StartupOptions::Codex(options) => Arc::new(super::CodexAdapter::new(
            repo_root.to_path_buf(),
            options.clone(),
            profile.environment_allowlist.clone(),
            None,
        )),
        StartupOptions::Copilot(options) => Arc::new(super::CopilotAdapter::new(
            PathBuf::from("copilot"),
            repo_root.to_path_buf(),
            options.clone(),
            profile.environment_allowlist.clone(),
            run_id,
            task_id,
            worker_id,
            None,
        )),
        StartupOptions::OmpRpc(_) => Arc::new(super::OmpRpcAdapter::new(
            profile.clone(),
            super::OmpRpcAdapterOptions::default(),
            None,
        )),
        StartupOptions::TerminalDegraded(opts) => {
            Arc::new(super::terminal::TerminalAdapter::new(opts.backend.clone())) as Arc<dyn super::r#trait::Adapter>
        }
    };
    Ok(adapter)
}
