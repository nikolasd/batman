//! Policy evaluation: the `PolicyEvaluator` implementing the
//! `AdapterAuthorization` trait, and [`ViolationService`] for mid-run
//! nested-worker policy violations.
//!
//! The evaluator enforces:
//! - Model allowlist (deny by default when allowlist is non-empty)
//! - Concurrency ceiling (block runs exceeding the ceiling)
//! - Nested worker policy (deny unexpected child workers)
//! - Security pattern enforcement (org-defined redaction patterns)

mod evaluate;
mod violation;

pub use evaluate::{
    DailySpend, JournalDailySpend, PolicyError, PolicyEvaluation, PolicyEvaluator, PolicyViolation,
    PolicyViolationKind,
};
pub use violation::{DecideOutcome, ViolationError, ViolationService};
