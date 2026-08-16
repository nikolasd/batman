//! Coordination broker wire types: the worker-safe messaging surface a
//! supervised vendor process uses to talk to its task, its peers, and OMP.
//!
//! Every operation here is scoped to the connection's bound `WorkerMcp`
//! principal (its run, task, and project) -- none of them may create,
//! reassign, or merge a task; that authority stays with OMP.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::ids::{MessageId, RunId, TaskId, WorkerId};
use crate::message::MessageKind;

/// Parameters for `coordination/task`: fetches the worker-safe view of the
/// task bound to this connection's scope token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct CoordinationTaskParams {
    pub run_id: RunId,
}

/// Parameters for `coordination/peers`: lists sibling workers on the same
/// task the connection's run is scoped to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct CoordinationPeersParams {
    pub run_id: RunId,
}

/// Parameters for `coordination/send`: sends a correlated, journaled
/// message from the scoped run to a peer or to OMP.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct CoordinationSendParams {
    pub run_id: RunId,
    pub sender_worker_id: WorkerId,
    pub task_id: TaskId,
    pub kind: MessageKind,
    pub payload: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recipient_worker_id: Option<WorkerId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<MessageId>,
}

/// Parameters for `coordination/requestChild`: asks OMP to authorize a
/// child worker. Only records intent -- it never creates a task or worker
/// itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct CoordinationRequestChildParams {
    pub run_id: RunId,
    pub reason: String,
}

/// Parameters for `coordination/publishArtifact`: records a reference to an
/// artifact produced by the scoped run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct CoordinationPublishArtifactParams {
    pub run_id: RunId,
    pub artifact_ref: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Parameters for `coordination/reportBlocked`: reports the scoped run is
/// blocked (e.g. on a peer answer) without changing task ownership.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct CoordinationReportBlockedParams {
    pub run_id: RunId,
    pub reason: String,
}

/// Parameters for `coordination/askPolicy`: asks OMP a policy question
/// (e.g. "may I write to this path?") without deciding it locally.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct CoordinationAskPolicyParams {
    pub run_id: RunId,
    pub question: String,
}

/// Parameters for `coordination/child/decide`: OMP's answer to a prior
/// `requestChild`. Acceptance supplies the OMP-created child ids;
/// denial supplies a reason. Exactly one of `accept`/`deny` applies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(
    tag = "decision",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
#[ts(export)]
pub enum CoordinationChildDecision {
    Accept {
        parent_run_id: RunId,
        child_task_id: TaskId,
        child_worker_id: WorkerId,
        child_run_id: RunId,
    },
    Deny {
        parent_run_id: RunId,
        reason: String,
    },
}

/// The upper bound on any single worker-supplied string a `coordination/*`
/// call can journal (`send`'s `payload`, `requestChild`'s `reason`,
/// `publishArtifact`'s `artifactRef`/`description`), in bytes. A larger
/// value is rejected with `INVALID_PARAMS` before any journaling.
pub const COORDINATION_PAYLOAD_MAX_BYTES: usize = 64 * 1024;

/// The maximum journaling coordination calls one sender may make within a
/// one-minute window before `coordination/send`,
/// `coordination/requestChild`, or `coordination/publishArtifact` returns
/// `RATE_LIMITED`. One budget, shared across the three methods.
pub const COORDINATION_RATE_LIMIT_PER_MINUTE: u32 = 30;
