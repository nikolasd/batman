//! The correlated approval flow: request creation paired with a run pause,
//! ownership-enforced decisions, and adapter-callback semantics that never
//! ask again on a failed callback -- they mark the run `protocolUnhealthy`
//! instead.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use batman_protocol::{ApprovalId, ApprovalRequest, EventEnvelope, ProjectId, RunFlags, RunId, RunState};
use serde_json::Value;
use tokio::sync::broadcast;

use crate::db::DatabaseHandle;
use crate::domain::{embed_envelope, take_envelope, DomainError, DomainRepository};

/// A boxed future returned by [`ApprovalCallback::acknowledge`].
pub type CallbackFuture<'a> = Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>>;

/// The adapter-callback seam invoked after a decision is recorded. The
/// (later) adapter registry plan implements this against real harnesses;
/// [`NoopApprovalCallback`] acknowledges immediately for tests and
/// fixtures without an adapter, and a test-injected failing callback
/// exercises the `protocolUnhealthy` path.
pub trait ApprovalCallback: Send + Sync {
    fn acknowledge(&self, approval_id: ApprovalId, decision: &str) -> CallbackFuture<'static>;
}

/// Acknowledges every callback immediately. The default when no adapter
/// registry is wired up.
pub struct NoopApprovalCallback;

impl ApprovalCallback for NoopApprovalCallback {
    fn acknowledge(&self, _approval_id: ApprovalId, _decision: &str) -> CallbackFuture<'static> {
        Box::pin(async { Ok(()) })
    }
}

/// Errors returned by [`ApprovalService`] operations.
#[derive(Debug, thiserror::Error)]
pub enum ApprovalError {
    /// The requesting principal does not own the task this approval
    /// belongs to.
    #[error("principal {instance_id} does not own the task for approval {approval_id}")]
    Forbidden {
        instance_id: String,
        approval_id: ApprovalId,
    },
    /// The approval already has a decision that conflicts with the one
    /// requested.
    #[error("approval {approval_id} already has a conflicting decision")]
    Conflict { approval_id: ApprovalId },
    /// The run this approval belongs to has already settled (reached a
    /// terminal state); a decision cannot target it.
    #[error("run {run_id} has already settled; cannot decide approval {approval_id}")]
    RunSettled { approval_id: ApprovalId, run_id: RunId },
    /// A referenced record was not found.
    #[error("{kind} {id} not found")]
    NotFound { kind: &'static str, id: String },
    /// A domain-layer command failed.
    #[error(transparent)]
    Domain(#[from] DomainError),
}

/// The outcome of [`ApprovalService::decide`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecideOutcome {
    /// A new decision was recorded and the callback succeeded: the run
    /// returned to `working`.
    Decided,
    /// A new decision was recorded but the callback failed: the decision
    /// is kept and the run is marked `protocolUnhealthy` instead of being
    /// asked again.
    DecidedCallbackFailed,
    /// An identical decision was already on record; this call is a no-op.
    AlreadyDecided,
}

/// Routes approval creation and decisions through the domain repository,
/// enforcing ownership, idempotency, and the settled-run invariant that
/// only the domain repository's mechanical layer does not itself know.
pub struct ApprovalService {
    db: Arc<DatabaseHandle>,
    project_id: ProjectId,
    callback: Arc<dyn ApprovalCallback>,
    events_tx: broadcast::Sender<EventEnvelope>,
}

impl ApprovalService {
    #[must_use]
    pub fn new(
        db: Arc<DatabaseHandle>,
        project_id: ProjectId,
        callback: Arc<dyn ApprovalCallback>,
        events_tx: broadcast::Sender<EventEnvelope>,
    ) -> Self {
        Self {
            db,
            project_id,
            callback,
            events_tx,
        }
    }

    /// Called when an adapter reports it needs approval for `action`.
    /// Atomically creates the request and transitions the run
    /// `working -> waitingUser`.
    ///
    /// # Errors
    /// Returns [`ApprovalError::Domain`] if the run does not exist or is
    /// not in `working` state.
    pub async fn request(&self, approval: ApprovalRequest) -> Result<(), ApprovalError> {
        let project_id = self.project_id;
        let mut result = self
            .db
            .run_domain_op(Box::new(move |conn| {
                let mut repo = DomainRepository::new(conn, project_id);
                repo.create_approval(&approval)
                    .map(|c| embed_envelope(serde_json::json!({ "sequence": c.sequence }), &c.envelope))
            }))
            .await
            .map_err(ApprovalError::Domain)?;
        self.broadcast(&mut result);
        Ok(())
    }

