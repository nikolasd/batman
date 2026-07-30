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
    ApprovalId, ApprovalRequest, BatmanMethod, DeliveryState, EventEnvelope, MessageId,
    MessageKind, ProjectId, Run, RunFlags, RunId, RunMessage, RunSpec, RunState, TaskId, TaskRef,
    Timestamp, Worker, WorkerId, WorkerProfileRef, error_code,
};
use serde_json::{Value, json};
use tokio::sync::broadcast;

use crate::db::DatabaseHandle;
use crate::domain::{
    DomainError, DomainRepository, TransitionError, embed_envelope, take_envelope,
};
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

impl From<crate::approval::ApprovalError> for ServiceError {
    fn from(err: crate::approval::ApprovalError) -> Self {
        use crate::approval::ApprovalError;
        match err {
            ApprovalError::Forbidden { .. } => Self {
                code: error_code::INVALID_PARAMS,
                message: err.to_string(),
            },
            ApprovalError::Conflict { .. } | ApprovalError::RunSettled { .. } => Self {
                code: error_code::INVALID_PARAMS,
                message: err.to_string(),
            },
            ApprovalError::NotFound { kind, id } => Self {
                code: error_code::INVALID_PARAMS,
                message: format!("{kind} {id} not found"),
            },
            ApprovalError::Domain(domain_err) => Self::from(domain_err),
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
    approval: Arc<crate::approval::ApprovalService>,
    events_tx: broadcast::Sender<EventEnvelope>,
}

impl OrchestrationService {
    #[must_use]
    pub fn new(
        db: Arc<DatabaseHandle>,
        project_id: ProjectId,
        run_driver: Option<Arc<dyn RunDriver>>,
        approval_callback: Arc<dyn crate::approval::ApprovalCallback>,
        events_tx: broadcast::Sender<EventEnvelope>,
    ) -> Self {
        let approval = Arc::new(crate::approval::ApprovalService::new(
            db.clone(),
            project_id,
            approval_callback,
            events_tx.clone(),
        ));
        Self {
            db,
            project_id,
            run_driver,
            approval,
            events_tx,
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
            BatmanMethod::ApprovalDecide => self.approval_decide(principal, params).await,
            BatmanMethod::ReconcileOmp => self.reconcile_omp(principal, params).await,
            BatmanMethod::CoordinationChildList => {
                self.coordination_child_list(principal, params).await
            }
            BatmanMethod::CoordinationChildDecide => self.coordination_child_decide(params).await,
            BatmanMethod::ProfileRegister => self.profile_register(params).await,
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
        let mut sequence = self
            .db
            .run_domain_op(Box::new(move |conn| {
                let mut repo = DomainRepository::new(conn, project_id);
                repo.upsert_task(task_id, &task_ref)
                    .map(|c| embed_envelope(json!({ "sequence": c.sequence }), &c.envelope))
            }))
            .await
            .map_err(ServiceError::from)?;
        self.broadcast(&mut sequence);

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
        let parent_worker_id = params
            .get("parentWorkerId")
            .and_then(Value::as_str)
            .map(WorkerId::parse)
            .transpose()
            .map_err(|_| ServiceError::invalid_params("parentWorkerId is not a valid id"))?;

        let legacy_fields_present = ["fingerprint", "adapter", "model", "permissionEnvelope"]
            .iter()
            .any(|field| params.get(*field).is_some());

        let (fingerprint, adapter, model, permission_envelope, resolved_profile_json) =
            if let Some(profile_id_value) = params.get("profileId") {
                if legacy_fields_present {
                    return Err(ServiceError::invalid_params(
                        "profileId and fingerprint/adapter/model/permissionEnvelope are mutually exclusive",
                    ));
                }
                let profile_id_str = profile_id_value
                    .as_str()
                    .ok_or_else(|| ServiceError::invalid_params("profileId must be a string"))?;
                let profile_id = crate::adapter::ProfileId::parse(profile_id_str)
                    .map_err(|_| ServiceError::invalid_params("profileId is not a valid id"))?;
                let resolved = self
                    .db
                    .run_domain_op(Box::new(move |conn| {
                        crate::adapter::ProfileStore::get(&*conn, profile_id)
                            .map(|(profile, fingerprint)| {
                                json!({
                                    "fingerprint": fingerprint,
                                    "adapter": profile.adapter,
                                    "model": profile.model,
                                    "permissionEnvelope": profile.permission_envelope,
                                    // The full resolved profile snapshot,
                                    // copied verbatim into the worker row
                                    // -- see `create_worker_with_snapshot`.
                                    "fullProfile": profile,
                                })
                            })
                            .map_err(|err| DomainError::NotFound {
                                kind: "profile",
                                id: err.to_string(),
                            })
                    }))
                    .await
                    .map_err(|_| {
                        ServiceError::invalid_params(format!(
                            "profileId {profile_id} was not found"
                        ))
                    })?;
                (
                    resolved["fingerprint"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string(),
                    resolved["adapter"].as_str().unwrap_or_default().to_string(),
                    resolved["model"].as_str().unwrap_or_default().to_string(),
                    resolved["permissionEnvelope"].clone(),
                    Some(
                        serde_json::to_string(&resolved["fullProfile"])
                            .expect("a resolved WorkerProfile always serializes"),
                    ),
                )
            } else {
                let fingerprint = str_field(params, "fingerprint")?;
                let adapter = str_field(params, "adapter")?;
                let model = str_field(params, "model")?;
                let permission_envelope = params
                    .get("permissionEnvelope")
                    .cloned()
                    .unwrap_or(json!({}));
                if crate::adapter::AdapterKind::from_wire_name(&adapter).is_some() {
                    return Err(ServiceError {
                        code: error_code::PROFILE_REQUIRED,
                        message: format!(
                            "adapter {adapter:?} requires a resolved profileId; register one via profile/register"
                        ),
                    });
                }
                (fingerprint, adapter, model, permission_envelope, None)
            };

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
        let mut sequence = self
            .db
            .run_domain_op(Box::new(move |conn| {
                let mut repo = DomainRepository::new(conn, project_id);
                repo.create_worker_with_snapshot(&worker, resolved_profile_json)
                    .map(|c| embed_envelope(json!({ "sequence": c.sequence }), &c.envelope))
            }))
            .await
            .map_err(ServiceError::from)?;
        self.broadcast(&mut sequence);

        Ok(json!({
            "workerId": worker_id.to_string(),
            "sequence": sequence["sequence"],
        }))
    }

    // ------------------------------------------------- adapter profiles

    /// Validates and registers a [`crate::adapter::WorkerProfile`],
    /// returning its freshly minted `profileId` and content fingerprint.
    /// Deliberately outside the append-only `events` journal -- profile
    /// registration is configuration, not an orchestration fact (see
    /// `crate::adapter::profile_store`) -- so there is nothing to
    /// broadcast here.
    async fn profile_register(&self, params: &Value) -> Result<Value, ServiceError> {
        let mut profile: crate::adapter::WorkerProfile = serde_json::from_value(params.clone())
            .map_err(|err| {
                ServiceError::invalid_params(format!("invalid worker profile: {err}"))
            })?;
        profile.id = crate::adapter::ProfileId::new();
        let fingerprint = profile.fingerprint();
        let policy = crate::adapter::EffectivePolicy::baseline();

        self.db
            .run_domain_op(Box::new({
                let profile = profile.clone();
                let fingerprint = fingerprint.clone();
                move |conn| {
                    crate::adapter::ProfileStore::register(&*conn, &profile, &policy, &fingerprint)
                        .map(|()| Value::Null)
                        .map_err(|err| DomainError::NotFound {
                            kind: "profile registration rejected",
                            id: err.to_string(),
                        })
                }
            }))
            .await
            .map_err(|err| ServiceError::invalid_params(err.to_string()))?;

        Ok(json!({
            "profileId": profile.id.to_string(),
            "fingerprint": fingerprint,
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
        let prompt = params.get("prompt").and_then(Value::as_str).map(str::to_string);
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
        let mut submit_result = self
            .db
            .run_domain_op(Box::new(move |conn| {
                let mut repo = DomainRepository::new(conn, project_id);
                repo.submit_run(&run)
                    .map(|c| embed_envelope(json!({ "sequence": c.sequence }), &c.envelope))
            }))
            .await
            .map_err(ServiceError::from)?;
        self.broadcast(&mut submit_result);

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
            prompt,
            events_tx: self.events_tx.clone(),
        };
        // Orchestration-test-scope: awaited synchronously so the caller
        // observes the final committed state deterministically.
        driver.start(ctx).await.map_err(ServiceError::internal)?;

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
        let mut sequence = self
            .db
            .run_domain_op(Box::new(move |conn| {
                let mut repo = DomainRepository::new(conn, project_id);
                repo.submit_run(&run)
                    .map(|c| embed_envelope(json!({ "sequence": c.sequence }), &c.envelope))
            }))
            .await
            .map_err(ServiceError::from)?;
        self.broadcast(&mut sequence);

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
        let mut result = self
            .db
            .run_domain_op(Box::new(move |conn| {
                let mut repo = DomainRepository::new(conn, project_id);
                repo.transition_run(run_id, &to)
                    .map(|c| embed_envelope(json!({ "sequence": c.sequence }), &c.envelope))
            }))
            .await
            .map_err(ServiceError::from)?;
        self.broadcast(&mut result);
        Ok(result)
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

        let follow_up_payload = payload.clone();
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
        let mut sequence = self
            .db
            .run_domain_op(Box::new(move |conn| {
                let mut repo = DomainRepository::new(conn, project_id);
                repo.record_message(&message)
                    .map(|c| embed_envelope(json!({ "sequence": c.sequence }), &c.envelope))
            }))
            .await
            .map_err(ServiceError::from)?;
        self.broadcast(&mut sequence);

        // Best-effort live delivery to an already-running adapter. A
        // missing driver, a `queued`/not-yet-started run (the normal case
        // -- `NoRunningAdapter`), or any other delivery failure must never
        // fail this RPC or unwind the message already durably recorded
        // above: the message stays `recorded` and the run's state is
        // untouched, matching `RunDriver::send_follow_up`'s own contract.
        if let Some(driver) = self.run_driver.clone()
            && let Err(err) = driver
                .send_follow_up(run_id, task_id, sender_worker_id, follow_up_payload)
                .await
            {
                let mut diagnostic = self
                    .db
                    .run_domain_op(Box::new(move |conn| {
                        let mut repo = DomainRepository::new(conn, project_id);
                        repo.record_diagnostic(
                            run_id,
                            batman_protocol::DiagnosticLevel::Warning,
                            "follow_up_delivery_failed",
                            format!("run {run_id}: {err}"),
                        )
                        .map(|c| embed_envelope(json!({ "sequence": c.sequence }), &c.envelope))
                    }))
                    .await
                    .map_err(ServiceError::from)?;
                self.broadcast(&mut diagnostic);
            }

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

    async fn approval_decide(
        &self,
        principal: &crate::ipc::ClientPrincipal,
        params: &Value,
    ) -> Result<Value, ServiceError> {
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

        let outcome = self
            .approval
            .decide(approval_id, &principal.instance_id, &decision, &reason)
            .await
            .map_err(ServiceError::from)?;

        Ok(json!({
            "approvalId": approval_id.to_string(),
            "outcome": match outcome {
                crate::approval::DecideOutcome::Decided => "decided",
                crate::approval::DecideOutcome::DecidedCallbackFailed => "decidedCallbackFailed",
                crate::approval::DecideOutcome::AlreadyDecided => "alreadyDecided",
            },
        }))
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
        let mut sequence = self
            .db
            .run_domain_op(Box::new(move |conn| {
                let mut repo = DomainRepository::new(conn, project_id);
                repo.reconcile_ownership(task_id, &new_owner, revision)
                    .map(|c| embed_envelope(json!({ "sequence": c.sequence }), &c.envelope))
            }))
            .await
            .map_err(ServiceError::from)?;
        self.broadcast(&mut sequence);

        Ok(json!({
            "taskId": task_id.to_string(),
            "newOwnerClientInstanceId": principal.instance_id,
            "sequence": sequence["sequence"],
        }))
    }

    // ---------------------------------------------------- coordination

    /// `coordination/child/list`: pending child-worker requests. A
    /// `workerMcp` principal sees only its own scoped run's request;
    /// `ompExtension`/`display` see every pending request in the project.
    async fn coordination_child_list(
        &self,
        principal: &crate::ipc::ClientPrincipal,
        params: &Value,
    ) -> Result<Value, ServiceError> {
        let scoped_run_id = principal.scoped_run_id;
        let requested_run_id = params
            .get("runId")
            .and_then(Value::as_str)
            .map(RunId::parse)
            .transpose()
            .map_err(|_| ServiceError::invalid_params("runId is not a valid id"))?;
        let run_filter = match scoped_run_id {
            Some(run_id) => Some(run_id),
            None => requested_run_id,
        };

        self.db
            .run_domain_op(Box::new(move |conn| {
                let (sql, params): (&str, Vec<String>) = match run_filter {
                    Some(run_id) => (
                        "SELECT sequence, event_json FROM events
                         WHERE run_id = ?1 AND event_json LIKE '%\"childEvent\"%'
                         ORDER BY sequence",
                        vec![run_id.to_string()],
                    ),
                    None => (
                        "SELECT sequence, event_json FROM events
                         WHERE event_json LIKE '%\"childEvent\"%' ORDER BY sequence",
                        vec![],
                    ),
                };
                let mut stmt = conn.prepare(sql)?;
                let rows: Vec<Value> = stmt
                    .query_map(rusqlite::params_from_iter(params.iter()), |row| {
                        row.get::<_, String>(1)
                    })?
                    .filter_map(|r| r.ok())
                    .filter_map(|json_text| serde_json::from_str::<Value>(&json_text).ok())
                    .collect();
                Ok(json!({ "requests": rows }))
            }))
            .await
            .map_err(ServiceError::from)
    }

    /// `coordination/child/decide`: OMP's answer to a prior
    /// `coordination/requestChild`. Acceptance supplies the OMP-created
    /// child ids and returns the parent run to `working`; denial records
    /// a reason and also returns the parent to `working`.
    async fn coordination_child_decide(&self, params: &Value) -> Result<Value, ServiceError> {
        let parent_run_id = parse_run_id(params.get("parentRunId"))?;
        let decision = str_field(params, "decision")?;
        let project_id = self.project_id;

        match decision.as_str() {
            "accept" => {
                let child_task_id = parse_task_id(params.get("childTaskId"))?;
                let child_worker_id = parse_worker_id(params.get("childWorkerId"))?;
                let child_run_id = parse_run_id(params.get("childRunId"))?;
                let mut result = self
                    .db
                    .run_domain_op(Box::new(move |conn| {
                        let mut repo = DomainRepository::new(conn, project_id);
                        repo.decide_child(
                            parent_run_id,
                            true,
                            Some(child_task_id),
                            Some(child_worker_id),
                            Some(child_run_id),
                            None,
                        )
                        .map(|c| embed_envelope(json!({ "sequence": c.sequence }), &c.envelope))
                    }))
                    .await
                    .map_err(ServiceError::from)?;
                self.broadcast(&mut result);
                Ok(result)
            }
            "deny" => {
                let reason = str_field(params, "reason")?;
                let mut result = self
                    .db
                    .run_domain_op(Box::new(move |conn| {
                        let mut repo = DomainRepository::new(conn, project_id);
                        repo.decide_child(parent_run_id, false, None, None, None, Some(&reason))
                            .map(|c| embed_envelope(json!({ "sequence": c.sequence }), &c.envelope))
                    }))
                    .await
                    .map_err(ServiceError::from)?;
                self.broadcast(&mut result);
                Ok(result)
            }
            other => Err(ServiceError::invalid_params(format!(
                "decision must be \"accept\" or \"deny\", got {other:?}"
            ))),
        }
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
    params.get(field).and_then(Value::as_u64).ok_or_else(|| {
        ServiceError::invalid_params(format!(
            "{field} is required and must be a non-negative integer"
        ))
    })
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
