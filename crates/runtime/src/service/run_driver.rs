//! The `RunDriver` seam: delegates adapter-backed run start to an injected
//! implementation. Production startup without an adapter registry has no
//! driver injected; `run/submit` then reports `adapter_unavailable` after
//! preserving the queued run. Orchestration tests inject [`FakeRunDriver`],
//! which drives `queued -> starting -> working` through the same domain
//! repository transitions a real adapter would use.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use batman_protocol::{ProjectId, RunId, RunState, TaskId, WorkerId};

use crate::db::DatabaseHandle;
use crate::domain::DomainRepository;

/// A boxed future returned by [`RunDriver::start`].
pub type AdapterFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Everything a [`RunDriver`] needs to start (and subsequently transition) a
/// run through the durable domain repository.
#[derive(Clone)]
pub struct RunDriverContext {
    pub db: Arc<DatabaseHandle>,
    pub project_id: ProjectId,
    pub run_id: RunId,
    pub task_id: TaskId,
    pub worker_id: WorkerId,
}

/// Seam for starting an adapter-backed run. The (later) adapter registry
/// plan implements this against real harnesses; orchestration tests inject
/// [`FakeRunDriver`].
pub trait RunDriver: Send + Sync {
    /// Starts the run described by `ctx`. Implementations drive subsequent
    /// lifecycle transitions themselves (through the same domain repository
    /// commands), rather than returning a single terminal result.
    fn start(&self, ctx: RunDriverContext) -> AdapterFuture<'static, Result<(), String>>;
}

/// A deterministic driver for orchestration tests and fixtures: acknowledges
/// immediately and transitions `queued -> starting -> working`.
pub struct FakeRunDriver;

impl RunDriver for FakeRunDriver {
    fn start(&self, ctx: RunDriverContext) -> AdapterFuture<'static, Result<(), String>> {
        Box::pin(async move {
            transition(&ctx, "starting").await?;
            transition(&ctx, "working").await?;
            Ok(())
        })
    }
}

async fn transition(ctx: &RunDriverContext, to: &str) -> Result<(), String> {
    let to_state = RunState::try_from(to).map_err(|e| e.to_string())?;
    let project_id = ctx.project_id;
    let run_id = ctx.run_id;
    ctx.db
        .run_domain_op(Box::new(move |conn| {
            let mut repo = DomainRepository::new(conn, project_id);
            repo.transition_run(run_id, &to_state)
                .map(|committed| serde_json::json!({ "sequence": committed.sequence }))
        }))
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}
