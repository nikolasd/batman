//! Extended JSON-RPC methods for the orchestration extension.
//!
//! Foundation scope implements `initialize`, `runtime/status`,
//! `events/subscribe`, `events/replay`, and `runtime/shutdown`.
//! The orchestration extension adds task, worker, run, message, approval,
//! coordination, and reconciliation methods.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// All JSON-RPC methods implemented by the BATMAN runtime, including
/// orchestration extension methods.
///
/// Serialized as the literal method name string used on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[ts(export)]
pub enum BatmanMethod {
    // Foundation methods
    #[serde(rename = "initialize")]
    Initialize,
    #[serde(rename = "runtime/status")]
    RuntimeStatus,
    #[serde(rename = "events/subscribe")]
    EventsSubscribe,
    #[serde(rename = "events/replay")]
    EventsReplay,
    #[serde(rename = "runtime/shutdown")]
    RuntimeShutdown,

    // Orchestration: task
    #[serde(rename = "task/upsert")]
    TaskUpsert,
    #[serde(rename = "task/get")]
    TaskGet,

    // Orchestration: worker
    #[serde(rename = "worker/create")]
    WorkerCreate,
    #[serde(rename = "worker/list")]
    WorkerList,
    #[serde(rename = "worker/get")]
    WorkerGet,

    // Orchestration: run
    #[serde(rename = "run/submit")]
    RunSubmit,
    #[serde(rename = "run/list")]
    RunList,
    #[serde(rename = "run/get")]
    RunGet,
    #[serde(rename = "run/retry")]
    RunRetry,
    #[serde(rename = "run/cancel")]
    RunCancel,

    // Orchestration: message
    #[serde(rename = "message/send")]
    MessageSend,
    #[serde(rename = "message/list")]
    MessageList,

    // Orchestration: approval
    #[serde(rename = "approval/list")]
    ApprovalList,
    #[serde(rename = "approval/decide")]
    ApprovalDecide,

    // Orchestration: coordination (child lifecycle)
    #[serde(rename = "coordination/child/list")]
    CoordinationChildList,
    #[serde(rename = "coordination/child/decide")]
    CoordinationChildDecide,

    // Orchestration: coordination (worker-safe broker surface)
    #[serde(rename = "coordination/task")]
    CoordinationTask,
    #[serde(rename = "coordination/peers")]
    CoordinationPeers,
    #[serde(rename = "coordination/send")]
    CoordinationSend,
    #[serde(rename = "coordination/requestChild")]
    CoordinationRequestChild,
    #[serde(rename = "coordination/publishArtifact")]
    CoordinationPublishArtifact,
    #[serde(rename = "coordination/reportBlocked")]
    CoordinationReportBlocked,
    #[serde(rename = "coordination/askPolicy")]
    CoordinationAskPolicy,
    #[serde(rename = "coordination/peerWorkspace")]
    CoordinationPeerWorkspace,

    // Orchestration: reconcile OMP-native agents
    #[serde(rename = "reconcile/omp")]
    ReconcileOmp,

    // Orchestration: adapter worker profiles (Worker Adapters milestone)
    #[serde(rename = "profile/register")]
    ProfileRegister,

    // Workspaces: lease and artifact operations
    #[serde(rename = "workspace/acquire")]
    WorkspaceAcquire,
    #[serde(rename = "workspace/get")]
    WorkspaceGet,
    #[serde(rename = "workspace/release")]
    WorkspaceRelease,
    #[serde(rename = "workspace/inspect")]
    WorkspaceInspect,
    #[serde(rename = "workspace/apply")]
    WorkspaceApply,
    #[serde(rename = "artifact/list")]
    ArtifactList,
    #[serde(rename = "artifact/fetch")]
    ArtifactFetch,

    // Policy: violation resolution
    #[serde(rename = "policy/violation/decide")]
    PolicyViolationDecide,
}
