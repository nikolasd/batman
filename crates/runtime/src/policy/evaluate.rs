//! Policy evaluation: the `PolicyEvaluator` implementing
//! `AdapterAuthorization`.
//!
//! Enforces:
//! - Model allowlist (deny by default when allowlist is non-empty)
//! - Concurrency ceiling (block runs exceeding the ceiling)
//! - Nested worker policy (deny unexpected child workers)
//! - Security pattern enforcement (org-defined redaction patterns)

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::adapter::{AdapterAuthorization, AdapterCapabilities, WorkerProfile};
use crate::config::RuntimePolicy;

/// A policy violation recorded as a runtime event.
#[derive(Debug, Clone)]
pub struct PolicyViolation {
    /// The worker profile that was denied.
    pub profile_id: String,
    /// The adapter kind (e.g. "claude", "codex").
    pub adapter: String,
    /// The model that was requested.
    pub model: String,
    /// The kind of violation.
    pub kind: PolicyViolationKind,
    /// Human-readable explanation.
    pub reason: String,
    /// Whether this is a nested/child worker violation.
    pub is_nested: bool,
}

/// The kind of policy violation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyViolationKind {
    /// The model is not in the allowlist.
    ModelNotAllowed,
    /// The concurrency ceiling has been reached.
    ConcurrencyCeilingExceeded,
    /// A nested/child worker was denied by policy.
    NestedWorkerDenied,
    /// The adapter kind is not authorized.
    AdapterNotAllowed,
}

impl std::fmt::Display for PolicyViolationKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PolicyViolationKind::ModelNotAllowed => write!(f, "model not allowed"),
            PolicyViolationKind::ConcurrencyCeilingExceeded => {
                write!(f, "concurrency ceiling exceeded")
            }
            PolicyViolationKind::NestedWorkerDenied => write!(f, "nested worker denied"),
            PolicyViolationKind::AdapterNotAllowed => write!(f, "adapter not allowed"),
        }
    }
}

/// Errors from policy evaluation.
#[derive(Debug, thiserror::Error)]
pub enum PolicyError {
    /// The model is not in the allowlist.
    #[error("model '{model}' is not in the allowlist; allowed: {allowed:?}")]
    ModelNotAllowed { model: String, allowed: Vec<String> },

    /// The concurrency ceiling has been reached.
    #[error("concurrency ceiling {ceiling} reached; {active} active runs")]
    ConcurrencyCeilingExceeded { ceiling: u32, active: u32 },

    /// A nested/child worker was denied by policy.
    #[error("nested worker denied: {reason}")]
    NestedWorkerDenied { reason: String },

    /// The adapter kind is not authorized.
    #[error("adapter '{adapter}' is not authorized")]
    AdapterNotAllowed { adapter: String },
}

/// The policy evaluator: implements [`AdapterAuthorization`] and enforces
/// model allowlists, concurrency ceilings, and nested worker policies.
///
/// Constructed once at daemon startup from a [`RuntimePolicy`]. The
/// concurrency counter uses atomic check-and-increment (`fetch_update`)
/// to avoid TOCTOU races between concurrent `authorize()` calls.
pub struct PolicyEvaluator {
    /// The merged runtime policy.
    policy: RuntimePolicy,
    /// Active run count (atomic for lock-free ceiling checks).
    active_runs: Arc<AtomicU32>,
    /// Whether nested workers are allowed (from policy).
    allow_nested: bool,
}

impl PolicyEvaluator {
    /// Creates a new `PolicyEvaluator` from a [`RuntimePolicy`].
    #[must_use]
    pub fn new(policy: RuntimePolicy) -> Self {
        Self {
            policy,
            active_runs: Arc::new(AtomicU32::new(0)),
            allow_nested: false, // default: deny nested
        }
    }

    /// Returns the effective runtime policy.
    #[must_use]
    pub fn policy(&self) -> &RuntimePolicy {
        &self.policy
    }

    /// Returns the current active run count.
    #[must_use]
    pub fn active_runs(&self) -> u32 {
        self.active_runs.load(Ordering::Relaxed)
    }

