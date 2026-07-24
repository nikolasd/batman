//! The coordination broker: the worker-safe messaging and task-signal
//! surface a supervised vendor process uses through its scope-bound
//! connection.
//!
//! Record-before-delivery: every send commits `recorded` first (one
//! durable event + projection row), then attempts delivery and commits
//! the outcome (`sent`, `acknowledged`, `failed`, or `unknown`). A runtime
//! crash between the two commits leaves the message `sent`/`recorded` --
//! [`CoordinationBroker::sweep_unacknowledged_as_unknown`] settles any
//! message left in a non-terminal delivery state after recovery to
//! `unknown`; it never resends automatically.

use std::sync::Arc;

use batman_protocol::{
    error_code, DeliveryState, EventEnvelope, MessageId, MessageKind, ProjectId, RunId, RunMessage, TaskId,
    Timestamp, WorkerId, COORDINATION_PAYLOAD_MAX_BYTES,
};
use serde_json::{json, Value};
use tokio::sync::broadcast;

use crate::db::DatabaseHandle;
use crate::domain::{embed_envelope, take_envelope, DomainRepository};

use super::rate_limit::RateLimiter;

/// A JSON-RPC-shaped error, matching [`crate::service::ServiceError`]'s
/// shape so the connection dispatch layer can map either uniformly.
#[derive(Debug)]
pub struct CoordinationError {
    pub code: i32,
    pub message: String,
}

impl CoordinationError {
    fn invalid_params(msg: impl Into<String>) -> Self {
        Self {
            code: error_code::INVALID_PARAMS,
            message: msg.into(),
        }
    }
}

impl From<crate::domain::DomainError> for CoordinationError {
    fn from(err: crate::domain::DomainError) -> Self {
        Self {
            code: error_code::INTERNAL_ERROR,
            message: err.to_string(),
        }
    }
}

/// Routes the worker-safe `coordination/*` operations to the domain
/// repository, enforcing message bounds, reply visibility, task
/// ownership, and the per-sender rate limit before any journaling.
pub struct CoordinationBroker {
    db: Arc<DatabaseHandle>,
    project_id: ProjectId,
    rate_limiter: RateLimiter,
    events_tx: broadcast::Sender<EventEnvelope>,
}

