//! The orchestration domain layer: projection persistence and run-state
//! transition enforcement.
//!
//! [`DomainRepository`] owns every mutating command; each appends a durable
//! event and updates its projection in one SQLite transaction.
//! [`TransitionError`] is raised when a run-state edge violates the canonical
//! lifecycle relation.

mod repository;
mod transitions;

pub use repository::{
    Committed, DomainError, DomainRepository, PolicyViolationSnapshot, RunFlag,
    broadcast_committed, embed_envelope, take_envelope,
};
pub use transitions::{TransitionError, check_transition};
