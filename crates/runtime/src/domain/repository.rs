//! The orchestration domain repository.
//!
//! Every mutating command runs one SQLite transaction that appends a durable
//! event to the `events` journal and updates the relevant projection row(s),
//! then commits. If the projection update fails, the transaction rolls back
//! and no event is retained. The append-only journal remains authoritative;
//! projection tables are rebuildable from it.
//!
//! The runtime is the sole authority for run-state transitions: every
//! transition is validated through [`super::transitions::check_transition`]
//! before its event is appended.

use batman_protocol::{
    ApprovalRequest, DeliveryState, EventEnvelope, EventSource, ProjectId, Run, RunFlags,
    RunId, RunMessage, RunState, RuntimeEvent, RuntimeEventKind, TaskId, TaskRef, Timestamp,
    Worker, WorkerId,
};
use rusqlite::Connection;

use super::transitions::{check_transition, TransitionError};

/// Errors returned by [`DomainRepository`] commands.
#[derive(Debug, thiserror::Error)]
pub enum DomainError {
    /// A database operation failed.
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    /// A requested run-state transition was illegal.
    #[error(transparent)]
    Transition(#[from] TransitionError),
    /// A referenced record was not found.
    #[error("{kind} {id} not found")]
    NotFound { kind: &'static str, id: String },
    /// A serialization step failed.
    #[error("failed to serialize event: {0}")]
    Serialize(#[from] serde_json::Error),
    /// The database actor thread is no longer running.
    #[error("database actor is not running")]
    ActorUnavailable,
}

/// The committed result of a mutation: the durable event sequence number the
/// mutation produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Committed {
    pub sequence: u64,
}

/// A repository over the orchestration projection tables and the durable
/// event journal. Holds no state of its own; every command borrows a
/// connection and commits before returning.
pub struct DomainRepository<'c> {
    conn: &'c mut Connection,
    project_id: ProjectId,
}

impl<'c> DomainRepository<'c> {
    /// Creates a repository bound to `conn` for `project_id`.
    #[must_use]
    pub fn new(conn: &'c mut Connection, project_id: ProjectId) -> Self {
        Self { conn, project_id }
    }

    /// Appends an event and runs `apply` against the same transaction,
    /// committing both atomically. Returns the assigned sequence number.
    fn append_and_apply<F>(
        &mut self,
        event: &RuntimeEvent,
        task_id: Option<batman_protocol::TaskId>,
        worker_id: Option<batman_protocol::WorkerId>,
        run_id: Option<batman_protocol::RunId>,
        apply: F,
    ) -> Result<Committed, DomainError>
    where
        F: FnOnce(&rusqlite::Transaction<'_>) -> Result<(), DomainError>,
    {
        let project_id = self.project_id;
        let tx = self.conn.transaction()?;
        let timestamp = Timestamp::now();

        // Build the envelope with a provisional sequence of 0; the real
        // sequence is the rowid assigned on insert. The stored JSON carries
        // the assigned sequence so replay is self-describing.
        let envelope_json = {
            // Insert with a placeholder, then rewrite with the real sequence.
            tx.execute(
                "INSERT INTO events (timestamp, project_id, run_id, event_json) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![
                    timestamp.as_str(),
                    project_id.to_string(),
                    run_id.map(|r| r.to_string()),
                    "{}",
                ],
            )?;
            let sequence = tx.last_insert_rowid() as u64;
            let envelope = EventEnvelope {
                sequence,
                timestamp: timestamp.clone(),
                project_id,
                task_id,
                worker_id,
                run_id,
                parent_worker_id: None,
                source: EventSource::Runtime,
                event: event.clone(),
                vendor_event_ref: None,
            };
            let json = serde_json::to_string(&envelope)?;
            tx.execute(
                "UPDATE events SET event_json = ?1 WHERE sequence = ?2",
                rusqlite::params![json, sequence],
            )?;
            (sequence, json)
        };
        let (sequence, _json) = envelope_json;

        apply(&tx)?;
        tx.commit()?;
        Ok(Committed { sequence })
    }

