//! Approval request and decision types.
//!
//! When an adapter reports an approval, the runtime atomically creates
//! the request, transitions the working run to `waitingUser`, and emits
//! one correlated event. On decision, the runtime records the decision
//! before invoking the adapter callback.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::Timestamp;
use crate::ids::{ApprovalId, RunId, TaskId};

/// An approval request raised by the runtime when an adapter needs
/// human or policy input.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct ApprovalRequest {
    /// The approval request identifier (UUIDv7).
    #[serde(rename = "approvalId")]
    pub approval_id: ApprovalId,
    /// The run that triggered the approval.
    #[serde(rename = "runId")]
    pub run_id: RunId,
    /// The task this approval relates to.
    #[serde(rename = "taskId")]
    pub task_id: TaskId,
    /// The action the adapter is requesting approval for.
    pub action: String,
    /// Arguments after redaction (never raw secrets).
    #[ts(type = "object")]
    pub arguments: serde_json::Value,
    /// Whether human approval is required.
    #[serde(rename = "humanRequired")]
    pub human_required: bool,
    /// The policy reason for this approval.
    pub policy_reason: String,
    /// When the request was created (UTC RFC 3339).
    #[serde(rename = "createdAt")]
    pub created_at: Timestamp,
    /// When the request was decided (UTC RFC 3339), if applicable.
    #[serde(rename = "decidedAt", skip_serializing_if = "Option::is_none")]
    pub decided_at: Option<Timestamp>,
    /// The decision made, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision: Option<String>,
}

/// A decision on an approval request: approve or deny.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct ApprovalDecision {
    /// Either `"approve"` or `"deny"`.
    pub decision: String,
    /// The reason for this decision.
    pub reason: String,
}
