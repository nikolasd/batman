//! The audited coordination broker: worker-safe messaging, task/peer
//! introspection, and the child-worker request seam, all scoped to a
//! reconnect-capable [`scope_token::ScopeTokenStore`] credential.

mod broker;
mod rate_limit;
mod scope_token;

pub use broker::{CoordinationBroker, CoordinationError};
pub use rate_limit::{RateLimitError, RateLimiter};
pub use scope_token::{
    AncestryError, PidAncestryChecker, ScopeTokenStore, ScopeTokenVerifier, SystemPidAncestryChecker,
    VendorProcessIdentity,
};