    /// Upserts an OMP-owned task. Idempotent for an identical revision; a
    /// lower revision is rejected by the caller (service layer) before this
    /// point. Emits a `TaskCreated`/`TaskUpdated` event.
    pub fn upsert_task(
        &mut self,
        task_id: batman_protocol::TaskId,
        task_ref: &TaskRef,
    ) -> Result<Committed, DomainError> {
        let existed: bool = self
            .conn
            .query_row(
                "SELECT 1 FROM tasks WHERE task_id = ?1",
                [task_id.to_string()],
                |_| Ok(true),
            )
            .unwrap_or(false);

        let kind = if existed {
            RuntimeEventKind::TaskUpdated
        } else {
            RuntimeEventKind::TaskCreated
        };
        let event = RuntimeEvent::TaskEvent {
            kind,
            task_id,
            owner_client_instance_id: task_ref.owner_client_instance_id.clone(),
            revision: task_ref.revision,
        };
        let owner = task_ref.owner_client_instance_id.clone();
        let revision = task_ref.revision;
        let project = self.project_id;
        self.append_and_apply(&event, Some(task_id), None, None, move |tx| {
            let now = Timestamp::now();
            tx.execute(
                "INSERT INTO tasks (task_id, project_id, owner_client_instance_id, revision, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?5)
                 ON CONFLICT(task_id) DO UPDATE SET
                   owner_client_instance_id = excluded.owner_client_instance_id,
                   revision = excluded.revision,
                   updated_at = excluded.updated_at",
                rusqlite::params![
                    task_id.to_string(),
                    project.to_string(),
                    owner,
                    revision,
                    now.as_str(),
                ],
            )?;
            Ok(())
        })
    }

    /// Creates a worker, persisting its immutable profile reference. Emits a
    /// `WorkerCreated` event. Fails if the worker id already exists.
    pub fn create_worker(&mut self, worker: &Worker) -> Result<Committed, DomainError> {
        let event = RuntimeEvent::WorkerEvent {
            kind: RuntimeEventKind::WorkerCreated,
            worker_id: worker.worker_id,
            profile_id: worker.profile_ref.id.to_string(),
        };
        let worker = worker.clone();
        let project = self.project_id;
        self.append_and_apply(&event, None, Some(worker.worker_id), None, move |tx| {
            let profile = &worker.profile_ref;
            tx.execute(
                "INSERT INTO worker_profiles (id, fingerprint, adapter, model, permission_envelope)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(id) DO NOTHING",
                rusqlite::params![
                    profile.id.to_string(),
                    profile.fingerprint,
                    profile.adapter,
                    profile.model,
                    serde_json::to_string(&profile.permission_envelope)?,
                ],
            )?;
            tx.execute(
                "INSERT INTO workers (worker_id, project_id, profile_id, parent_worker_id, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![
                    worker.worker_id.to_string(),
                    project.to_string(),
                    profile.id.to_string(),
                    worker.parent_worker_id.map(|w| w.to_string()),
                    worker.created_at.as_str(),
                ],
            )?;
            Ok(())
        })
    }

    /// Submits a run in `queued` state. Requires the task and worker to
    /// exist (enforced by foreign keys). Emits a `RunQueued` event.
    pub fn submit_run(&mut self, run: &Run) -> Result<Committed, DomainError> {
        let event = RuntimeEvent::RunEvent {
            kind: RuntimeEventKind::RunQueued,
            run_id: run.run_id,
            task_id: run.task_id,
            worker_id: run.worker_id,
            state: run.state.to_string(),
        };
        let run = run.clone();
        self.append_and_apply(
            &event,
            Some(run.task_id),
            Some(run.worker_id),
            Some(run.run_id),
            move |tx| {
                let now = Timestamp::now();
                tx.execute(
                    "INSERT INTO runs (run_id, task_id, worker_id, state,
                       flags_degraded_control, flags_needs_reconciliation, flags_protocol_unhealthy,
                       flags_policy_quarantined, flags_workspace_dirty, flags_children_active,
                       vendor_session_id, created_at, started_at, completed_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
                    rusqlite::params![
                        run.run_id.to_string(),
                        run.task_id.to_string(),
                        run.worker_id.to_string(),
                        run.state.to_string(),
                        run.flags.degraded_control as i64,
                        run.flags.needs_reconciliation as i64,
                        run.flags.protocol_unhealthy as i64,
                        run.flags.policy_quarantined as i64,
                        run.flags.workspace_dirty as i64,
                        run.flags.children_active as i64,
                        run.vendor_session_id,
                        now.as_str(),
                        run.started_at.as_ref().map(|t| t.as_str().to_string()),
                        run.completed_at.as_ref().map(|t| t.as_str().to_string()),
                    ],
                )?;
                Ok(())
            },
        )
    }

