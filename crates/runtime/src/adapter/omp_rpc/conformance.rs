//! The OMP-RPC adapter's fixture/live conformance scenario suite. See
//! `batman_runtime::conformance` for the shared report/scenario contract
//! this module fills in.

use batman_runtime::adapter::{
    Adapter, AdapterKind, OmpRpcStartupOptions, ProfileId, StartupOptions, WorkerProfile,
};
use batman_runtime::conformance::report::AdapterKindLabel;
use batman_runtime::conformance::{ConformanceMode, ConformanceReport, ScenarioResult, scenario};

fn conformance_profile() -> WorkerProfile {
    WorkerProfile {
        id: ProfileId::new(),
        adapter: "ompRpc".to_string(),
        model: "lm-studio/x".to_string(),
        permission_envelope: serde_json::json!({}),
        startup_options: StartupOptions::OmpRpc(OmpRpcStartupOptions {
            profile: None,
            host_tools: None,
        }),
        environment_allowlist: Vec::new(),
        source: "conformance".to_string(),
    }
}

fn new_adapter() -> super::OmpRpcAdapter {
    super::OmpRpcAdapter::new(
        conformance_profile(),
        super::OmpRpcAdapterOptions::default(),
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
                    "omp --version reported {:?}; authReady={}",
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
        AdapterKindLabel::from(AdapterKind::OmpRpc),
        ConformanceMode::Fixture,
        version,
        declared_capabilities,
        scenarios,
    )
}

/// Runs the live conformance suite against the installed `omp` CLI.
/// Gated on `BATMAN_LIVE_OMP=1`; never runs otherwise.
///
/// # Errors
/// Returns a message if `BATMAN_LIVE_OMP` is unset.
pub async fn live_report() -> Result<ConformanceReport, String> {
    if std::env::var("BATMAN_LIVE_OMP").as_deref() != Ok("1") {
        return Err("live OMP-RPC conformance requires BATMAN_LIVE_OMP=1".to_string());
    }
    let (probe_result, version, declared_capabilities) = probe_scenario().await;
    let scenarios = vec![probe_result];
    Ok(ConformanceReport::new(
        AdapterKindLabel::from(AdapterKind::OmpRpc),
        ConformanceMode::Live,
        version,
        declared_capabilities,
        scenarios,
    ))
}
