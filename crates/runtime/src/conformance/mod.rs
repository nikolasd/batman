//! The adapter conformance runner: fixture (default, always safe, zero
//! model calls) and live scenario suites that decide which of an adapter's
//! *declared* capabilities are actually *effective* -- the only set
//! `crate::adapter::registry::AdapterRegistry` and `batcave adapters --json`
//! may ever expose to OMP.
//!
//! Each adapter owns its own scenario implementations in a `conformance`
//! submodule beside its `mod.rs` (`crate::adapter::claude::conformance`,
//! `crate::adapter::codex::conformance`, and so on), covering every name
//! in [`scenario::ALL`] exactly once. This module only dispatches by
//! [`crate::adapter::AdapterKind`] and defines the shared report shape --
//! it never itself decides pass/fail for any adapter's scenario.
//!
//! **Gating model.** Vendor CLIs are ordinary installed dependencies, and
//! which adapters a run may use is decided by org policy
//! (`crate::config::RuntimePolicy::allowed_adapters`) plus the real
//! availability probe -- never by an environment variable a deployment has
//! to remember to set. So real invocation is the default, and the single
//! opt-out [`DISABLE_VENDOR_CLI_ENV`] forbids only the vendor processes
//! this runtime would spawn purely to *observe* a CLI.

pub mod capture;
pub mod report;
pub mod scenario;
pub mod scrub;

pub use report::{ConformanceMode, ConformanceReport, ScenarioResult};

use crate::adapter::{AdapterCapabilities, AdapterKind};

/// Set to `"1"` to forbid every vendor-CLI process this runtime would
/// spawn purely to *observe* the CLI -- conformance live suites and the
/// availability probe. A development and CI switch only: production
/// leaves it unset, and which adapters a run may use is decided by org
/// policy (`RuntimePolicy::allowed_adapters`), not by this variable.
///
/// It deliberately does **not** gate `Adapter::start()`: run execution is
/// authorized by policy, so a development switch must never be able to
/// silently stop production work.
pub const DISABLE_VENDOR_CLI_ENV: &str = "BATMAN_DISABLE_VENDOR_CLI";

/// Whether observation-only vendor-CLI invocation is disabled.
#[must_use]
pub fn vendor_cli_invocation_disabled() -> bool {
    std::env::var(DISABLE_VENDOR_CLI_ENV).as_deref() == Ok("1")
}

/// An honest, non-spawning result for a scenario that can only be proven by
/// a real vendor-CLI spawn, for use when [`vendor_cli_invocation_disabled`]
/// is set. Mirrors [`probe_availability`]'s reasoning for PROBE (skip, don't
/// fabricate a pass) but returns `fail` rather than `pass`, because unlike
/// PROBE these scenarios have no adapter-neutral "not applicable" state --
/// skipping them must not silently count as proof they work.
#[must_use]
pub fn vendor_cli_required_scenario(name: &'static str) -> ScenarioResult {
    ScenarioResult::fail(
        name,
        format!(
            "skipped: real vendor CLI invocation is disabled ({DISABLE_VENDOR_CLI_ENV}=1); this \
             scenario has no fixture-only proof and can only run via live_report \
             ({DISABLE_VENDOR_CLI_ENV} unset)"
        ),
    )
}

/// The exact detail [`probe_availability`] uses when the kill switch skips
/// a PROBE, so every adapter's own `probe_scenario` reports the skip
/// identically rather than each inventing its own wording.
#[must_use]
pub fn vendor_cli_skipped_probe() -> ScenarioResult {
    ScenarioResult::pass(
        scenario::PROBE,
        format!("vendor CLI probe skipped: {DISABLE_VENDOR_CLI_ENV}=1"),
    )
}

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
/// vendor CLI. Each adapter's `live_report` self-checks
/// [`vendor_cli_invocation_disabled`] and returns `Err` when the kill
/// switch is set, so this function needs no gating of its own.
///
/// # Errors
/// Returns a plain message when the kill switch is set or the installed
/// vendor CLI is unavailable.
pub async fn run_live_conformance(kind: AdapterKind) -> Result<ConformanceReport, String> {
    match kind {
        AdapterKind::Claude => crate::adapter::claude::conformance::live_report().await,
        AdapterKind::Codex => crate::adapter::codex::conformance::live_report().await,
        AdapterKind::Copilot => crate::adapter::copilot::conformance::live_report().await,
        AdapterKind::OmpRpc => crate::adapter::omp_rpc::conformance::live_report().await,
    }
}

/// How long a probe result stays fresh. Long enough that a burst of run
/// submits re-spawns no binary, short enough that installing or
/// authenticating a CLI is picked up without restarting the daemon.
const PROBE_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(60);

/// Caches [`probe_availability`] results by adapter kind. Mirrors
/// `crate::display::herdr::HerdrDisplay::probe`'s cache exactly: the guard
/// is dropped before the `await` and re-taken to store, so it is never held
/// across a suspension point.
static PROBE_CACHE: std::sync::LazyLock<
    parking_lot::Mutex<
        std::collections::HashMap<AdapterKind, (std::time::Instant, ScenarioResult)>,
    >,
> = std::sync::LazyLock::new(|| parking_lot::Mutex::new(std::collections::HashMap::new()));

/// Probes the installed vendor CLI for `kind` -- version handshake only,
/// never a model call -- cached for 60 seconds so repeated run submits do
/// not re-spawn the binary.
///
/// Honors [`DISABLE_VENDOR_CLI_ENV`] **permissively**: when the switch is
/// set this returns a passing result without spawning anything, and does
/// not cache it. Pass rather than fail is deliberate -- the switch is a
/// development and CI convenience, and turning it into a denial would make
/// every run in CI unauthorized.
pub async fn probe_availability(kind: AdapterKind) -> ScenarioResult {
    if vendor_cli_invocation_disabled() {
        return vendor_cli_skipped_probe();
    }

    {
        let cache = PROBE_CACHE.lock();
        if let Some((observed_at, result)) = cache.get(&kind)
            && observed_at.elapsed() < PROBE_CACHE_TTL
        {
            return result.clone();
        }
    }

    let (result, _version, _capabilities): (ScenarioResult, Option<String>, AdapterCapabilities) =
        match kind {
            AdapterKind::Claude => crate::adapter::claude::conformance::probe_scenario().await,
            AdapterKind::Codex => crate::adapter::codex::conformance::probe_scenario().await,
            AdapterKind::Copilot => crate::adapter::copilot::conformance::probe_scenario().await,
            AdapterKind::OmpRpc => crate::adapter::omp_rpc::conformance::probe().await,
        };

    PROBE_CACHE
        .lock()
        .insert(kind, (std::time::Instant::now(), result.clone()));
    result
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