impl CoordinationBroker {
    #[must_use]
    pub fn new(
        db: Arc<DatabaseHandle>,
        project_id: ProjectId,
        events_tx: broadcast::Sender<EventEnvelope>,
    ) -> Self {
        Self {
            db,
            project_id,
            rate_limiter: RateLimiter::default(),
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

    /// `coordination/send`: validates bounds, reply visibility, and task
    /// ownership, checks the rate limit, then records the message
    /// (`recorded`) and immediately marks it `sent` -- no adapter exists in
    /// this milestone to acknowledge it further, so it settles there until
    /// a future adapter integration acknowledges or fails it.
    pub async fn send(
        &self,
        run_id: RunId,
        sender_worker_id: WorkerId,
        task_id: TaskId,
        kind: MessageKind,
        payload: String,
        recipient_worker_id: Option<WorkerId>,
        reply_to: Option<MessageId>,
    ) -> Result<Value, CoordinationError> {
        if payload.len() > COORDINATION_PAYLOAD_MAX_BYTES {
            return Err(CoordinationError::invalid_params(format!(
                "payload of {} bytes exceeds the {}-byte maximum",
                payload.len(),
                COORDINATION_PAYLOAD_MAX_BYTES
            )));
        }

        self.rate_limiter
            .check(sender_worker_id, std::time::Instant::now())
            .map_err(|err| CoordinationError {
                code: error_code::RATE_LIMITED,
                message: err.to_string(),
            })?;

        // A child (a run's own messages) cannot address a task other than
        // the one its run belongs to.
        let run_task_id: String = self
            .db
            .run_domain_op(Box::new(move |conn| {
                conn.query_row(
                    "SELECT task_id FROM runs WHERE run_id = ?1",
                    [run_id.to_string()],
                    |row| row.get::<_, String>(0),
                )
                .map(|task_id| json!({ "taskId": task_id }))
                .map_err(crate::domain::DomainError::Sqlite)
            }))
            .await
            .map_err(|_| CoordinationError::invalid_params(format!("run {run_id} not found")))?
            .get("taskId")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if run_task_id != task_id.to_string() {
            return Err(CoordinationError::invalid_params(
                "a run cannot address a task other than its own",
            ));
        }

        // replyTo must reference a visible prior message on this run.
        if let Some(reply_to) = reply_to {
            let exists: bool = self
                .db
                .run_domain_op(Box::new(move |conn| {
                    conn.query_row(
                        "SELECT 1 FROM messages WHERE message_id = ?1 AND run_id = ?2",
                        rusqlite::params![reply_to.to_string(), run_id.to_string()],
                        |_| Ok(true),
                    )
                    .map(|found| json!({ "found": found }))
                    .or_else(|_| Ok(json!({ "found": false })))
                }))
                .await
                .map(|v| v["found"].as_bool().unwrap_or(false))
                .unwrap_or(false);
            if !exists {
                return Err(CoordinationError::invalid_params(format!(
                    "replyTo {reply_to} does not reference a visible prior message on this run"
                )));
            }
        }

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
        let mut recorded_sequence = self
            .db
            .run_domain_op(Box::new(move |conn| {
                let mut repo = DomainRepository::new(conn, project_id);
                repo.record_message(&message)
                    .map(|c| embed_envelope(json!({ "sequence": c.sequence }), &c.envelope))
            }))
            .await?;
        self.broadcast(&mut recorded_sequence);

        let mut sent_sequence = self
            .db
            .run_domain_op(Box::new(move |conn| {
                let mut repo = DomainRepository::new(conn, project_id);
                repo.update_delivery(message_id, &DeliveryState::Sent)
                    .map(|c| embed_envelope(json!({ "sequence": c.sequence }), &c.envelope))
            }))
            .await?;
        self.broadcast(&mut sent_sequence);

        Ok(json!({
            "messageId": message_id.to_string(),
            "deliveryState": "sent",
            "recordedSequence": recorded_sequence["sequence"],
            "sentSequence": sent_sequence["sequence"],
        }))
    }

    /// `coordination/task`: the worker-safe view of the task bound to
    /// `run_id`'s scope.
    pub async fn task(&self, run_id: RunId) -> Result<Value, CoordinationError> {
        self.db
            .run_domain_op(Box::new(move |conn| {
                conn.query_row(
                    "SELECT t.task_id, t.owner_client_instance_id, t.revision
                     FROM runs r JOIN tasks t ON r.task_id = t.task_id
                     WHERE r.run_id = ?1",
                    [run_id.to_string()],
                    |row| {
                        Ok(json!({
                            "taskId": row.get::<_, String>(0)?,
                            "ownerClientInstanceId": row.get::<_, String>(1)?,
                            "revision": row.get::<_, i64>(2)?,
                        }))
                    },
                )
                .map_err(|_| crate::domain::DomainError::NotFound {
                    kind: "run",
                    id: run_id.to_string(),
                })
            }))
            .await
            .map_err(Into::into)
    }

    /// `coordination/peers`: sibling workers on the same task as `run_id`.
    pub async fn peers(&self, run_id: RunId) -> Result<Value, CoordinationError> {
        self.db
            .run_domain_op(Box::new(move |conn| {
                let task_id: String = conn
                    .query_row(
                        "SELECT task_id FROM runs WHERE run_id = ?1",
                        [run_id.to_string()],
                        |row| row.get(0),
                    )
                    .map_err(|_| crate::domain::DomainError::NotFound {
                        kind: "run",
                        id: run_id.to_string(),
                    })?;

                let mut stmt = conn.prepare(
                    "SELECT DISTINCT w.worker_id, p.adapter
                     FROM runs r JOIN workers w ON r.worker_id = w.worker_id
                     JOIN worker_profiles p ON w.profile_id = p.id
                     WHERE r.task_id = ?1 AND r.run_id != ?2",
                )?;
                let rows = stmt
                    .query_map(rusqlite::params![task_id, run_id.to_string()], |row| {
                        Ok(json!({
                            "workerId": row.get::<_, String>(0)?,
                            "adapter": row.get::<_, String>(1)?,
                        }))
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(json!({ "peers": rows }))
            }))
            .await
            .map_err(Into::into)
    }

    /// `coordination/requestChild`: records intent only, transitions the
    /// requesting run to `waitingPeer`, and notifies OMP (via the durable
    /// event journal OMP already replays). Never creates a task or worker.
    pub async fn request_child(&self, run_id: RunId, reason: String) -> Result<Value, CoordinationError> {
        let project_id = self.project_id;
        let mut result = self
            .db
            .run_domain_op(Box::new(move |conn| {
                let mut repo = DomainRepository::new(conn, project_id);
                repo.request_child(run_id, &reason)
                    .map(|c| embed_envelope(json!({ "sequence": c.sequence }), &c.envelope))
            }))
            .await
            .map_err(CoordinationError::from)?;
        self.broadcast(&mut result);
        Ok(result)
    }

    /// `coordination/publishArtifact`: journals an artifact reference for
    /// the scoped run without a dedicated projection table -- the durable
    /// event is the record.
    pub async fn publish_artifact(
        &self,
        run_id: RunId,
        artifact_ref: String,
        description: Option<String>,
    ) -> Result<Value, CoordinationError> {
        let sender_and_task = self.run_participants(run_id).await?;
        let project_id = self.project_id;
        let (task_id, worker_id) = sender_and_task;
        let kind = MessageKind::PeerMessage;
        let payload = description.unwrap_or_else(|| artifact_ref.clone());
        let mut result = self
            .db
            .run_domain_op(Box::new(move |conn| {
                let mut repo = DomainRepository::new(conn, project_id);
                let message = RunMessage {
                    message_id: MessageId::new(),
                    run_id,
                    sender_worker_id: worker_id,
                    recipient_worker_id: None,
                    task_id,
                    kind,
                    payload,
                    delivery_state: DeliveryState::Recorded,
                    created_at: Timestamp::now(),
                    sent_at: None,
                    acknowledged_at: None,
                    reply_to: None,
                };
                repo.record_message(&message).map(|c| {
                    embed_envelope(
                        json!({ "sequence": c.sequence, "artifactRef": artifact_ref }),
                        &c.envelope,
                    )
                })
            }))
            .await
            .map_err(CoordinationError::from)?;
        self.broadcast(&mut result);
        Ok(result)
    }

    /// `coordination/reportBlocked`: reports the scoped run is blocked, as
    /// a journaled message OMP can observe, without changing ownership.
    pub async fn report_blocked(&self, run_id: RunId, reason: String) -> Result<Value, CoordinationError> {
        let (task_id, worker_id) = self.run_participants(run_id).await?;
        self.send_internal(run_id, worker_id, task_id, MessageKind::PeerMessage, reason, None, None)
            .await
    }

    /// `coordination/askPolicy`: asks OMP a policy question, as a
    /// journaled message OMP can observe, without deciding it locally.
    pub async fn ask_policy(&self, run_id: RunId, question: String) -> Result<Value, CoordinationError> {
        let (task_id, worker_id) = self.run_participants(run_id).await?;
        self.send_internal(run_id, worker_id, task_id, MessageKind::Question, question, None, None)
            .await
    }

    async fn run_participants(&self, run_id: RunId) -> Result<(TaskId, WorkerId), CoordinationError> {
        let value = self
            .db
            .run_domain_op(Box::new(move |conn| {
                conn.query_row(
                    "SELECT task_id, worker_id FROM runs WHERE run_id = ?1",
                    [run_id.to_string()],
                    |row| {
                        Ok(json!({
                            "taskId": row.get::<_, String>(0)?,
                            "workerId": row.get::<_, String>(1)?,
                        }))
                    },
                )
                .map_err(|_| crate::domain::DomainError::NotFound {
                    kind: "run",
                    id: run_id.to_string(),
                })
            }))
            .await?;
        let task_id = TaskId::parse(value["taskId"].as_str().unwrap_or_default())
            .map_err(|_| CoordinationError::invalid_params("stored run has an invalid taskId"))?;
        let worker_id = WorkerId::parse(value["workerId"].as_str().unwrap_or_default())
            .map_err(|_| CoordinationError::invalid_params("stored run has an invalid workerId"))?;
        Ok((task_id, worker_id))
    }

    async fn send_internal(
        &self,
        run_id: RunId,
        sender_worker_id: WorkerId,
        task_id: TaskId,
        kind: MessageKind,
        payload: String,
        recipient_worker_id: Option<WorkerId>,
        reply_to: Option<MessageId>,
    ) -> Result<Value, CoordinationError> {
        self.send(run_id, sender_worker_id, task_id, kind, payload, recipient_worker_id, reply_to)
            .await
    }

    /// Settles any message left in a non-terminal delivery state
    /// (`recorded` or `sent`, never acknowledged or failed) to `unknown`.
    /// Call once at startup, after the durable journal has been recovered:
    /// a crash between record-intent and adapter acknowledgement leaves
    /// exactly this state, and this sweep never resends -- it only
    /// reclassifies the outcome as unknown.
    pub async fn sweep_unacknowledged_as_unknown(&self) -> Result<u64, CoordinationError> {
        let project_id = self.project_id;
        let mut result = self
            .db
            .run_domain_op(Box::new(move |conn| {
                let ids: Vec<String> = {
                    let mut stmt = conn.prepare(
                        "SELECT message_id FROM messages WHERE delivery_state IN ('recorded', 'sent')",
                    )?;
                    stmt.query_map([], |row| row.get::<_, String>(0))?
                        .collect::<Result<Vec<_>, _>>()?
                };
                let mut repo = DomainRepository::new(conn, project_id);
                let mut count = 0u64;
                let mut envelopes = Vec::new();
                for id in ids {
                    let Ok(message_id) = MessageId::parse(&id) else { continue };
                    let committed = repo.update_delivery(message_id, &DeliveryState::Unknown)?;
                    envelopes.push(committed.envelope);
                    count += 1;
                }
                Ok(json!({ "swept": count, "__envelopes": envelopes }))
            }))
            .await?;
        let swept = result["swept"].as_u64().unwrap_or(0);
        if let Some(envelopes) = result.as_object_mut().and_then(|m| m.remove("__envelopes")) {
            if let Ok(envelopes) = serde_json::from_value::<Vec<EventEnvelope>>(envelopes) {
                for envelope in envelopes {
                    let _ = self.events_tx.send(envelope);
                }
            }
        }
        Ok(swept)
    }
}
