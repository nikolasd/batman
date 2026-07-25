//! The Claude adapter's fixture/live conformance scenario suite. See
//! `batman_runtime::conformance` for the shared report/scenario contract
//! this module fills in.
//!
//! Uses `batman_runtime::` (this crate's own external path) rather than
//! `crate::`, exactly like every other file in this directory -- see
//! `super::super::command`'s module doc for why (this file compiles
//! unchanged both inside the library and when pulled into a standalone
//! integration test binary via `#[path = "..."] mod claude;`, e.g.
//! `tests/claude_live.rs`).

use batman_protocol::{RunId, TaskId, WorkerId};
use batman_runtime::adapter::{Adapter, AdapterKind, ClaudeStartupOptions};
use batman_runtime::conformance::report::AdapterKindLabel;
use batman_runtime::conformance::{ConformanceMode, ConformanceReport, ScenarioResult, scenario};

fn new_adapter() -> super::ClaudeAdapter {
    super::ClaudeAdapter::new(
        ClaudeStartupOptions::default(),
        std::env::temp_dir(),
        Vec::new(),
        RunId::new(),
        TaskId::new(),
        WorkerId::new(),
        None,
    )
}

async fn probe_scenario() -> (
    ScenarioResult,
    Option<String>,
    batman_runtime::adapter::AdapterCapabilities,
) {
    let adapter = new_adapter();
    let declared_capabilities = adapter.capabilities();
    match adapter.probe().await {
        Ok(result) => (
            ScenarioResult::pass(
                scenario::PROBE,
                format!(
                    "claude --version reported {:?}; authReady={}",
                    result.version, result.auth_ready
                ),
            ),
            result.version,
            declared_capabilities,
        ),
        Err(err) => (
            ScenarioResult::fail(scenario::PROBE, format!("probe failed: {err}")),
            None,
            declared_capabilities,
        ),
    }
}

/// Runs every scenario this adapter can prove without a model call.
pub async fn fixture_report() -> ConformanceReport {
    let (probe_result, version, declared_capabilities) = probe_scenario().await;
    let scenarios = vec![probe_result];
    ConformanceReport::new(
        AdapterKindLabel::from(AdapterKind::Claude),
        ConformanceMode::Fixture,
        version,
        declared_capabilities,
        scenarios,
    )
}

/// Runs the live conformance suite against the installed `claude` CLI.
/// Gated on `BATMAN_LIVE_CLAUDE=1`; never runs otherwise.
///
/// # Errors
/// Returns a message if `BATMAN_LIVE_CLAUDE` is unset.
pub async fn live_report() -> Result<ConformanceReport, String> {
    if std::env::var("BATMAN_LIVE_CLAUDE").as_deref() != Ok("1") {
        return Err("live Claude conformance requires BATMAN_LIVE_CLAUDE=1".to_string());
    }
    let (probe_result, version, declared_capabilities) = probe_scenario().await;
    let scenarios = vec![probe_result];
    Ok(ConformanceReport::new(
        AdapterKindLabel::from(AdapterKind::Claude),
        ConformanceMode::Live,
        version,
        declared_capabilities,
        scenarios,
    ))
}
