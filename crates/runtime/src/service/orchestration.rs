//! The orchestration RPC service: routes every Task 1 method to typed
//! [`DomainRepository`] commands (mutations) or read-only query closures
//! (lookups), translating results and errors to JSON-RPC shapes.
//!
//! OMP remains authoritative for the task graph, scheduling, and policy;
//! this service only persists OMP-supplied intent and enforces run-lifecycle
//! and ownership invariants that only the runtime can see (process/protocol
//! evidence, monotonic revision, connected-instance identity).

use std::sync::Arc;

use batman_protocol::{
    ApprovalDecision as WireApprovalDecision, ApprovalId, ApprovalRequest, BatmanMethod,
    DeliveryState, MessageId, MessageKind, ProjectId, Run, RunFlags, RunId, RunMessage, RunSpec,
    RunState, TaskId, TaskRef, Timestamp, Worker, WorkerId, WorkerProfileRef, error_code,
};
use serde_json::{Value, json};

use crate::db::DatabaseHandle;
use crate::domain::{DomainError, DomainRepository, TransitionError};
use crate::ipc::ClientPrincipal;

use super::query;
use super::run_driver::{RunDriver, RunDriverContext};

/// A JSON-RPC-shaped error: `(code, message)`, mapped directly onto the
/// wire error object by the connection dispatch layer.
#[derive(Debug)]
pub struct ServiceError {
    pub code: i32,
    pub message: String,
}

impl ServiceError {
    fn invalid_params(msg: impl Into<String>) -> Self {
        Self {
            code: error_code::INVALID_PARAMS,
            message: msg.into(),
        }
    }

    fn internal(msg: impl Into<String>) -> Self {
        Self {
            code: error_code::INTERNAL_ERROR,
            message: msg.into(),
        }
    }
}

impl From<DomainError> for ServiceError {
    fn from(err: DomainError) -> Self {
        match err {
            DomainError::Transition(TransitionError::Illegal { run_id, from, to }) => Self {
                code: error_code::ILLEGAL_TRANSITION,
                message: format!("illegal run transition for {run_id}: {from} -> {to}"),
            },
            DomainError::NotFound { kind, id } => Self {
                code: error_code::INVALID_PARAMS,
                message: format!("{kind} {id} not found"),
            },
            other => Self::internal(other.to_string()),
        }
    }
}

/// Routes every orchestration method to the domain repository. Holds no
/// mutable state itself; every command borrows the shared
/// [`DatabaseHandle`] and commits on the actor thread.
pub struct OrchestrationService {
    db: Arc<DatabaseHandle>,
    project_id: ProjectId,
    run_driver: Option<Arc<dyn RunDriver>>,
}

impl OrchestrationService {
    #[must_use]
    pub fn new(
        db: Arc<DatabaseHandle>,
        project_id: ProjectId,
        run_driver: Option<Arc<dyn RunDriver>>,
    ) -> Self {
        Self {
            db,
            project_id,
            run_driver,
        }
    }

    /// Dispatches one already role-authorized orchestration method.
    /// `principal` is consulted for ownership checks (`reconcile/omp`,
    /// future approval ownership); role admission itself already happened
    /// in the connection layer's method table.
    pub async fn dispatch(
        &self,
        method: BatmanMethod,
        principal: &ClientPrincipal,
        params: &Value,
    ) -> Result<Value, ServiceError> {
        match method {
            BatmanMethod::TaskUpsert => self.task_upsert(params).await,
            BatmanMethod::TaskGet => self.task_get(params).await,
            BatmanMethod::WorkerCreate => self.worker_create(params).await,
            BatmanMethod::WorkerList => self.worker_list().await,
            BatmanMethod::WorkerGet => self.worker_get(params).await,
            BatmanMethod::RunSubmit => self.run_submit(params).await,
            BatmanMethod::RunList => self.run_list(params).await,
            BatmanMethod::RunGet => self.run_get(params).await,
            BatmanMethod::RunRetry => self.run_retry(params).await,
            BatmanMethod::RunCancel => self.run_cancel(params).await,
            BatmanMethod::MessageSend => self.message_send(params).await,
            BatmanMethod::MessageList => self.message_list(params).await,
            BatmanMethod::ApprovalList => self.approval_list(params).await,
            BatmanMethod::ApprovalDecide => self.approval_decide(params).await,
            BatmanMethod::ReconcileOmp => self.reconcile_omp(principal, params).await,
            _ => Err(ServiceError::internal(
                "method is not routed through OrchestrationService",
            )),
        }
    }

