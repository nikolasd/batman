//! Policy evaluation: the `PolicyEvaluator` implementing the
//! `AdapterAuthorization` trait.
//!
//! The evaluator enforces:
//! - Model allowlist (deny by default when allowlist is non-empty)
//! - Concurrency ceiling (block runs exceeding the ceiling)
//! - Nested worker policy (deny unexpected child workers)
//! - Security pattern enforcement (org-defined redaction patterns)

mod evaluate;

pub use evaluate::{
    PolicyError, PolicyEvaluation, PolicyEvaluator, PolicyViolation, PolicyViolationKind,
};
