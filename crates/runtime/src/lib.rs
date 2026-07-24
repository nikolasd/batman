pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub mod adapter;
pub mod approval;
pub mod coordination;
pub mod db;
pub mod domain;
pub mod ipc;
pub mod lifecycle;
pub mod paths;
pub mod security;
pub mod service;

pub use approval::{
    ApprovalCallback, ApprovalError, ApprovalService, DecideOutcome, NoopApprovalCallback,
};
pub use coordination::{CoordinationBroker, CoordinationError, ScopeTokenStore};
pub use db::{DatabaseHandle, DbError};
pub use domain::{Committed, DomainError, DomainRepository, TransitionError};
pub use ipc::{IpcError, Server, ServerConfig};
pub use lifecycle::should_idle_shutdown;
pub use paths::{PathError, RuntimePaths, repository_id_from_canonical_root};
pub use security::{SecurityError, StateRoot};
pub use service::{FakeRunDriver, OrchestrationService, RunDriver, RunDriverContext, ServiceError};