    // ------------------------------------------------------------- task

    async fn task_upsert(&self, params: &Value) -> Result<Value, ServiceError> {
        let task_id = parse_or_new_task_id(params.get("taskId"))?;
        let owner = str_field(params, "ownerClientInstanceId")?;
        let revision = u64_field(params, "revision")?;

        // Idempotency / monotonicity: reject a lower revision than what is
        // already stored; an identical revision is a no-op success.
        let existing = self.db.run_domain_op(query::task_get_op(task_id)).await;
        if let Ok(existing) = existing {
            let stored_revision = existing["revision"].as_u64().unwrap_or(0);
            if revision < stored_revision {
                return Err(ServiceError::invalid_params(format!(
                    "revision {revision} is lower than stored revision {stored_revision}"
                )));
            }
        }

        let task_ref = TaskRef {
            owner_client_instance_id: owner,
            revision,
        };
        let project_id = self.project_id;
        let sequence = self
            .db
            .run_domain_op(Box::new(move |conn| {
                let mut repo = DomainRepository::new(conn, project_id);
                repo.upsert_task(task_id, &task_ref)
                    .map(|c| json!({ "sequence": c.sequence }))
            }))
            .await
            .map_err(ServiceError::from)?;

        Ok(json!({
            "taskId": task_id.to_string(),
            "sequence": sequence["sequence"],
        }))
    }

    async fn task_get(&self, params: &Value) -> Result<Value, ServiceError> {
        let task_id = parse_task_id(params.get("taskId"))?;
        self.db
            .run_domain_op(query::task_get_op(task_id))
            .await
            .map_err(ServiceError::from)
    }

    // ----------------------------------------------------------- worker

    async fn worker_create(&self, params: &Value) -> Result<Value, ServiceError> {
        let fingerprint = str_field(params, "fingerprint")?;
        let adapter = str_field(params, "adapter")?;
        let model = str_field(params, "model")?;
        let permission_envelope = params
            .get("permissionEnvelope")
            .cloned()
            .unwrap_or(json!({}));
        let parent_worker_id = params
            .get("parentWorkerId")
            .and_then(Value::as_str)
            .map(WorkerId::parse)
            .transpose()
            .map_err(|_| ServiceError::invalid_params("parentWorkerId is not a valid id"))?;

        let worker_id = WorkerId::new();
        let profile = WorkerProfileRef {
            id: worker_id,
            fingerprint,
            adapter,
            model,
            permission_envelope,
        };
        let worker = Worker {
            worker_id,
            profile_ref: profile,
            parent_worker_id,
            created_at: Timestamp::now(),
        };

        let project_id = self.project_id;
        let sequence = self
            .db
            .run_domain_op(Box::new(move |conn| {
                let mut repo = DomainRepository::new(conn, project_id);
                repo.create_worker(&worker)
                    .map(|c| json!({ "sequence": c.sequence }))
            }))
            .await
            .map_err(ServiceError::from)?;

        Ok(json!({
            "workerId": worker_id.to_string(),
            "sequence": sequence["sequence"],
        }))
    }

    async fn worker_list(&self) -> Result<Value, ServiceError> {
        self.db
            .run_domain_op(query::worker_list_op(self.project_id))
            .await
            .map_err(ServiceError::from)
    }

    async fn worker_get(&self, params: &Value) -> Result<Value, ServiceError> {
        let worker_id = parse_worker_id(params.get("workerId"))?;
        self.db
            .run_domain_op(query::worker_get_op(worker_id))
            .await
            .map_err(ServiceError::from)
    }

    // -------------------------------------------------------------- run

