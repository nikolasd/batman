//! The adapter conformance runner: fixture (default, always safe, zero
//! model calls) and live (explicitly gated per adapter) scenario suites
//! that decide which of an adapter's *declared* capabilities are actually
//! *effective* -- the only set `crate::adapter::registry::AdapterRegistry`
//! and `batcave adapters --json` may ever expose to OMP.
//!
//! Each adapter owns its own scenario implementations in a `conformance`
//! submodule beside its `mod.rs` (`crate::adapter::claude::conformance`,
//! `crate::adapter::codex::conformance`, and so on), covering every name
//! in [`scenario::ALL`] exactly once. This module only dispatches by
//! [`crate::adapter::AdapterKind`] and defines the shared report shape --
//! it never itself decides pass/fail for any adapter's scenario.

pub mod report;
pub mod scenario;

pub use report::{ConformanceMode, ConformanceReport, ScenarioResult};

use crate::adapter::AdapterKind;

/// Runs one adapter kind's full fixture conformance suite (never a model
/// call) and returns its report.
pub async fn run_fixture_conformance(kind: AdapterKind) -> ConformanceReport {
    match kind {
        AdapterKind::Claude => crate::adapter::claude::conformance::fixture_report().await,
        AdapterKind::Codex => crate::adapter::codex::conformance::fixture_report().await,
        AdapterKind::Copilot => crate::adapter::copilot::conformance::fixture_report().await,
        AdapterKind::OmpRpc => crate::adapter::omp_rpc::conformance::fixture_report().await,
    }
}

/// Runs one adapter kind's live conformance suite against its installed
/// vendor CLI. Callers must have already checked this adapter kind's own
/// `BATMAN_LIVE_<ADAPTER>` environment gate -- this function itself
/// performs no gating, so it must never be reachable from a default (CI)
/// test run or an ungated CLI invocation.
///
/// # Errors
/// Returns a plain message if this adapter kind has no live-mode gate
/// satisfied in the current environment, or the installed vendor CLI is
/// unavailable.
pub async fn run_live_conformance(kind: AdapterKind) -> Result<ConformanceReport, String> {
    match kind {
        AdapterKind::Claude => crate::adapter::claude::conformance::live_report().await,
        AdapterKind::Codex => crate::adapter::codex::conformance::live_report().await,
        AdapterKind::Copilot => crate::adapter::copilot::conformance::live_report().await,
        AdapterKind::OmpRpc => crate::adapter::omp_rpc::conformance::live_report().await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn every_adapter_kind_produces_a_fixture_report() {
        for kind in [
            AdapterKind::Claude,
            AdapterKind::Codex,
            AdapterKind::Copilot,
            AdapterKind::OmpRpc,
        ] {
            let report = run_fixture_conformance(kind).await;
            assert_eq!(report.adapter, kind.wire_name());
            assert_eq!(report.mode, ConformanceMode::Fixture);
            assert!(
                !report.scenarios.is_empty(),
                "{kind} fixture report must run at least one scenario"
            );
        }
    }
}
