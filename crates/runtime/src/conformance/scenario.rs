//! The canonical conformance scenario names every adapter's fixture suite
//! must cover, exactly once, with these exact strings -- so a
//! [`super::report::ConformanceReport`]'s `scenarios` are comparable
//! across adapters, and so `crate::adapter::registry`'s effective-
//! capability computation can name, unambiguously, which scenario backs
//! (or fails to back) which capability.
//!
//! Per the Worker Adapters plan's Task 8: "Implement the shared
//! scenarios: probe; read-only start/progress; isolated write;
//! follow-up; approval; every cancellation scope; session resume; OMP
//! reconnect behavior; runtime restart behavior; result/transcript/
//! usage/artifacts; native discovery; redaction; managed-nesting
//! rejection; and unexpected-child observation normalization."
//!
//! Not every scenario applies to every adapter (e.g. `VENDOR_RECONNECT`
//! is OMP-RPC-specific; a foreign adapter's suite reports it `outcome:
//! "pass"` with a detail explaining it is not applicable, never silently
//! omits it -- omission and "not applicable" must stay distinguishable
//! from an unrun scenario).

/// A no-model-call probe of version, auth readiness, and capabilities.
pub const PROBE: &str = "probe";
/// Starting a run and observing progress without ever writing outside
/// the worker's own workspace.
pub const READ_ONLY_START_AND_PROGRESS: &str = "read_only_start_and_progress";
/// A write confined to the worker's own isolated workspace.
pub const ISOLATED_WRITE: &str = "isolated_write";
/// Delivering a follow-up/steer message to an already-started adapter.
pub const FOLLOW_UP: &str = "follow_up";
/// An approval request the adapter reports and this runtime resolves.
pub const APPROVAL: &str = "approval";
/// Every [`crate::adapter::CancelScope`] variant actually terminates the
/// vendor process/session at the requested scope.
pub const CANCELLATION_SCOPE: &str = "cancellation_scope";
/// Resuming a previously-established vendor session.
pub const SESSION_RESUME: &str = "session_resume";
/// OMP-RPC-specific: a worker-MCP subprocess reconnecting to the same
/// live vendor process with the same token. Adapters other than OMP-RPC
/// report this scenario as passed/not-applicable.
pub const VENDOR_RECONNECT: &str = "vendor_reconnect";
/// Behavior across a runtime restart (durability capability proof).
pub const RUNTIME_RESTART: &str = "runtime_restart";
/// Result, transcript, usage, and artifact events all correlate to one
/// run.
pub const RESULT_USAGE_ARTIFACTS: &str = "result_usage_artifacts";
/// Native user/project skill, agent, plugin, hook, and MCP discovery is
/// never suppressed by this adapter's own command line.
pub const NATIVE_DISCOVERY: &str = "native_discovery";
/// Secrets and hidden reasoning never reach a journaled event.
pub const REDACTION: &str = "redaction";
/// A foreign adapter never advertises `nested: managed`; only OMP-native
/// nesting may, and only through OMP's own limits.
pub const MANAGED_NESTING_REJECTION: &str = "managed_nesting_rejection";
/// An unexpected vendor-spawned child is normalized as
/// `NestedWorkerObserved` without upgrading the declared `nested`
/// capability.
pub const UNEXPECTED_CHILD_OBSERVATION: &str = "unexpected_child_observation";

/// Every canonical scenario name, in the plan's own order. A conformance
/// suite that reports a name outside this list, or omits one on this
/// list entirely, is malformed -- see
/// `super::report::ConformanceReport::validate_scenario_coverage`.
pub const ALL: [&str; 14] = [
    PROBE,
    READ_ONLY_START_AND_PROGRESS,
    ISOLATED_WRITE,
    FOLLOW_UP,
    APPROVAL,
    CANCELLATION_SCOPE,
    SESSION_RESUME,
    VENDOR_RECONNECT,
    RUNTIME_RESTART,
    RESULT_USAGE_ARTIFACTS,
    NATIVE_DISCOVERY,
    REDACTION,
    MANAGED_NESTING_REJECTION,
    UNEXPECTED_CHILD_OBSERVATION,
];
#[cfg(test)]
mod tests {
    use super::ALL;

    #[test]
    fn every_name_is_unique() {
        let mut seen = std::collections::HashSet::new();
        for name in ALL {
            assert!(seen.insert(name), "duplicate scenario name: {name}");
        }
    }
}