    /// `approval/decide`: records `decision` for `approval_id` after
    /// verifying `principal_instance_id` owns the approval's task, the
    /// decision does not conflict with a prior one, and the run has not
    /// already settled. Invokes the configured [`ApprovalCallback`] after
    /// recording; a failed callback keeps the decision and marks the run
    /// `protocolUnhealthy` rather than asking again.
    ///
    /// # Errors
    /// Returns [`ApprovalError::Forbidden`] if `principal_instance_id`
    /// does not own the task, [`ApprovalError::Conflict`] if a different
    /// decision is already on record, and [`ApprovalError::RunSettled`]
    /// if the run has already reached a terminal state.
    pub async fn decide(
        &self,
        approval_id: ApprovalId,
        principal_instance_id: &str,
        decision: &str,
        reason: &str,
    ) -> Result<DecideOutcome, ApprovalError> {
        let snapshot = self.load_snapshot(approval_id).await?;

        if snapshot.owner_client_instance_id != principal_instance_id {
            return Err(ApprovalError::Forbidden {
                instance_id: principal_instance_id.to_string(),
                approval_id,
            });
        }

        if let Some(existing) = &snapshot.decision {
            return if existing == decision {
                Ok(DecideOutcome::AlreadyDecided)
            } else {
                Err(ApprovalError::Conflict { approval_id })
            };
        }

        let run_state = RunState::try_from(snapshot.run_state.as_str())
            .map_err(|_| ApprovalError::NotFound { kind: "run-state", id: snapshot.run_state.clone() })?;
        if run_state.is_terminal() {
            return Err(ApprovalError::RunSettled {
                approval_id,
                run_id: snapshot.run_id,
            });
        }

        let project_id = self.project_id;
        let decision_owned = decision.to_string();
        let reason_owned = reason.to_string();
        let mut decide_result = self
            .db
            .run_domain_op(Box::new(move |conn| {
                let mut repo = DomainRepository::new(conn, project_id);
                repo.decide_approval(approval_id, &decision_owned, &reason_owned)
                    .map(|c| embed_envelope(serde_json::json!({ "sequence": c.sequence }), &c.envelope))
            }))
            .await
            .map_err(ApprovalError::Domain)?;
        self.broadcast(&mut decide_result);

        match self.callback.acknowledge(approval_id, decision).await {
            Ok(()) => {
                let run_id = snapshot.run_id;
                let working = RunState::try_from("working").expect("working is valid");
                let mut result = self
                    .db
                    .run_domain_op(Box::new(move |conn| {
                        let mut repo = DomainRepository::new(conn, project_id);
                        repo.transition_run(run_id, &working)
                            .map(|c| embed_envelope(serde_json::json!({ "sequence": c.sequence }), &c.envelope))
                    }))
                    .await
                    .map_err(ApprovalError::Domain)?;
                self.broadcast(&mut result);
                Ok(DecideOutcome::Decided)
            }
            Err(_) => {
                let run_id = snapshot.run_id;
                let mut flags = snapshot.run_flags;
                flags.protocol_unhealthy = true;
                let mut result = self
                    .db
                    .run_domain_op(Box::new(move |conn| {
                        let mut repo = DomainRepository::new(conn, project_id);
                        repo.set_run_flags(run_id, &flags)
                            .map(|c| embed_envelope(serde_json::json!({ "sequence": c.sequence }), &c.envelope))
                    }))
                    .await
                    .map_err(ApprovalError::Domain)?;
                self.broadcast(&mut result);
                Ok(DecideOutcome::DecidedCallbackFailed)
            }
        }
    }

    /// Broadcasts the envelope embedded by a mutation's `run_domain_op`
    /// closure to live subscribers, if present, then strips it so the
    /// caller's JSON-RPC response never carries the internal key.
    fn broadcast(&self, value: &mut Value) {
        if let Some(envelope) = take_envelope(value) {
            let _ = self.events_tx.send(envelope);
        }
    }

    async fn load_snapshot(&self, approval_id: ApprovalId) -> Result<ApprovalSnapshot, ApprovalError> {
        let value: Value = self
            .db
            .run_domain_op(Box::new(move |conn| {
                conn.query_row(
                    "SELECT a.run_id, a.decision, t.owner_client_instance_id, r.state,
                            r.flags_degraded_control, r.flags_needs_reconciliation, r.flags_protocol_unhealthy,
                            r.flags_policy_quarantined, r.flags_workspace_dirty, r.flags_children_active
                     FROM approvals a
                     JOIN runs r ON a.run_id = r.run_id
                     JOIN tasks t ON a.task_id = t.task_id
                     WHERE a.approval_id = ?1",
                    [approval_id.to_string()],
                    |row| {
                        Ok(serde_json::json!({
                            "runId": row.get::<_, String>(0)?,
                            "decision": row.get::<_, Option<String>>(1)?,
                            "ownerClientInstanceId": row.get::<_, String>(2)?,
                            "runState": row.get::<_, String>(3)?,
                            "flags": {
                                "degradedControl": row.get::<_, i64>(4)? != 0,
                                "needsReconciliation": row.get::<_, i64>(5)? != 0,
                                "protocolUnhealthy": row.get::<_, i64>(6)? != 0,
                                "policyQuarantined": row.get::<_, i64>(7)? != 0,
                                "workspaceDirty": row.get::<_, i64>(8)? != 0,
                                "childrenActive": row.get::<_, i64>(9)? != 0,
                            }
                        }))
                    },
                )
                .map_err(|_| DomainError::NotFound { kind: "approval", id: approval_id.to_string() })
            }))
            .await
            .map_err(ApprovalError::Domain)?;

        Ok(ApprovalSnapshot {
            run_id: RunId::parse(value["runId"].as_str().unwrap_or_default())
                .map_err(|_| ApprovalError::NotFound { kind: "run", id: "invalid".to_string() })?,
            decision: value["decision"].as_str().map(str::to_string),
            owner_client_instance_id: value["ownerClientInstanceId"].as_str().unwrap_or_default().to_string(),
            run_state: value["runState"].as_str().unwrap_or_default().to_string(),
            run_flags: RunFlags {
                degraded_control: value["flags"]["degradedControl"].as_bool().unwrap_or(false),
                needs_reconciliation: value["flags"]["needsReconciliation"].as_bool().unwrap_or(false),
                protocol_unhealthy: value["flags"]["protocolUnhealthy"].as_bool().unwrap_or(false),
                policy_quarantined: value["flags"]["policyQuarantined"].as_bool().unwrap_or(false),
                workspace_dirty: value["flags"]["workspaceDirty"].as_bool().unwrap_or(false),
                children_active: value["flags"]["childrenActive"].as_bool().unwrap_or(false),
            },
        })
    }
}

struct ApprovalSnapshot {
    run_id: RunId,
    decision: Option<String>,
    owner_client_instance_id: String,
    run_state: String,
    run_flags: RunFlags,
}
