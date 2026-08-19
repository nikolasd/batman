//! `batman-protocol` is the canonical owner of every BATMAN wire type.
//!
//! BATMAN is an OMP extension backed by a Rust daemon speaking JSON-RPC 2.0
//! over NDJSON. Every type in this crate that crosses the wire derives
//! `Serialize`, `Deserialize`, `JsonSchema`, and `TS` so that a later build
//! step can generate a JSON Schema document and TypeScript bindings directly
//! from this crate.

mod approval;
mod artifact;
mod coordination;
mod display;
mod event;
mod ids;
mod message;
mod method;
mod rpc;
mod run;
mod schema;
mod task;
mod version;
mod violation;
mod worker;
mod workspace;

pub use approval::{ApprovalDecision, ApprovalRequest, DecidedBy};
pub use coordination::{
    COORDINATION_PAYLOAD_MAX_BYTES, COORDINATION_RATE_LIMIT_PER_MINUTE,
    CoordinationAskPolicyParams, CoordinationChildDecision, CoordinationPeersParams,
    CoordinationPublishArtifactParams, CoordinationReportBlockedParams,
    CoordinationRequestChildParams, CoordinationSendParams, CoordinationTaskParams,
};
pub use display::{
    DisplayBackend, DisplayConfig, DisplayPlacement, DisplayPreference, DisplaySelection,
    DisplayStatus,
};
pub use event::RunFlags;
pub use event::{
    Classified, ContentClass, DiagnosticLevel, EventEnvelope, EventSource, RuntimeEvent,
    RuntimeEventKind, Timestamp, TimestampParseError,
};
pub use ids::{
    ApprovalId, ArtifactId, MessageId, OperationId, PolicyViolationId, ProjectId, RunId, TaskId,
    WorkerId,
};
pub use message::{DeliveryState, MessageKind, RunMessage};
pub use method::BatmanMethod;
pub use rpc::{
    BinarySource, ClientAuth, ClientCapabilities, ClientInfo, ClientPrincipalSummary, ClientRole,
    EVENTS_EVENT_METHOD, InitializeParams, InitializeResult, JSONRPC_VERSION, JsonRpcError,
    JsonRpcErrorResponse, JsonRpcNotification, JsonRpcRequest, JsonRpcResponse, RepositoryIdentity,
    RequestId, RuntimeCapabilities, RuntimeInfo, RuntimeStatus, error_code,
};
pub use run::{Run, RunSpec, RunState};
pub use task::TaskRef;
pub use version::{
    PROTOCOL_VERSION, ProtocolVersion, VersionRange, supported_range_text, supported_versions,
};
pub use violation::{PolicyViolationListResult, PolicyViolationSummary};
pub use worker::{Worker, WorkerProfileRef};
pub use workspace::{
    ApplyRequest, ApplyResult, ApplyStrategy, InspectRequest, InspectResult, IsolationKind,
    LeaseMode, LeaseRequest, ReleaseRequest, WorkspaceEvent, WorkspaceInfo, WorkspaceLease,
    WorkspaceState,
};

pub use artifact::{
    Artifact, ArtifactFetchRequest, ArtifactFetchResult, ArtifactKind, ArtifactListRequest,
    ArtifactListResult,
};

pub use schema::{ProtocolDocument, render_schema};

#[cfg(test)]
mod tests {
    #[test]
    fn crate_is_wired() {
        assert_eq!(env!("CARGO_PKG_NAME"), "batman-protocol");
    }
}