    async fn run_submit(&self, params: &Value) -> Result<Value, ServiceError> {
        let task_id = parse_task_id(params.get("taskId"))?;
        let worker_id = parse_worker_id(params.get("workerId"))?;
        // Cross-project rejection: the task must exist in this project.
        self.db
            .run_domain_op(query::task_get_op(task_id))
            .await
            .map_err(|_| {
                ServiceError::invalid_params(format!("task {task_id} not found in this project"))
            })?;

        let run_id = RunId::new();
        let run = Run {
            run_id,
            task_id,
            worker_id,
            state: RunState::try_from("queued").expect("queued is a valid RunState"),
            flags: RunFlags::default(),
            vendor_session_id: None,
            started_at: None,
            completed_at: None,
        };

        let project_id = self.project_id;
        let submit_result = self
            .db
            .run_domain_op(Box::new(move |conn| {
                let mut repo = DomainRepository::new(conn, project_id);
                repo.submit_run(&run)
                    .map(|c| json!({ "sequence": c.sequence }))
            }))
            .await
            .map_err(ServiceError::from)?;

        let Some(driver) = self.run_driver.clone() else {
            // The queued run is preserved; the caller learns the adapter
            // registry is unavailable without a fabricated "started" state.
            return Err(ServiceError {
                code: error_code::ADAPTER_UNAVAILABLE,
                message: "adapter_unavailable".to_string(),
            });
        };

        let ctx = RunDriverContext {
            db: self.db.clone(),
            project_id,
            run_id,
            task_id,
            worker_id,
        };
        // Orchestration-test-scope: awaited synchronously so the caller
        // observes the final committed state deterministically.
        driver
            .start(ctx)
            .await
            .map_err(ServiceError::internal)?;

        Ok(json!({
            "runId": run_id.to_string(),
            "taskId": task_id.to_string(),
            "sequence": submit_result["sequence"],
        }))
    }

    async fn run_list(&self, params: &Value) -> Result<Value, ServiceError> {
        let task_id = params
            .get("taskId")
            .and_then(Value::as_str)
            .map(TaskId::parse)
            .transpose()
            .map_err(|_| ServiceError::invalid_params("taskId is not a valid id"))?;
        self.db
            .run_domain_op(query::run_list_op(task_id, self.project_id))
            .await
            .map_err(ServiceError::from)
    }

    async fn run_get(&self, params: &Value) -> Result<Value, ServiceError> {
        let run_id = parse_run_id(params.get("runId"))?;
        self.db
            .run_domain_op(query::run_get_op(run_id))
            .await
            .map_err(ServiceError::from)
    }

    /// `run/retry` takes the prior `RunId` and a new `WorkerId`; the result
    /// always contains a distinct `RunId` and the same `TaskId`. The prior
    /// run must be in a terminal state (no in-place resurrection).
    async fn run_retry(&self, params: &Value) -> Result<Value, ServiceError> {
        let prior_run_id = parse_run_id(params.get("priorRunId"))?;
        let worker_id = parse_worker_id(params.get("workerId"))?;

        let prior = self
            .db
            .run_domain_op(query::run_get_op(prior_run_id))
            .await
            .map_err(ServiceError::from)?;
        let task_id = TaskId::parse(prior["taskId"].as_str().unwrap_or_default())
            .map_err(|_| ServiceError::internal("stored run has an invalid taskId"))?;
        let prior_state = prior["state"].as_str().unwrap_or_default();
        let prior_is_terminal = RunState::try_from(prior_state)
            .map(|s| s.is_terminal())
            .unwrap_or(false);
        if !prior_is_terminal {
            return Err(ServiceError::invalid_params(format!(
                "run {prior_run_id} is not in a terminal state ({prior_state})"
            )));
        }

        let new_run_id = RunId::new();
        let run = Run {
            run_id: new_run_id,
            task_id,
            worker_id,
            state: RunState::try_from("queued").expect("queued is valid"),
            flags: RunFlags::default(),
            vendor_session_id: None,
            started_at: None,
            completed_at: None,
        };
        let project_id = self.project_id;
        let sequence = self
            .db
            .run_domain_op(Box::new(move |conn| {
                let mut repo = DomainRepository::new(conn, project_id);
                repo.submit_run(&run)
                    .map(|c| json!({ "sequence": c.sequence }))
            }))
            .await
            .map_err(ServiceError::from)?;

        Ok(json!({
            "runId": new_run_id.to_string(),
            "taskId": task_id.to_string(),
            "priorRunId": prior_run_id.to_string(),
            "sequence": sequence["sequence"],
        }))
    }