    /// Transitions a run to a new state, validating the edge first. Emits the
    /// matching `Run*` event and updates the projection. An illegal edge
    /// appends nothing.
    pub fn transition_run(
        &mut self,
        run_id: batman_protocol::RunId,
        to: &RunState,
    ) -> Result<Committed, DomainError> {
        let (from_str, task_id_str, worker_id_str): (String, String, String) = self
            .conn
            .query_row(
                "SELECT state, task_id, worker_id FROM runs WHERE run_id = ?1",
                [run_id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(|_| DomainError::NotFound {
                kind: "run",
                id: run_id.to_string(),
            })?;

        let from = RunState::try_from(from_str.as_str()).map_err(|_| DomainError::NotFound {
            kind: "run-state",
            id: from_str.clone(),
        })?;
        check_transition(&run_id.to_string(), &from, to)?;

        let task_id = batman_protocol::TaskId::parse(&task_id_str).map_err(|_| {
            DomainError::NotFound {
                kind: "task",
                id: task_id_str.clone(),
            }
        })?;
        let worker_id = batman_protocol::WorkerId::parse(&worker_id_str).map_err(|_| {
            DomainError::NotFound {
                kind: "worker",
                id: worker_id_str.clone(),
            }
        })?;

        let kind = kind_for_state(to);
        let event = RuntimeEvent::RunEvent {
            kind,
            run_id,
            task_id,
            worker_id,
            state: to.to_string(),
        };
        let to_owned = to.clone();
        let is_terminal = to.is_terminal();
        let entering_working = to.to_string() == "starting";
        self.append_and_apply(
            &event,
            Some(task_id),
            Some(worker_id),
            Some(run_id),
            move |tx| {
                let now = Timestamp::now();
                tx.execute(
                    "UPDATE runs SET state = ?1 WHERE run_id = ?2",
                    rusqlite::params![to_owned.to_string(), run_id.to_string()],
                )?;
                if entering_working {
                    tx.execute(
                        "UPDATE runs SET started_at = COALESCE(started_at, ?1) WHERE run_id = ?2",
                        rusqlite::params![now.as_str(), run_id.to_string()],
                    )?;
                }
                if is_terminal {
                    tx.execute(
                        "UPDATE runs SET completed_at = ?1 WHERE run_id = ?2",
                        rusqlite::params![now.as_str(), run_id.to_string()],
                    )?;
                }
                Ok(())
            },
        )
    }

    /// Sets the flags on a run. Emits a `RunFlagsChanged` event.
    pub fn set_run_flags(
        &mut self,
        run_id: batman_protocol::RunId,
        flags: &RunFlags,
    ) -> Result<Committed, DomainError> {
        let event = RuntimeEvent::RunFlagsEvent {
            run_id,
            flags: flags.clone(),
        };
        let flags = flags.clone();
        self.append_and_apply(&event, None, None, Some(run_id), move |tx| {
            tx.execute(
                "UPDATE runs SET
                   flags_degraded_control = ?1, flags_needs_reconciliation = ?2,
                   flags_protocol_unhealthy = ?3, flags_policy_quarantined = ?4,
                   flags_workspace_dirty = ?5, flags_children_active = ?6
                 WHERE run_id = ?7",
                rusqlite::params![
                    flags.degraded_control as i64,
                    flags.needs_reconciliation as i64,
                    flags.protocol_unhealthy as i64,
                    flags.policy_quarantined as i64,
                    flags.workspace_dirty as i64,
                    flags.children_active as i64,
                    run_id.to_string(),
                ],
            )?;
            Ok(())
        })
    }

    /// Records a message in `recorded` delivery state (record-before-send).
    /// Emits a `MessageRecorded` event.
    pub fn record_message(&mut self, message: &RunMessage) -> Result<Committed, DomainError> {
        let event = RuntimeEvent::MessageEvent {
            kind: RuntimeEventKind::MessageRecorded,
            message_id: message.message_id,
            run_id: message.run_id,
            task_id: message.task_id,
            delivery_state: delivery_state_str(&message.delivery_state).to_string(),
        };
        let message = message.clone();
        self.append_and_apply(
            &event,
            Some(message.task_id),
            Some(message.sender_worker_id),
            Some(message.run_id),
            move |tx| {
                tx.execute(
                    "INSERT INTO messages (message_id, run_id, sender_worker_id, recipient_worker_id,
                       task_id, kind, payload, delivery_state, created_at, sent_at, acknowledged_at, reply_to)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                    rusqlite::params![
                        message.message_id.to_string(),
                        message.run_id.to_string(),
                        message.sender_worker_id.to_string(),
                        message.recipient_worker_id.map(|w| w.to_string()),
                        message.task_id.to_string(),
                        message_kind_str(&message.kind),
                        message.payload,
                        delivery_state_str(&message.delivery_state),
                        message.created_at.as_str(),
                        message.sent_at.as_ref().map(|t| t.as_str().to_string()),
                        message.acknowledged_at.as_ref().map(|t| t.as_str().to_string()),
                        message.reply_to.map(|m| m.to_string()),
                    ],
                )?;
                Ok(())
            },
        )
    }

    /// Updates a message's delivery state. Emits the matching `Message*`
    /// event.
    pub fn update_delivery(
        &mut self,
        message_id: batman_protocol::MessageId,
        state: &DeliveryState,
    ) -> Result<Committed, DomainError> {
        let (run_id_str, task_id_str): (String, String) = self
            .conn
            .query_row(
                "SELECT run_id, task_id FROM messages WHERE message_id = ?1",
                [message_id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|_| DomainError::NotFound {
                kind: "message",
                id: message_id.to_string(),
            })?;
        let run_id = batman_protocol::RunId::parse(&run_id_str).map_err(|_| {
            DomainError::NotFound {
                kind: "run",
                id: run_id_str.clone(),
            }
        })?;
        let task_id = batman_protocol::TaskId::parse(&task_id_str).map_err(|_| {
            DomainError::NotFound {
                kind: "task",
                id: task_id_str.clone(),
            }
        })?;

        let kind = match state {
            DeliveryState::Sent => RuntimeEventKind::MessageSent,
            DeliveryState::Acknowledged => RuntimeEventKind::MessageAcknowledged,
            DeliveryState::Failed => RuntimeEventKind::MessageFailed,
            DeliveryState::Recorded | DeliveryState::Unknown => RuntimeEventKind::MessageRecorded,
        };
        let event = RuntimeEvent::MessageEvent {
            kind,
            message_id,
            run_id,
            task_id,
            delivery_state: delivery_state_str(state).to_string(),
        };
        let state = state.clone();
        self.append_and_apply(&event, Some(task_id), None, Some(run_id), move |tx| {
            let now = Timestamp::now();
            tx.execute(
                "UPDATE messages SET delivery_state = ?1 WHERE message_id = ?2",
                rusqlite::params![delivery_state_str(&state), message_id.to_string()],
            )?;
            match state {
                DeliveryState::Sent => {
                    tx.execute(
                        "UPDATE messages SET sent_at = COALESCE(sent_at, ?1) WHERE message_id = ?2",
                        rusqlite::params![now.as_str(), message_id.to_string()],
                    )?;
                }
                DeliveryState::Acknowledged => {
                    tx.execute(
                        "UPDATE messages SET acknowledged_at = COALESCE(acknowledged_at, ?1) WHERE message_id = ?2",
                        rusqlite::params![now.as_str(), message_id.to_string()],
                    )?;
                }
                _ => {}
            }
            Ok(())
        })
    }

    /// Creates an approval request and atomically transitions its run
    /// `working -> waitingUser`, in one durable event. Called when an
    /// adapter reports it needs approval for `action`.
    pub fn create_approval(
        &mut self,
        approval: &ApprovalRequest,
    ) -> Result<Committed, DomainError> {
        let (from_str,): (String,) = self
            .conn
            .query_row(
                "SELECT state FROM runs WHERE run_id = ?1",
                [approval.run_id.to_string()],
                |row| Ok((row.get(0)?,)),
            )
            .map_err(|_| DomainError::NotFound {
                kind: "run",
                id: approval.run_id.to_string(),
            })?;
        let from = RunState::try_from(from_str.as_str()).map_err(|_| DomainError::NotFound {
            kind: "run-state",
            id: from_str.clone(),
        })?;
        let waiting_user = RunState::try_from("waitingUser").expect("waitingUser is valid");
        check_transition(&approval.run_id.to_string(), &from, &waiting_user)?;

        let event = RuntimeEvent::ApprovalEvent {
            kind: RuntimeEventKind::ApprovalRequested,
            approval_id: approval.approval_id,
            run_id: approval.run_id,
            task_id: approval.task_id,
            action: approval.action.clone(),
        };
        let approval = approval.clone();
        self.append_and_apply(
            &event,
            Some(approval.task_id),
            None,
            Some(approval.run_id),
            move |tx| {
                tx.execute(
                    "INSERT INTO approvals (approval_id, run_id, task_id, action, arguments,
                       human_required, policy_reason, created_at, decided_at, decision)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    rusqlite::params![
                        approval.approval_id.to_string(),
                        approval.run_id.to_string(),
                        approval.task_id.to_string(),
                        approval.action,
                        serde_json::to_string(&approval.arguments)?,
                        approval.human_required as i64,
                        approval.policy_reason,
                        approval.created_at.as_str(),
                        approval.decided_at.as_ref().map(|t| t.as_str().to_string()),
                        approval.decision,
                    ],
                )?;
                tx.execute(
                    "UPDATE runs SET state = 'waitingUser' WHERE run_id = ?1",
                    rusqlite::params![approval.run_id.to_string()],
                )?;
                Ok(())
            },
        )
    }

    /// Records an approval decision. Emits an `ApprovalDecided` event.
    pub fn decide_approval(
        &mut self,
        approval_id: batman_protocol::ApprovalId,
        decision: &str,
        reason: &str,
    ) -> Result<Committed, DomainError> {
        let (run_id_str, task_id_str, action): (String, String, String) = self
            .conn
            .query_row(
                "SELECT run_id, task_id, action FROM approvals WHERE approval_id = ?1",
                [approval_id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(|_| DomainError::NotFound {
                kind: "approval",
                id: approval_id.to_string(),
            })?;
        let run_id = batman_protocol::RunId::parse(&run_id_str).map_err(|_| {
            DomainError::NotFound {
                kind: "run",
                id: run_id_str.clone(),
            }
        })?;
        let task_id = batman_protocol::TaskId::parse(&task_id_str).map_err(|_| {
            DomainError::NotFound {
                kind: "task",
                id: task_id_str.clone(),
            }
        })?;

        let event = RuntimeEvent::ApprovalEvent {
            kind: RuntimeEventKind::ApprovalDecided,
            approval_id,
            run_id,
            task_id,
            action,
        };
        let decision = decision.to_string();
        let reason = reason.to_string();
        self.append_and_apply(&event, Some(task_id), None, Some(run_id), move |tx| {
            let now = Timestamp::now();
            let _ = reason;
            tx.execute(
                "UPDATE approvals SET decision = ?1, decided_at = ?2 WHERE approval_id = ?3",
                rusqlite::params![decision, now.as_str(), approval_id.to_string()],
            )?;
            Ok(())
        })
    }

    /// Rebinds a task's owning OMP client instance during reconciliation.
    /// Emits a `ReconcileOwnershipChanged` event carrying old/new owner ids.
    pub fn reconcile_ownership(
        &mut self,
        task_id: batman_protocol::TaskId,
        new_owner: &str,
        revision: u64,
    ) -> Result<Committed, DomainError> {
        let old_owner: String = self
            .conn
            .query_row(
                "SELECT owner_client_instance_id FROM tasks WHERE task_id = ?1",
                [task_id.to_string()],
                |row| row.get(0),
            )
            .map_err(|_| DomainError::NotFound {
                kind: "task",
                id: task_id.to_string(),
            })?;

        let event = RuntimeEvent::ReconcileEvent {
            task_id,
            old_owner_client_instance_id: old_owner,
            new_owner_client_instance_id: new_owner.to_string(),
            revision,
        };
        let new_owner = new_owner.to_string();
        self.append_and_apply(&event, Some(task_id), None, None, move |tx| {
            let now = Timestamp::now();
            tx.execute(
                "UPDATE tasks SET owner_client_instance_id = ?1, revision = ?2, updated_at = ?3 WHERE task_id = ?4",
                rusqlite::params![new_owner, revision, now.as_str(), task_id.to_string()],
            )?;
            Ok(())
        })
    }

    /// Records a child-worker request: appends `ChildWorkerRequested` and
    /// transitions the requesting run `working -> waitingPeer`. Never
    /// creates a task or worker itself -- OMP answers through
    /// [`DomainRepository::decide_child`].
    pub fn request_child(
        &mut self,
        parent_run_id: RunId,
        reason: &str,
    ) -> Result<Committed, DomainError> {
        let (from_str, task_id_str, worker_id_str): (String, String, String) = self
            .conn
            .query_row(
                "SELECT state, task_id, worker_id FROM runs WHERE run_id = ?1",
                [parent_run_id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(|_| DomainError::NotFound {
                kind: "run",
                id: parent_run_id.to_string(),
            })?;
        let from = RunState::try_from(from_str.as_str()).map_err(|_| DomainError::NotFound {
            kind: "run-state",
            id: from_str.clone(),
        })?;
        let waiting_peer = RunState::try_from("waitingPeer").expect("waitingPeer is valid");
        check_transition(&parent_run_id.to_string(), &from, &waiting_peer)?;

        let task_id = TaskId::parse(&task_id_str)
            .map_err(|_| DomainError::NotFound { kind: "task", id: task_id_str.clone() })?;
        let worker_id = WorkerId::parse(&worker_id_str)
            .map_err(|_| DomainError::NotFound { kind: "worker", id: worker_id_str.clone() })?;

        let event = RuntimeEvent::ChildEvent {
            kind: RuntimeEventKind::ChildWorkerRequested,
            parent_run_id,
            child_task_id: None,
            child_worker_id: None,
            child_run_id: None,
            reason: Some(reason.to_string()),
        };
        self.append_and_apply(
            &event,
            Some(task_id),
            Some(worker_id),
            Some(parent_run_id),
            move |tx| {
                tx.execute(
                    "UPDATE runs SET state = 'waitingPeer' WHERE run_id = ?1",
                    rusqlite::params![parent_run_id.to_string()],
                )?;
                Ok(())
            },
        )
    }

    /// Records OMP's decision on a prior child-worker request and returns
    /// the parent run to `working`. Acceptance carries the OMP-created
    /// child ids; denial carries a reason. The runtime owns both
    /// transitions after the correlated decision commits.
    pub fn decide_child(
        &mut self,
        parent_run_id: RunId,
        accepted: bool,
        child_task_id: Option<TaskId>,
        child_worker_id: Option<WorkerId>,
        child_run_id: Option<RunId>,
        reason: Option<&str>,
    ) -> Result<Committed, DomainError> {
        let (from_str, task_id_str, worker_id_str): (String, String, String) = self
            .conn
            .query_row(
                "SELECT state, task_id, worker_id FROM runs WHERE run_id = ?1",
                [parent_run_id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(|_| DomainError::NotFound {
                kind: "run",
                id: parent_run_id.to_string(),
            })?;
        let from = RunState::try_from(from_str.as_str()).map_err(|_| DomainError::NotFound {
            kind: "run-state",
            id: from_str.clone(),
        })?;
        let working = RunState::try_from("working").expect("working is valid");
        check_transition(&parent_run_id.to_string(), &from, &working)?;

        let task_id = TaskId::parse(&task_id_str)
            .map_err(|_| DomainError::NotFound { kind: "task", id: task_id_str.clone() })?;
        let worker_id = WorkerId::parse(&worker_id_str)
            .map_err(|_| DomainError::NotFound { kind: "worker", id: worker_id_str.clone() })?;

        let kind = if accepted {
            RuntimeEventKind::ChildWorkerRequested
        } else {
            RuntimeEventKind::ChildWorkerRequestDenied
        };
        let event = RuntimeEvent::ChildEvent {
            kind,
            parent_run_id,
            child_task_id,
            child_worker_id,
            child_run_id,
            reason: reason.map(str::to_string),
        };
        self.append_and_apply(
            &event,
            Some(task_id),
            Some(worker_id),
            Some(parent_run_id),
            move |tx| {
                tx.execute(
                    "UPDATE runs SET state = 'working' WHERE run_id = ?1",
                    rusqlite::params![parent_run_id.to_string()],
                )?;
                Ok(())
            },
        )
    }
}

/// Maps a run state to the event kind that records entering it.
fn kind_for_state(state: &RunState) -> RuntimeEventKind {
    match state.to_string().as_str() {
        "queued" => RuntimeEventKind::RunQueued,
        "starting" => RuntimeEventKind::RunStarting,
        "working" => RuntimeEventKind::RunWorking,
        "waitingUser" => RuntimeEventKind::RunWaitingUser,
        "waitingPeer" => RuntimeEventKind::RunWaitingPeer,
        "paused" => RuntimeEventKind::RunPaused,
        "succeeded" => RuntimeEventKind::RunSucceeded,
        "failed" => RuntimeEventKind::RunFailed,
        "cancelled" => RuntimeEventKind::RunCancelled,
        "lost" => RuntimeEventKind::RunLost,
        _ => RuntimeEventKind::RunWorking,
    }
}

/// The canonical wire string for a delivery state.
fn delivery_state_str(state: &DeliveryState) -> &'static str {
    match state {
        DeliveryState::Recorded => "recorded",
        DeliveryState::Sent => "sent",
        DeliveryState::Acknowledged => "acknowledged",
        DeliveryState::Failed => "failed",
        DeliveryState::Unknown => "unknown",
    }
}

/// The canonical wire string for a message kind.
fn message_kind_str(kind: &batman_protocol::MessageKind) -> &'static str {
    use batman_protocol::MessageKind;
    match kind {
        MessageKind::Assign => "assign",
        MessageKind::Steer => "steer",
        MessageKind::FollowUp => "followUp",
        MessageKind::Question => "question",
        MessageKind::Answer => "answer",
        MessageKind::PeerMessage => "peerMessage",
        MessageKind::ApprovalDecision => "approvalDecision",
        MessageKind::Cancel => "cancel",
        MessageKind::Shutdown => "shutdown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use batman_protocol::{ProjectId, RunId, TaskId, WorkerId, WorkerProfileRef};
    use rusqlite::Connection;

    fn open_test_db() -> Connection {
        let conn = Connection::open_in_memory().expect("in-memory db");
        conn.execute_batch(
            "CREATE TABLE events (
                sequence INTEGER PRIMARY KEY,
                timestamp TEXT NOT NULL,
                project_id TEXT NOT NULL,
                run_id TEXT,
                event_json TEXT NOT NULL
            );
            CREATE TABLE worker_profiles (
                id TEXT PRIMARY KEY, fingerprint TEXT NOT NULL, adapter TEXT NOT NULL,
                model TEXT NOT NULL, permission_envelope TEXT NOT NULL
            );
            CREATE TABLE tasks (
                task_id TEXT PRIMARY KEY, project_id TEXT NOT NULL,
                owner_client_instance_id TEXT NOT NULL, revision INTEGER NOT NULL,
                created_at TEXT NOT NULL, updated_at TEXT NOT NULL
            );
            CREATE TABLE workers (
                worker_id TEXT PRIMARY KEY, project_id TEXT NOT NULL,
                profile_id TEXT NOT NULL REFERENCES worker_profiles(id),
                parent_worker_id TEXT REFERENCES workers(worker_id), created_at TEXT NOT NULL
            );
            CREATE TABLE runs (
                run_id TEXT PRIMARY KEY, task_id TEXT NOT NULL REFERENCES tasks(task_id),
                worker_id TEXT NOT NULL REFERENCES workers(worker_id), state TEXT NOT NULL,
                flags_degraded_control INTEGER NOT NULL DEFAULT 0,
                flags_needs_reconciliation INTEGER NOT NULL DEFAULT 0,
                flags_protocol_unhealthy INTEGER NOT NULL DEFAULT 0,
                flags_policy_quarantined INTEGER NOT NULL DEFAULT 0,
                flags_workspace_dirty INTEGER NOT NULL DEFAULT 0,
                flags_children_active INTEGER NOT NULL DEFAULT 0,
                vendor_session_id TEXT, created_at TEXT NOT NULL, started_at TEXT, completed_at TEXT
            );
            CREATE TABLE messages (
                message_id TEXT PRIMARY KEY, run_id TEXT NOT NULL REFERENCES runs(run_id),
                sender_worker_id TEXT NOT NULL, recipient_worker_id TEXT, task_id TEXT NOT NULL,
                kind TEXT NOT NULL, payload TEXT NOT NULL, delivery_state TEXT NOT NULL,
                created_at TEXT NOT NULL, sent_at TEXT, acknowledged_at TEXT, reply_to TEXT
            );
            CREATE TABLE approvals (
                approval_id TEXT PRIMARY KEY, run_id TEXT NOT NULL REFERENCES runs(run_id),
                task_id TEXT NOT NULL, action TEXT NOT NULL, arguments TEXT NOT NULL,
                human_required INTEGER NOT NULL DEFAULT 0, policy_reason TEXT NOT NULL,
                created_at TEXT NOT NULL, decided_at TEXT, decision TEXT
            );",
        )
        .expect("schema");
        conn
    }

    fn seed_worker(conn: &mut Connection, project_id: ProjectId) -> (TaskId, WorkerId) {
        let mut repo = DomainRepository::new(conn, project_id);
        let task_id = TaskId::new();
        repo.upsert_task(
            task_id,
            &TaskRef {
                owner_client_instance_id: "omp-1".into(),
                revision: 1,
            },
        )
        .expect("upsert task");
        let worker_id = WorkerId::new();
        let worker = Worker {
            worker_id,
            profile_ref: WorkerProfileRef {
                id: worker_id,
                fingerprint: "sha256:fake".into(),
                adapter: "fake".into(),
                model: "test".into(),
                permission_envelope: serde_json::json!({}),
            },
            parent_worker_id: None,
            created_at: Timestamp::now(),
        };
        repo.create_worker(&worker).expect("create worker");
        (task_id, worker_id)
    }

    /// Exercises the actual `DomainRepository` API (not raw SQL): submits a
    /// run through `submit_run`, then transitions it through the repository,
    /// proving each command commits one event + one projection update in a
    /// single transaction.
    #[test]
    fn submit_run_and_transition_commit_event_and_projection_together() {
        let mut conn = open_test_db();
        let project_id = ProjectId::new();
        let (task_id, worker_id) = seed_worker(&mut conn, project_id);

        let run_id = RunId::new();
        let run = Run {
            run_id,
            task_id,
            worker_id,
            state: RunState::try_from("queued").unwrap(),
            flags: RunFlags::default(),
            vendor_session_id: None,
            started_at: None,
            completed_at: None,
        };

        let mut repo = DomainRepository::new(&mut conn, project_id);
        let committed = repo.submit_run(&run).expect("submit_run commits");
        assert_eq!(committed.sequence, 3, "task upsert (1), worker create (2), run submit (3)");

        let working = RunState::try_from("starting").unwrap();
        let committed2 = repo
            .transition_run(run_id, &working)
            .expect("transition_run commits");
        assert_eq!(committed2.sequence, 4);

        // The projection reflects the transition.
        let state: String = conn
            .query_row(
                "SELECT state FROM runs WHERE run_id = ?1",
                [run_id.to_string()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(state, "starting");

        // The event journal has exactly 4 durable rows (task, worker, run, transition).
        let event_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(event_count, 4);
    }

    /// An illegal transition through the real repository API commits
    /// nothing: no new event, no projection change.
    #[test]
    fn transition_run_rejects_illegal_edge_and_appends_nothing() {
        let mut conn = open_test_db();
        let project_id = ProjectId::new();
        let (task_id, worker_id) = seed_worker(&mut conn, project_id);
        let run_id = RunId::new();
        let run = Run {
            run_id,
            task_id,
            worker_id,
            state: RunState::try_from("queued").unwrap(),
            flags: RunFlags::default(),
            vendor_session_id: None,
            started_at: None,
            completed_at: None,
        };
        let mut repo = DomainRepository::new(&mut conn, project_id);
        repo.submit_run(&run).unwrap();

        let before: i64 = conn
            .query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))
            .unwrap();

        // queued -> succeeded is not a legal edge.
        let mut repo = DomainRepository::new(&mut conn, project_id);
        let target = RunState::try_from("succeeded").unwrap();
        let err = repo.transition_run(run_id, &target).unwrap_err();
        assert!(matches!(err, DomainError::Transition(_)));

        let after: i64 = conn
            .query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(before, after, "illegal transition must append no event");

        let state: String = conn
            .query_row(
                "SELECT state FROM runs WHERE run_id = ?1",
                [run_id.to_string()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(state, "queued", "projection must be unchanged");
    }
}
