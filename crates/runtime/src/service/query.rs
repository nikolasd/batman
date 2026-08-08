//! Read-only projection queries, built as [`DomainClosure`]s so they run on
//! the database actor thread alongside every mutating command.

use batman_protocol::{ApprovalId, MessageId, ProjectId, RunId, TaskId, WorkerId};
use rusqlite::OptionalExtension;
use serde_json::{Value, json};

use crate::db::DomainClosure;
use crate::domain::DomainError;

pub fn task_get_op(task_id: TaskId) -> DomainClosure {
    Box::new(move |conn| {
        conn.query_row(
            "SELECT task_id, project_id, owner_client_instance_id, revision, created_at, updated_at
             FROM tasks WHERE task_id = ?1",
            [task_id.to_string()],
            |row| {
                Ok(json!({
                    "taskId": row.get::<_, String>(0)?,
                    "projectId": row.get::<_, String>(1)?,
                    "ownerClientInstanceId": row.get::<_, String>(2)?,
                    "revision": row.get::<_, i64>(3)?,
                    "createdAt": row.get::<_, String>(4)?,
                    "updatedAt": row.get::<_, String>(5)?,
                }))
            },
        )
        .optional()
        .map_err(DomainError::Sqlite)?
        .ok_or(DomainError::NotFound {
            kind: "task",
            id: task_id.to_string(),
        })
    })
}

/// Reads a run's `policyQuarantined` flag, for the shared quarantine gate
/// `message/send`/`workspace/apply`/`coordination/publishArtifact` each
/// apply before mutating.
pub fn run_flags_op(run_id: RunId) -> DomainClosure {
    Box::new(move |conn| {
        conn.query_row(
            "SELECT flags_policy_quarantined FROM runs WHERE run_id = ?1",
            [run_id.to_string()],
            |row| Ok(json!({ "policyQuarantined": row.get::<_, i64>(0)? != 0 })),
        )
        .optional()
        .map_err(DomainError::Sqlite)?
        .ok_or(DomainError::NotFound {
            kind: "run",
            id: run_id.to_string(),
        })
    })
}

pub fn worker_get_op(worker_id: WorkerId) -> DomainClosure {
    Box::new(move |conn| {
        conn.query_row(
            "SELECT w.worker_id, w.project_id, w.parent_worker_id, w.created_at,
                    p.id, p.fingerprint, p.adapter, p.model, p.permission_envelope
             FROM workers w JOIN worker_profiles p ON w.profile_id = p.id
             WHERE w.worker_id = ?1",
            [worker_id.to_string()],
            |row| {
                Ok(json!({
                    "workerId": row.get::<_, String>(0)?,
                    "projectId": row.get::<_, String>(1)?,
                    "parentWorkerId": row.get::<_, Option<String>>(2)?,
                    "createdAt": row.get::<_, String>(3)?,
                    "profileRef": {
                        "id": row.get::<_, String>(4)?,
                        "fingerprint": row.get::<_, String>(5)?,
                        "adapter": row.get::<_, String>(6)?,
                        "model": row.get::<_, String>(7)?,
                        "permissionEnvelope": serde_json::from_str::<Value>(&row.get::<_, String>(8)?).unwrap_or(Value::Null),
                    }
                }))
            },
        )
        .optional()
        .map_err(DomainError::Sqlite)?
        .ok_or(DomainError::NotFound {
            kind: "worker",
            id: worker_id.to_string(),
        })
    })
}

