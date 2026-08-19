//! Runtime orchestration RPC service.
//!
//! [`OrchestrationService`] routes every Task 1 orchestration method to
//! typed [`crate::domain::DomainRepository`] commands or read-only query
//! closures. [`RunDriver`] is the seam through which `run/submit` delegates
//! adapter-backed run start; [`FakeRunDriver`] drives deterministic
//! `queued -> starting -> working` transitions for tests and fixtures.

mod orchestration;
pub(crate) mod query;
mod run_driver;

pub use orchestration::{OrchestrationService, ServiceError};
pub use run_driver::{AdapterFuture, CancelOutcome, FakeRunDriver, RunDriver, RunDriverContext};