    /// OMP may request cancellation; the transition is applied only after
    /// this synchronous domain check succeeds (representing the runtime's
    /// own bookkeeping — a real adapter's acknowledgement is a Worker
    /// Adapters plan concern).
    async fn run_cancel(&self, params: &Value) -> Result<Value, ServiceError> {
        let run_id = parse_run_id(params.get("runId"))?;
        let project_id = self.project_id;
        let to = RunState::try_from("cancelled").expect("cancelled is valid");
        self.db
            .run_domain_op(Box::new(move |conn| {
                let mut repo = DomainRepository::new(conn, project_id);
                repo.transition_run(run_id, &to)
                    .map(|c| json!({ "sequence": c.sequence }))
            }))
            .await
            .map_err(ServiceError::from)
    }

    // ---------------------------------------------------------- message

    async fn message_send(&self, params: &Value) -> Result<Value, ServiceError> {
        let run_id = parse_run_id(params.get("runId"))?;
        let sender_worker_id = parse_worker_id(params.get("senderWorkerId"))?;
        let task_id = parse_task_id(params.get("taskId"))?;
        let kind = parse_message_kind(params.get("kind"))?;
        let payload = str_field(params, "payload")?;
        let recipient_worker_id = params
            .get("recipientWorkerId")
            .and_then(Value::as_str)
            .map(WorkerId::parse)
            .transpose()
            .map_err(|_| ServiceError::invalid_params("recipientWorkerId is not a valid id"))?;
        let reply_to = params
            .get("replyTo")
            .and_then(Value::as_str)
            .map(MessageId::parse)
            .transpose()
            .map_err(|_| ServiceError::invalid_params("replyTo is not a valid id"))?;

        let message_id = MessageId::new();
        let message = RunMessage {
            message_id,
            run_id,
            sender_worker_id,
            recipient_worker_id,
            task_id,
            kind,
            payload,
            delivery_state: DeliveryState::Recorded,
            created_at: Timestamp::now(),
            sent_at: None,
            acknowledged_at: None,
            reply_to,
        };
        let project_id = self.project_id;
        let sequence = self
            .db
            .run_domain_op(Box::new(move |conn| {
                let mut repo = DomainRepository::new(conn, project_id);
                repo.record_message(&message)
                    .map(|c| json!({ "sequence": c.sequence }))
            }))
            .await
            .map_err(ServiceError::from)?;

        Ok(json!({
            "messageId": message_id.to_string(),
            "sequence": sequence["sequence"],
        }))
    }

    async fn message_list(&self, params: &Value) -> Result<Value, ServiceError> {
        let run_id = parse_run_id(params.get("runId"))?;
        self.db
            .run_domain_op(query::message_list_op(run_id))
            .await
            .map_err(ServiceError::from)
    }

    // --------------------------------------------------------- approval

    async fn approval_list(&self, params: &Value) -> Result<Value, ServiceError> {
        let run_id = params
            .get("runId")
            .and_then(Value::as_str)
            .map(RunId::parse)
            .transpose()
            .map_err(|_| ServiceError::invalid_params("runId is not a valid id"))?;
        self.db
            .run_domain_op(query::approval_list_op(run_id))
            .await
            .map_err(ServiceError::from)
    }

    async fn approval_decide(&self, params: &Value) -> Result<Value, ServiceError> {
        let approval_id = params
            .get("approvalId")
            .and_then(Value::as_str)
            .ok_or_else(|| ServiceError::invalid_params("approvalId is required"))
            .and_then(|s| {
                ApprovalId::parse(s)
                    .map_err(|_| ServiceError::invalid_params("approvalId is not a valid id"))
            })?;
        let decision = str_field(params, "decision")?;
        if decision != "approve" && decision != "deny" {
            return Err(ServiceError::invalid_params(
                "decision must be \"approve\" or \"deny\"",
            ));
        }
        let reason = str_field(params, "reason")?;
        let _typed = WireApprovalDecision {
            decision: decision.clone(),
            reason: reason.clone(),
        };

        let project_id = self.project_id;
        self.db
            .run_domain_op(Box::new(move |conn| {
                let mut repo = DomainRepository::new(conn, project_id);
                repo.decide_approval(approval_id, &decision, &reason)
                    .map(|c| json!({ "sequence": c.sequence }))
            }))
            .await
            .map_err(ServiceError::from)
    }

    // -------------------------------------------------------- reconcile

