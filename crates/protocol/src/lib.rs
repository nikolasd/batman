//! `batman-protocol` is the canonical owner of every BATMAN wire type.
//!
//! BATMAN is an OMP extension backed by a Rust daemon speaking JSON-RPC 2.0
//! over NDJSON. Every type in this crate that crosses the wire derives
//! `Serialize`, `Deserialize`, `JsonSchema`, and `TS` so that a later build
//! step can generate a JSON Schema document and TypeScript bindings directly
//! from this crate.

mod approval;
mod event;
mod ids;
mod message;
mod method;
mod run;
mod rpc;
mod task;
mod worker;
mod version;

pub use approval::{ApprovalDecision, ApprovalRequest};
pub use event::{
    Classified, ContentClass, DiagnosticLevel, EventEnvelope, EventSource, RuntimeEvent,
    RuntimeEventKind, Timestamp, TimestampParseError,
};
pub use ids::{
    ApprovalId, ArtifactId, MessageId, OperationId, ProjectId, RunId, TaskId, WorkerId,
};
pub use message::{DeliveryState, MessageKind, RunMessage};
pub use method::BatmanMethod;
pub use rpc::{
    BinarySource, ClientAuth, ClientCapabilities, ClientInfo, ClientPrincipalSummary,
    ClientRole, EVENTS_EVENT_METHOD, InitializeParams, InitializeResult, JSONRPC_VERSION,
    JsonRpcError, JsonRpcErrorResponse, JsonRpcNotification, JsonRpcRequest, JsonRpcResponse,
    RepositoryIdentity, RequestId, RuntimeCapabilities, RuntimeInfo, RuntimeStatus, error_code,
};
pub use event::RunFlags;
pub use run::{Run, RunSpec, RunState};
pub use task::TaskRef;
pub use worker::{Worker, WorkerProfileRef};
pub use version::{ProtocolVersion, VersionRange};

#[cfg(test)]
mod tests {
    #[test]
    fn crate_is_wired() {
        assert_eq!(env!("CARGO_PKG_NAME"), "batman-protocol");
    }
}
