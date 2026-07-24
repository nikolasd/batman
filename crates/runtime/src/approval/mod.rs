//! The correlated approval flow: request creation paired with a run
//! pause, ownership-enforced decisions, and adapter-callback semantics.

mod service;

pub use service::{
    ApprovalCallback, ApprovalError, ApprovalService, CallbackFuture, DecideOutcome,
    NoopApprovalCallback,
};
