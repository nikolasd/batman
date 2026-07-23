//! `batman-protocol` is the canonical owner of every BATMAN wire type.
//!
//! BATMAN is an OMP extension backed by a Rust daemon speaking JSON-RPC 2.0
//! over NDJSON. Every type in this crate that crosses the wire derives
//! `Serialize`, `Deserialize`, `JsonSchema`, and `TS` so that a later build
//! step can generate a JSON Schema document and TypeScript bindings directly
//! from this crate.

mod event;
mod ids;
mod rpc;
mod version;

pub use event::{
    Classified, ContentClass, DiagnosticLevel, EventEnvelope, EventSource, RuntimeEvent, Timestamp,
    TimestampParseError,
};
pub use ids::{ApprovalId, ArtifactId, MessageId, OperationId, ProjectId, RunId, TaskId, WorkerId};
pub use rpc::{
    BatmanMethod, BinarySource, ClientAuth, ClientCapabilities, ClientInfo, ClientPrincipalSummary,
    ClientRole, EVENTS_EVENT_METHOD, InitializeParams, InitializeResult, JSONRPC_VERSION,
    JsonRpcError, JsonRpcErrorResponse, JsonRpcNotification, JsonRpcRequest, JsonRpcResponse,
    RepositoryIdentity, RequestId, RuntimeCapabilities, RuntimeInfo, RuntimeStatus, error_code,
};
pub use version::{ProtocolVersion, VersionRange};

#[cfg(test)]
mod tests {
    #[test]
    fn crate_is_wired() {
        assert_eq!(env!("CARGO_PKG_NAME"), "batman-protocol");
    }
}