    /// Rebinds a task from a disconnected OMP client instance to the
    /// connected `principal`, only when task ID and monotonic OMP revision
    /// match; journals the old/new owner IDs.
    async fn reconcile_omp(
        &self,
        principal: &ClientPrincipal,
        params: &Value,
    ) -> Result<Value, ServiceError> {
        let task_id = parse_task_id(params.get("taskId"))?;
        let revision = u64_field(params, "revision")?;

        let existing = self
            .db
            .run_domain_op(query::task_get_op(task_id))
            .await
            .map_err(ServiceError::from)?;
        let stored_revision = existing["revision"].as_u64().unwrap_or(0);
        if revision != stored_revision {
            return Err(ServiceError::invalid_params(format!(
                "revision {revision} does not match stored revision {stored_revision}"
            )));
        }

        let new_owner = principal.instance_id.clone();
        let project_id = self.project_id;
        let sequence = self
            .db
            .run_domain_op(Box::new(move |conn| {
                let mut repo = DomainRepository::new(conn, project_id);
                repo.reconcile_ownership(task_id, &new_owner, revision)
                    .map(|c| json!({ "sequence": c.sequence }))
            }))
            .await
            .map_err(ServiceError::from)?;

        Ok(json!({
            "taskId": task_id.to_string(),
            "newOwnerClientInstanceId": principal.instance_id,
            "sequence": sequence["sequence"],
        }))
    }
}

// ----------------------------------------------------------------- parsing

fn str_field(params: &Value, field: &'static str) -> Result<String, ServiceError> {
    params
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| ServiceError::invalid_params(format!("{field} is required")))
}

fn u64_field(params: &Value, field: &'static str) -> Result<u64, ServiceError> {
    params
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| ServiceError::invalid_params(format!("{field} is required and must be a non-negative integer")))
}

fn parse_task_id(value: Option<&Value>) -> Result<TaskId, ServiceError> {
    value
        .and_then(Value::as_str)
        .ok_or_else(|| ServiceError::invalid_params("taskId is required"))
        .and_then(|s| {
            TaskId::parse(s).map_err(|_| ServiceError::invalid_params("taskId is not a valid id"))
        })
}

fn parse_or_new_task_id(value: Option<&Value>) -> Result<TaskId, ServiceError> {
    match value.and_then(Value::as_str) {
        Some(s) => {
            TaskId::parse(s).map_err(|_| ServiceError::invalid_params("taskId is not a valid id"))
        }
        None => Ok(TaskId::new()),
    }
}

fn parse_worker_id(value: Option<&Value>) -> Result<WorkerId, ServiceError> {
    value
        .and_then(Value::as_str)
        .ok_or_else(|| ServiceError::invalid_params("workerId is required"))
        .and_then(|s| {
            WorkerId::parse(s)
                .map_err(|_| ServiceError::invalid_params("workerId is not a valid id"))
        })
}

fn parse_run_id(value: Option<&Value>) -> Result<RunId, ServiceError> {
    value
        .and_then(Value::as_str)
        .ok_or_else(|| ServiceError::invalid_params("runId is required"))
        .and_then(|s| {
            RunId::parse(s).map_err(|_| ServiceError::invalid_params("runId is not a valid id"))
        })
}

fn parse_message_kind(value: Option<&Value>) -> Result<MessageKind, ServiceError> {
    let raw = value
        .and_then(Value::as_str)
        .ok_or_else(|| ServiceError::invalid_params("kind is required"))?;
    match raw {
        "assign" => Ok(MessageKind::Assign),
        "steer" => Ok(MessageKind::Steer),
        "followUp" => Ok(MessageKind::FollowUp),
        "question" => Ok(MessageKind::Question),
        "answer" => Ok(MessageKind::Answer),
        "peerMessage" => Ok(MessageKind::PeerMessage),
        "approvalDecision" => Ok(MessageKind::ApprovalDecision),
        "cancel" => Ok(MessageKind::Cancel),
        "shutdown" => Ok(MessageKind::Shutdown),
        other => Err(ServiceError::invalid_params(format!(
            "unknown message kind {other:?}"
        ))),
    }
}

/// Suppresses an unused-import warning for a type referenced only through
/// generic bounds in this module's signatures.
#[allow(unused_imports)]
use ApprovalRequest as _ApprovalRequest;
#[allow(unused_imports)]
use RunSpec as _RunSpec;