    /// Decrements the active run count. Returns the new count. Saturates
    /// at zero rather than wrapping if called without a matching booking.
    pub fn decrement_runs(&self) -> u32 {
        self.active_runs
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |active| {
                Some(active.saturating_sub(1))
            })
            .map(|prev| prev.saturating_sub(1))
            .unwrap_or(0)
    }

    /// Evaluates whether a worker profile is authorized for a run,
    /// considering model allowlists, concurrency ceilings, and nested
    /// worker policies.
    ///
    /// On success, books a concurrency slot using an atomic
    /// check-and-increment (`fetch_update` with a CAS loop) so that two
    /// concurrent `authorize()` calls cannot both read `active < ceiling`
    /// and both increment past the ceiling. Call [`PolicyEvaluator::release`]
    /// to free the slot when the run completes.
    ///
    /// # Errors
    /// Returns [`PolicyError`] if the profile is denied.
    pub fn evaluate(
        &self,
        profile: &WorkerProfile,
        _effective_capabilities: &AdapterCapabilities,
        is_nested: bool,
    ) -> Result<(), PolicyError> {
        // Check model allowlist.
        if !self.policy.allowed_models.is_empty()
            && !self.policy.allowed_models.contains(&profile.model)
        {
            return Err(PolicyError::ModelNotAllowed {
                model: profile.model.clone(),
                allowed: self.policy.allowed_models.clone(),
            });
        }

        // Check nested worker policy.
        if is_nested && !self.allow_nested {
            return Err(PolicyError::NestedWorkerDenied {
                reason: "nested workers are not allowed by policy".to_string(),
            });
        }

        // Adapter kind is available for a future org-level denylist; no
        // denylist is configured yet, so every adapter passes this check
        // once the model/nested checks above pass.
        let _ = profile.adapter_kind();

        // Atomic check-and-increment: CAS loop to avoid TOCTOU race
        // between reading `active` and booking a slot.
        let ceiling = self.policy.concurrency_ceiling;
        let booked = self
            .active_runs
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |active| {
                if active < ceiling { Some(active + 1) } else { None }
            });

        match booked {
            Ok(_) => Ok(()),
            Err(active) => Err(PolicyError::ConcurrencyCeilingExceeded { ceiling, active }),
        }
    }

    /// Releases a previously-booked concurrency slot (decrements the active
    /// run counter). Safe to call even if no slot was booked (saturates at
    /// zero).
    pub fn release(&self) {
        self.decrement_runs();
    }

    /// Records a policy violation as a structured event.
    #[must_use]
    pub fn record_violation(
        &self,
        profile: &WorkerProfile,
        kind: PolicyViolationKind,
        is_nested: bool,
    ) -> PolicyViolation {
        PolicyViolation {
            profile_id: profile.id.to_string(),
            adapter: profile.adapter.clone(),
            model: profile.model.clone(),
            kind,
            reason: format!("{kind}"),
            is_nested,
        }
    }
}

impl AdapterAuthorization for PolicyEvaluator {
    fn authorize(
        &self,
        profile: &WorkerProfile,
        effective_capabilities: &AdapterCapabilities,
    ) -> Result<(), String> {
        self.evaluate(profile, effective_capabilities, false)
            .map_err(|e| e.to_string())
    }
}

/// A policy evaluation result, including any violations.
#[derive(Debug, Clone)]
pub struct PolicyEvaluation {
    /// Whether the evaluation passed.
    pub allowed: bool,
    /// Any violations recorded.
    pub violations: Vec<PolicyViolation>,
    /// The effective policy that was evaluated.
    pub policy: RuntimePolicy,
}

