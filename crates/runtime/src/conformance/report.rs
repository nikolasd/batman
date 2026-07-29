//! The machine-readable conformance report shape `batcave conformance
//! --output <path>` writes, and the effective-capability computation
//! `crate::adapter::registry::AdapterRegistry` gates run start on.
//!
//! Plain `serde` derives only, matching `crate::adapter::capability`'s own
//! choice: this never crosses the extension-facing generated-schema wire,
//! only ad hoc CLI-facing JSON.

use serde::Serialize;

use crate::adapter::AdapterCapabilities;

/// One scenario's outcome within a [`ConformanceReport`].
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScenarioResult {
    /// One of `super::scenario::ALL`'s exact strings.
    pub name: &'static str,
    pub passed: bool,
    /// A human-readable explanation, always present -- on failure, what
    /// went wrong; on success, what was actually observed (the concrete
    /// evidence, not just "ok"); for a not-applicable scenario, why.
    pub detail: String,
}

impl ScenarioResult {
    #[must_use]
    pub fn pass(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            name,
            passed: true,
            detail: detail.into(),
        }
    }

    #[must_use]
    pub fn fail(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            name,
            passed: false,
            detail: detail.into(),
        }
    }
}

/// A full conformance run for one adapter kind, fixture- or live-mode.
///
/// `effective_capabilities` is never the raw `declared_capabilities`
/// verbatim: it is computed by [`Self::new`] as the subset of declared
/// capabilities this report's own scenarios actually proved, per the
/// plan's "a capability whose scenario failed is removed from effective
/// capabilities" requirement. OMP-facing surfaces (`batcave adapters
/// --json`, `AdapterRegistry`) must only ever consult
/// `effective_capabilities`, never `declared_capabilities`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConformanceReport {
    /// The adapter's wire name (`"claude"`, `"codex"`, `"copilot"`, or
    /// `"ompRpc"`).
    pub adapter: String,
    pub mode: ConformanceMode,
    /// The installed vendor CLI/tool version, if the probe scenario
    /// observed one.
    pub version: Option<String>,
    pub declared_capabilities: AdapterCapabilities,
    pub effective_capabilities: AdapterCapabilities,
    pub scenarios: Vec<ScenarioResult>,
    /// True only if every scenario in `scenarios` passed. A single
    /// failing scenario still produces a complete report (every other
    /// scenario still runs) -- this flag is the aggregate, not a
    /// short-circuit.
    pub passed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ConformanceMode {
    Fixture,
    Live,
}

impl ConformanceReport {
    /// Builds a report from `declared_capabilities` (the adapter's own
    /// [`crate::adapter::Adapter::capabilities`]) and every scenario this
    /// suite ran, deriving `effective_capabilities` and `passed`.
    ///
    /// The declared-to-scenario mapping is intentionally coarse and
    /// conservative: a capability is downgraded to its adapter's
    /// "unsupported"/`none`-shaped variant only when a scenario whose
    /// name directly proves it failed. A capability with no
    /// corresponding scenario in this run is left as declared (there is
    /// nothing to disprove it with) -- callers that need a stricter
    /// "only capabilities with a passing scenario" guarantee should
    /// additionally consult `scenarios` themselves, exactly as the
    /// plan's own "every effective capability points to a passing
    /// fixture scenario" acceptance criterion is verified in this
    /// milestone's completion check, not silently inferred here.
    #[must_use]
    pub fn new(
        adapter: AdapterKindLabel,
        mode: ConformanceMode,
        version: Option<String>,
        declared_capabilities: AdapterCapabilities,
        scenarios: Vec<ScenarioResult>,
    ) -> Self {
        let passed = scenarios.iter().all(|s| s.passed);
        let effective_capabilities =
            downgrade_on_scenario_failure(declared_capabilities, &scenarios);
        Self {
            adapter: adapter.0,
            mode,
            version,
            declared_capabilities,
            effective_capabilities,
            scenarios,
            passed,
        }
    }
}

/// A thin wrapper forcing callers to pass an [`crate::adapter::AdapterKind`]
/// wire name rather than an arbitrary string.
#[derive(Debug, Clone)]
pub struct AdapterKindLabel(String);

impl From<crate::adapter::AdapterKind> for AdapterKindLabel {
    fn from(kind: crate::adapter::AdapterKind) -> Self {
        Self(kind.wire_name().to_string())
    }
}