pub fn worker_list_op(project_id: ProjectId) -> DomainClosure {
    Box::new(move |conn| {
        let mut stmt = conn.prepare(
            "SELECT w.worker_id, w.parent_worker_id, w.created_at,
                    p.id, p.fingerprint, p.adapter, p.model
             FROM workers w JOIN worker_profiles p ON w.profile_id = p.id
             WHERE w.project_id = ?1 ORDER BY w.created_at",
        )?;
        let rows = stmt
            .query_map([project_id.to_string()], |row| {
                Ok(json!({
                    "workerId": row.get::<_, String>(0)?,
                    "parentWorkerId": row.get::<_, Option<String>>(1)?,
                    "createdAt": row.get::<_, String>(2)?,
                    "profileRef": {
                        "id": row.get::<_, String>(3)?,
                        "fingerprint": row.get::<_, String>(4)?,
                        "adapter": row.get::<_, String>(5)?,
                        "model": row.get::<_, String>(6)?,
                    }
                }))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(json!({ "workers": rows }))
    })
}

pub fn run_get_op(run_id: RunId) -> DomainClosure {
    Box::new(move |conn| {
        conn.query_row(
            "SELECT run_id, task_id, worker_id, state,
                    flags_degraded_control, flags_needs_reconciliation, flags_protocol_unhealthy,
                    flags_policy_quarantined, flags_workspace_dirty, flags_children_active,
                    vendor_session_id, created_at, started_at, completed_at, policy_fingerprint
             FROM runs WHERE run_id = ?1",
            [run_id.to_string()],
            row_to_run_json,
        )
        .optional()
        .map_err(DomainError::Sqlite)?
        .ok_or(DomainError::NotFound {
            kind: "run",
            id: run_id.to_string(),
        })
    })
}

pub fn run_list_op(task_id: Option<TaskId>, project_id: ProjectId) -> DomainClosure {
    Box::new(move |conn| {
        let rows = if let Some(task_id) = task_id {
            let mut stmt = conn.prepare(
                "SELECT run_id, task_id, worker_id, state,
                        flags_degraded_control, flags_needs_reconciliation, flags_protocol_unhealthy,
                        flags_policy_quarantined, flags_workspace_dirty, flags_children_active,
                        vendor_session_id, created_at, started_at, completed_at, policy_fingerprint
                 FROM runs WHERE task_id = ?1 ORDER BY created_at",
            )?;
            stmt.query_map([task_id.to_string()], row_to_run_json)?
                .collect::<Result<Vec<_>, _>>()?
        } else {
            let mut stmt = conn.prepare(
                "SELECT r.run_id, r.task_id, r.worker_id, r.state,
                        r.flags_degraded_control, r.flags_needs_reconciliation, r.flags_protocol_unhealthy,
                        r.flags_policy_quarantined, r.flags_workspace_dirty, r.flags_children_active,
                        r.vendor_session_id, r.created_at, r.started_at, r.completed_at,
                        r.policy_fingerprint
                 FROM runs r JOIN tasks t ON r.task_id = t.task_id
                 WHERE t.project_id = ?1 ORDER BY r.created_at",
            )?;
            stmt.query_map([project_id.to_string()], row_to_run_json)?
                .collect::<Result<Vec<_>, _>>()?
        };
        Ok(json!({ "runs": rows }))
    })
}

fn row_to_run_json(row: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    Ok(json!({
        "runId": row.get::<_, String>(0)?,
        "taskId": row.get::<_, String>(1)?,
        "workerId": row.get::<_, String>(2)?,
        "state": row.get::<_, String>(3)?,
        "flags": {
            "degradedControl": row.get::<_, i64>(4)? != 0,
            "needsReconciliation": row.get::<_, i64>(5)? != 0,
            "protocolUnhealthy": row.get::<_, i64>(6)? != 0,
            "policyQuarantined": row.get::<_, i64>(7)? != 0,
            "workspaceDirty": row.get::<_, i64>(8)? != 0,
            "childrenActive": row.get::<_, i64>(9)? != 0,
        },
        "vendorSessionId": row.get::<_, Option<String>>(10)?,
        "createdAt": row.get::<_, String>(11)?,
        "startedAt": row.get::<_, Option<String>>(12)?,
        "completedAt": row.get::<_, Option<String>>(13)?,
        // The immutable snapshot of the merged policy this run was
        // authorized under, so a later violation is auditable against the
        // exact merge that permitted it. `None` for runs created without a
        // merged startup config (tests and embeddings).
        "policyFingerprint": row.get::<_, Option<String>>(14)?,
    }))
}

pub fn message_list_op(run_id: RunId) -> DomainClosure {
    Box::new(move |conn| {
        let mut stmt = conn.prepare(
            "SELECT message_id, run_id, sender_worker_id, recipient_worker_id, task_id, kind,
                    payload, delivery_state, created_at, sent_at, acknowledged_at, reply_to
             FROM messages WHERE run_id = ?1 ORDER BY created_at",
        )?;
        let rows = stmt
            .query_map([run_id.to_string()], |row| {
                Ok(json!({
                    "messageId": row.get::<_, String>(0)?,
                    "runId": row.get::<_, String>(1)?,
                    "senderWorkerId": row.get::<_, String>(2)?,
                    "recipientWorkerId": row.get::<_, Option<String>>(3)?,
                    "taskId": row.get::<_, String>(4)?,
                    "kind": row.get::<_, String>(5)?,
                    "payload": row.get::<_, String>(6)?,
                    "deliveryState": row.get::<_, String>(7)?,
                    "createdAt": row.get::<_, String>(8)?,
                    "sentAt": row.get::<_, Option<String>>(9)?,
                    "acknowledgedAt": row.get::<_, Option<String>>(10)?,
                    "replyTo": row.get::<_, Option<String>>(11)?,
                }))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(json!({ "messages": rows }))
    })
}

pub fn approval_list_op(run_id: Option<RunId>) -> DomainClosure {
    Box::new(move |conn| {
        let rows = if let Some(run_id) = run_id {
            let mut stmt = conn.prepare(
                "SELECT approval_id, run_id, task_id, action, arguments, human_required,
                        policy_reason, created_at, decided_at, decision
                 FROM approvals WHERE run_id = ?1 ORDER BY created_at",
            )?;
            stmt.query_map([run_id.to_string()], row_to_approval_json)?
                .collect::<Result<Vec<_>, _>>()?
        } else {
            let mut stmt = conn.prepare(
                "SELECT approval_id, run_id, task_id, action, arguments, human_required,
                        policy_reason, created_at, decided_at, decision
                 FROM approvals WHERE decision IS NULL ORDER BY created_at",
            )?;
            stmt.query_map([], row_to_approval_json)?
                .collect::<Result<Vec<_>, _>>()?
        };
        Ok(json!({ "approvals": rows }))
    })
}

fn row_to_approval_json(row: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    Ok(json!({
        "approvalId": row.get::<_, String>(0)?,
        "runId": row.get::<_, String>(1)?,
        "taskId": row.get::<_, String>(2)?,
        "action": row.get::<_, String>(3)?,
        "arguments": serde_json::from_str::<Value>(&row.get::<_, String>(4)?).unwrap_or(Value::Null),
        "humanRequired": row.get::<_, i64>(5)? != 0,
        "policyReason": row.get::<_, String>(6)?,
        "createdAt": row.get::<_, String>(7)?,
        "decidedAt": row.get::<_, Option<String>>(8)?,
        "decision": row.get::<_, Option<String>>(9)?,
    }))
}

/// Suppresses unused-import warnings for ids referenced only in signatures
/// across the various `Option<T>` positions.
#[allow(unused_imports)]
use ApprovalId as _ApprovalId;
#[allow(unused_imports)]
use MessageId as _MessageId;
pub fn owned_run_ids_op(
    owner_instance_id: String,
    task_id: Option<TaskId>,
    project_id: ProjectId,
) -> DomainClosure {
    Box::new(move |conn| {
        let sql = if task_id.is_some() {
            "SELECT r.run_id FROM runs r JOIN tasks t ON r.task_id = t.task_id WHERE t.project_id = ?1 AND t.owner_client_instance_id = ?2 AND t.task_id = ?3"
        } else {
            "SELECT r.run_id FROM runs r JOIN tasks t ON r.task_id = t.task_id WHERE t.project_id = ?1 AND t.owner_client_instance_id = ?2"
        };
        let mut stmt = conn.prepare(sql)?;
        let ids: Vec<String> = if let Some(task_id) = task_id {
            stmt.query_map(
                rusqlite::params![
                    project_id.to_string(),
                    owner_instance_id,
                    task_id.to_string()
                ],
                |row| row.get(0),
            )?
            .filter_map(|r| r.ok())
            .collect()
        } else {
            stmt.query_map(
                rusqlite::params![project_id.to_string(), owner_instance_id],
                |row| row.get(0),
            )?
            .filter_map(|r| r.ok())
            .collect()
        };
        Ok(json!(ids))
    })
}