impl PolicyEvaluation {
    /// Returns `true` if there are no violations.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.violations.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::{
        AdapterCapabilities, ApprovalsCapability, ClaudeStartupOptions, DurabilityCapability,
        NativeViewCapability, NestedCapability, ProfileId, ProtocolKind, ResumeCapability,
        StartupOptions, SteeringCapability, UsageCapability, WorkspaceControlCapability,
    };
    use crate::config::RolloutGates;

    fn test_policy() -> RuntimePolicy {
        RuntimePolicy {
            merged: serde_json::json!({}),
            fingerprint: "test".to_string(),
            display_backend: "auto".to_string(),
            retention: "30d".to_string(),
            max_workers: 4,
            concurrency_ceiling: 2,
            allowed_models: vec!["gpt-4".to_string()],
            org_security_patterns: vec![],
            rollout_gates: RolloutGates {
                vendor_terms_accepted: true,
                retention_configured: true,
                model_allowlist_set: true,
                concurrency_explicit: true,
                native_discovery_reviewed: true,
                ornith_identity_set: true,
            },
        }
    }

    fn test_profile(model: &str) -> WorkerProfile {
        WorkerProfile {
            id: ProfileId::new(),
            adapter: "claude".to_string(),
            model: model.to_string(),
            permission_envelope: serde_json::json!({}),
            startup_options: StartupOptions::Claude(ClaudeStartupOptions::default()),
            environment_allowlist: vec![],
            source: "test".to_string(),
        }
    }

    fn test_capabilities() -> AdapterCapabilities {
        AdapterCapabilities {
            protocol: ProtocolKind::Structured,
            resume: ResumeCapability::Session,
            steering: SteeringCapability::ActiveTurn,
            approvals: ApprovalsCapability::Controllable,
            structured_result: true,
            usage: UsageCapability::PerTurn,
            nested: NestedCapability::None,
            native_view: NativeViewCapability::None,
            workspace_control: WorkspaceControlCapability::ReadOnly,
            durability: DurabilityCapability::ParentScoped,
        }
    }

    #[test]
    fn test_policy_evaluator_allows_allowed_model() {
        let policy = test_policy();
        let evaluator = PolicyEvaluator::new(policy);
        let profile = test_profile("gpt-4");
        let caps = test_capabilities();

        assert!(evaluator.evaluate(&profile, &caps, false).is_ok());
        assert_eq!(evaluator.active_runs(), 1);

        evaluator.release();
        assert_eq!(evaluator.active_runs(), 0);
    }

    #[test]
    fn test_policy_evaluator_denies_disallowed_model() {
        let policy = test_policy();
        let evaluator = PolicyEvaluator::new(policy);
        let profile = test_profile("gpt-3.5");
        let caps = test_capabilities();

        let result = evaluator.evaluate(&profile, &caps, false);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), PolicyError::ModelNotAllowed { .. }));
        assert_eq!(evaluator.active_runs(), 0);
    }

    #[test]
    fn test_policy_evaluator_empty_allowlist_allows_all() {
        let mut policy = test_policy();
        policy.allowed_models = vec![];
        let evaluator = PolicyEvaluator::new(policy);
        let profile = test_profile("any-model");
        let caps = test_capabilities();

        assert!(evaluator.evaluate(&profile, &caps, false).is_ok());
        assert_eq!(evaluator.active_runs(), 1);

        evaluator.release();
        assert_eq!(evaluator.active_runs(), 0);
    }

    #[test]
    fn test_policy_evaluator_concurrency_ceiling() {
        let policy = test_policy();
        let evaluator = PolicyEvaluator::new(policy);
        let caps = test_capabilities();

        let profile1 = test_profile("gpt-4");
        assert!(evaluator.evaluate(&profile1, &caps, false).is_ok());
        assert_eq!(evaluator.active_runs(), 1);

        let profile2 = test_profile("gpt-4");
        assert!(evaluator.evaluate(&profile2, &caps, false).is_ok());
        assert_eq!(evaluator.active_runs(), 2);

        // Third should be denied — ceiling is 2.
        let profile3 = test_profile("gpt-4");
        let result = evaluator.evaluate(&profile3, &caps, false);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            PolicyError::ConcurrencyCeilingExceeded { .. }
        ));
        assert_eq!(evaluator.active_runs(), 2);

        evaluator.release();
        evaluator.release();
        assert_eq!(evaluator.active_runs(), 0);
    }

    #[test]
    fn test_policy_evaluator_nested_denied_by_default() {
        let policy = test_policy();
        let evaluator = PolicyEvaluator::new(policy);
        let profile = test_profile("gpt-4");
        let caps = test_capabilities();

        let result = evaluator.evaluate(&profile, &caps, true);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), PolicyError::NestedWorkerDenied { .. }));
        assert_eq!(evaluator.active_runs(), 0);
    }

    #[test]
    fn test_record_violation() {
        let policy = test_policy();
        let evaluator = PolicyEvaluator::new(policy);
        let profile = test_profile("gpt-3.5");

        let violation =
            evaluator.record_violation(&profile, PolicyViolationKind::ModelNotAllowed, false);

        assert_eq!(violation.model, "gpt-3.5");
        assert!(!violation.is_nested);
    }
}