fn downgrade_on_scenario_failure(
    mut capabilities: AdapterCapabilities,
    scenarios: &[ScenarioResult],
) -> AdapterCapabilities {
    let failed = |name: &str| scenarios.iter().any(|s| s.name == name && !s.passed);
    use crate::adapter::{
        ApprovalsCapability, NestedCapability, ResumeCapability, SteeringCapability,
        UsageCapability, WorkspaceControlCapability,
    };
    if failed(super::scenario::APPROVAL) {
        capabilities.approvals = ApprovalsCapability::None;
    }
    if failed(super::scenario::FOLLOW_UP) {
        capabilities.steering = SteeringCapability::None;
    }
    if failed(super::scenario::SESSION_RESUME) {
        capabilities.resume = ResumeCapability::None;
    }
    if failed(super::scenario::ISOLATED_WRITE) {
        capabilities.workspace_control = WorkspaceControlCapability::ReadOnly;
    }
    if failed(super::scenario::MANAGED_NESTING_REJECTION) {
        // A failed rejection scenario means the adapter did NOT prove
        // `nested: none` -- the safe direction to move is to whatever
        // this capability's own most restrictive variant is, never
        // toward `Managed` (that direction requires a passing proof,
        // never the absence of one).
        capabilities.nested = NestedCapability::None;
    }
    capabilities
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::{
        AdapterKind, ApprovalsCapability, DurabilityCapability, NativeViewCapability,
        NestedCapability, ProtocolKind, ResumeCapability, SteeringCapability, UsageCapability,
        WorkspaceControlCapability,
    };

    fn fully_capable() -> AdapterCapabilities {
        AdapterCapabilities {
            protocol: ProtocolKind::Structured,
            resume: ResumeCapability::Session,
            steering: SteeringCapability::ActiveTurn,
            approvals: ApprovalsCapability::Controllable,
            structured_result: true,
            usage: UsageCapability::PerTurn,
            nested: NestedCapability::None,
            native_view: NativeViewCapability::None,
            workspace_control: WorkspaceControlCapability::Write,
            durability: DurabilityCapability::RuntimeScoped,
        }
    }

    #[test]
    fn a_failed_approval_scenario_downgrades_only_the_approvals_capability() {
        let scenarios = vec![
            ScenarioResult::pass(super::super::scenario::PROBE, "ok"),
            ScenarioResult::fail(super::super::scenario::APPROVAL, "boom"),
        ];
        let report = ConformanceReport::new(
            AdapterKindLabel::from(AdapterKind::Claude),
            ConformanceMode::Fixture,
            None,
            fully_capable(),
            scenarios,
        );
        assert_eq!(
            report.effective_capabilities.approvals,
            ApprovalsCapability::None
        );
        // Every other capability is untouched by an unrelated scenario's
        // failure.
        assert_eq!(
            report.effective_capabilities.steering,
            SteeringCapability::ActiveTurn
        );
        assert_eq!(
            report.effective_capabilities.resume,
            ResumeCapability::Session
        );
        assert!(
            !report.passed,
            "one failing scenario must mark the whole report unpassed"
        );
    }

    #[test]
    fn every_scenario_passing_leaves_effective_capabilities_equal_to_declared() {
        let scenarios = vec![
            ScenarioResult::pass(super::super::scenario::PROBE, "ok"),
            ScenarioResult::pass(super::super::scenario::APPROVAL, "ok"),
        ];
        let declared = fully_capable();
        let report = ConformanceReport::new(
            AdapterKindLabel::from(AdapterKind::Codex),
            ConformanceMode::Fixture,
            Some("1.2.3".to_string()),
            declared,
            scenarios,
        );
        assert_eq!(report.effective_capabilities, declared);
        assert!(report.passed);
        assert_eq!(report.version, Some("1.2.3".to_string()));
    }

    #[test]
    fn a_failed_nesting_rejection_scenario_forces_nested_to_none() {
        let mut declared = fully_capable();
        declared.nested = NestedCapability::Observable;
        let scenarios = vec![ScenarioResult::fail(
            super::super::scenario::MANAGED_NESTING_REJECTION,
            "boom",
        )];
        let report = ConformanceReport::new(
            AdapterKindLabel::from(AdapterKind::Copilot),
            ConformanceMode::Fixture,
            None,
            declared,
            scenarios,
        );
        assert_eq!(report.effective_capabilities.nested, NestedCapability::None);
    }
}
