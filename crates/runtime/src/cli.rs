//! The `batcave` command-line interface: `serve`, `status`, `stop`,
//! `version`, `schema`, `monitor`, `audit`, `doctor`, and
//! `coordination-mcp`. This layer only
//! parses arguments, resolves the state root when `--state-dir` is omitted,
//! and maps [`crate::lifecycle`] outcomes to process exit codes; all
//! behaviour lives in the library.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

use batman_runtime::VERSION;
/// The committed fixture-mode baseline loaded from
/// `fixtures/conformance/fixture-mode-baseline.json`. Maps each adapter to
/// the scenario names that are expected to fail in fixture mode.
#[derive(serde::Deserialize)]
struct FixtureBaseline {
    #[serde(rename = "expectedFailures")]
    expected_failures: std::collections::HashMap<String, Vec<String>>,
}

/// The BATMAN runtime daemon.
#[derive(Parser)]
#[command(name = "batcave", version, about = "The BATMAN runtime daemon")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Serve the runtime socket protocol for a repository.
    Serve {
        /// The BATMAN state root. Defaults to the resolved state root.
        #[arg(long)]
        state_dir: Option<PathBuf>,
        /// The repository this runtime instance serves.
        #[arg(long)]
        repo: PathBuf,
        /// Exit after this many seconds with no connections and no active
        /// runs. Omit to run until signalled.
        #[arg(long)]
        idle_seconds: Option<u64>,
        /// Run in the foreground, logging structured records to stderr rather
        /// than to `runtime.log`.
        #[arg(long)]
        foreground: bool,
        /// Path to the org-level configuration file.
        #[arg(long = "org-config")]
        org_config: Option<PathBuf>,
        /// Path to the repo-level configuration file.
        #[arg(long = "repo-config")]
        repo_config: Option<PathBuf>,
        /// Path to the user-level configuration file.
        #[arg(long = "user-config")]
        user_config: Option<PathBuf>,
    },
    /// Print the runtime's `runtime/status` snapshot as JSON.
    Status {
        /// Retry connecting for up to this many seconds (startup races).
        #[arg(long)]
        wait_seconds: Option<u64>,
        /// The BATMAN state root. Defaults to the resolved state root.
        #[arg(long)]
        state_dir: Option<PathBuf>,
        /// The repository whose runtime to query.
        #[arg(long)]
        repo: PathBuf,
    },
    /// Gracefully stop the runtime serving a repository.
    Stop {
        /// The BATMAN state root. Defaults to the resolved state root.
        #[arg(long)]
        state_dir: Option<PathBuf>,
        /// The repository whose runtime to stop.
        #[arg(long)]
        repo: PathBuf,
    },
    /// Display runtime events for one or all runs.
    Monitor {
        /// The BATMAN state root. Defaults to the resolved state root.
        #[arg(long)]
        state_dir: Option<PathBuf>,
        /// The repository whose events to display.
        #[arg(long)]
        repo: PathBuf,
        /// Render only the run matching this id (full, un-truncated form).
        #[arg(long)]
        run_id: Option<String>,
    },
    /// Print the runtime version.
    Version,
    /// Print the canonical JSON Schema document to stdout.
    Schema,
    /// Audit commands for managing event retention and export.
    Audit {
        #[command(subcommand)]
        command: AuditCommand,
    },
    /// Run diagnostic checks on the runtime state and configuration.
    Doctor {
        /// The BATMAN state root. Defaults to the resolved state root.
        #[arg(long)]
        state_dir: Option<PathBuf>,
        /// The repository to diagnose.
        #[arg(long)]
        repo: PathBuf,
        /// Output as JSON (machine-readable).
        #[arg(long)]
        json: bool,
        /// Organization policy layer, as an explicit file path.
        #[arg(long)]
        org_config: Option<PathBuf>,
        /// Repository policy layer, as an explicit file path.
        #[arg(long)]
        repo_config: Option<PathBuf>,
        /// User policy layer, as an explicit file path.
        #[arg(long)]
        user_config: Option<PathBuf>,
    },
    /// Serve the worker-coordination MCP proxy for one run over stdio.
    CoordinationMcp {
        /// The BATMAN state root. Defaults to the resolved state root.
        #[arg(long)]
        state_dir: Option<PathBuf>,
        /// The repository this run belongs to.
        #[arg(long)]
        repo: PathBuf,
        /// The run this MCP proxy is scoped to.
        #[arg(long)]
        run_id: String,
    },
    /// Probe the display backend status.
    Display {
        #[command(subcommand)]
        probe: DisplayCommand,
    },
    /// Run conformance tests for one or all adapters.
    Conformance {
        /// Adapter name: claude, codex, copilot, ompRpc, or all.
        #[arg(long)]
        adapter: String,
        /// Use fixture mode (no real model calls).
        #[arg(long, default_value_t = false)]
        fixture: bool,
        /// Use live mode (real vendor CLI), gated per adapter.
        #[arg(long, default_value_t = false)]
        live: bool,
        /// Output file path for the conformance report.
        #[arg(long)]
        output: PathBuf,
    },
    /// List registered adapters with declared vs effective capabilities.
    Adapters {
        /// Output as JSON.
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// Re-record adapter fixtures from a real vendor CLI.
    Capture {
        /// Adapter name: claude, codex, copilot, or ompRpc. No "all" --
        /// capture spends real vendor turns, so it is always explicit.
        #[arg(long)]
        adapter: String,
        /// Regenerate only this fixture filename instead of every entry.
        #[arg(long)]
        fixture: Option<String>,
        /// Print scrubbed output instead of overwriting the committed files.
        #[arg(long, default_value_t = false)]
        dry_run: bool,
    },
}

#[derive(Subcommand)]
enum AuditCommand {
    /// Export events to a JSONL file.
    Export {
        /// The BATMAN state root. Defaults to the resolved state root.
        #[arg(long)]
        state_dir: Option<PathBuf>,
        /// The repository whose events to export.
        #[arg(long)]
        repo: PathBuf,
        /// Export events from this timestamp (ISO 8601).
        #[arg(long)]
        from: Option<String>,
        /// Export events up to this timestamp (ISO 8601).
        #[arg(long)]
        to: Option<String>,
        /// The output file path (defaults to stdout).
        #[arg(long)]
        output: PathBuf,
    },
}

#[derive(Subcommand)]
enum DisplayCommand {
    /// Probe the display backend status.
    Probe {
        /// Backend to probe: herdr, tmux, or terminal.
        backend: String,
        /// Output as JSON.
        #[arg(long, default_value_t = false)]
        json: bool,
    },
}

/// The CLI's entry point, called from `main.rs`.
pub async fn run() -> ExitCode {
    let cli = Cli::parse();

    match cli.command {
        Command::Serve {
            state_dir,
            repo,
            idle_seconds,
            foreground,
            org_config,
            repo_config,
            user_config,
        } => {
            run_serve(
                state_dir,
                repo,
                idle_seconds,
                foreground,
                org_config,
                repo_config,
                user_config,
            )
            .await
        }
        Command::Status {
            wait_seconds,
            state_dir,
            repo,
        } => run_status(wait_seconds, state_dir, repo).await,
        Command::Stop { state_dir, repo } => run_stop(state_dir, repo).await,
        Command::Monitor {
            state_dir,
            repo,
            run_id,
        } => run_monitor(state_dir, repo, run_id).await,
        Command::Version => {
            println!("batcave {VERSION}");
            ExitCode::SUCCESS
        }
        Command::Schema => run_schema().await,
        Command::Audit {
            command:
                AuditCommand::Export {
                    state_dir,
                    repo,
                    from,
                    to,
                    output,
                },
        } => run_audit_export(state_dir, repo, from, to, output).await,
        Command::Doctor {
            state_dir,
            repo,
            json,
            org_config,
            repo_config,
            user_config,
        } => run_doctor(state_dir, repo, json, org_config, repo_config, user_config).await,
        Command::CoordinationMcp {
            state_dir,
            repo,
            run_id,
        } => run_coordination_mcp(state_dir, repo, run_id).await,
        Command::Display {
            probe: DisplayCommand::Probe { backend, json },
        } => run_display_probe(backend, json).await,
        Command::Conformance {
            adapter,
            fixture,
            live,
            output,
        } => run_conformance(adapter, fixture, live, output).await,
        Command::Adapters { json } => run_adapters(json).await,
        Command::Capture {
            adapter,
            fixture,
            dry_run,
        } => run_capture(adapter, fixture, dry_run).await,
    }
}

/// Reads the launcher's `BATMAN_BINARY_SOURCE` hint. The extension sets it
/// when it spawns the daemon (`packages/extension/src/runtime.ts`); a
/// hand-run `batcave` leaves it unset, which is `Unknown` rather than an
/// error -- the field is diagnostic, never load-bearing.
fn binary_source_from_env() -> batman_protocol::BinarySource {
    use batman_protocol::BinarySource;
    match std::env::var("BATMAN_BINARY_SOURCE").as_deref() {
        Ok("override") => BinarySource::Override,
        Ok("package") => BinarySource::Package,
        _ => BinarySource::Unknown,
    }
}

/// Runs `batcave serve`: acquires the single-instance lock, starts the IPC
/// server, and serves until signalled, idle-shutdown, or in-band stop.
async fn run_serve(
    state_dir: Option<PathBuf>,
    repo: PathBuf,
    idle_seconds: Option<u64>,
    foreground: bool,
    org_config: Option<PathBuf>,
    repo_config: Option<PathBuf>,
    user_config: Option<PathBuf>,
) -> ExitCode {
    use batman_runtime::lifecycle::{self, ServeOptions};

    let state_dir = match resolve_state_dir(state_dir) {
        Ok(dir) => dir,
        Err(err) => return fail(&err),
    };

    let options = ServeOptions {
        state_dir,
        repo,
        idle_seconds,
        foreground,
        binary_source: binary_source_from_env(),
        org_config,
        repo_config,
        user_config,
    };

    match lifecycle::serve(&options).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(lifecycle::ServeError::AlreadyRunning(already)) => {
            // Machine-readable identity of the live runtime, on stdout.
            println!(
                "{}",
                serde_json::to_string(&already).expect("AlreadyRunning serializes")
            );
            // EX_TEMPFAIL (73): a peer already holds the lock.
            ExitCode::from(73)
        }
        Err(err) => fail(&err),
    }
}

/// Runs `batcave status`: connects to the runtime, queries `runtime/status`,
/// and prints the result as JSON.
async fn run_status(
    wait_seconds: Option<u64>,
    state_dir: Option<PathBuf>,
    repo: PathBuf,
) -> ExitCode {
    use batman_runtime::lifecycle::{self, StatusOptions};

    let state_dir = match resolve_state_dir(state_dir) {
        Ok(dir) => dir,
        Err(err) => return fail(&err),
    };

    let options = StatusOptions {
        state_dir,
        repo,
        wait_seconds,
    };

    match lifecycle::status(&options).await {
        Ok(value) => {
            println!(
                "{}",
                serde_json::to_string(&value).expect("status serializes")
            );
            ExitCode::SUCCESS
        }
        Err(err) => fail(&err),
    }
}

/// Runs `batcave stop`: signals a live runtime and waits for it to shut down.
async fn run_stop(state_dir: Option<PathBuf>, repo: PathBuf) -> ExitCode {
    use batman_runtime::lifecycle::{self, StopOptions};

    let state_dir = match resolve_state_dir(state_dir) {
        Ok(dir) => dir,
        Err(err) => return fail(&err),
    };

    let options = StopOptions { state_dir, repo };

    match lifecycle::stop(&options).await {
        Ok(batman_runtime::lifecycle::StopOutcome::Stopped) => {
            println!("runtime stopped");
            ExitCode::SUCCESS
        }
        Ok(batman_runtime::lifecycle::StopOutcome::NotRunning) => {
            println!("no runtime running for this repository");
            ExitCode::from(1)
        }
        Err(err) => fail(&err),
    }
}

/// Runs `batcave monitor`: connects to the runtime, replays events, and
/// renders them as plain-text lines until interrupted.
async fn run_monitor(
    state_dir: Option<PathBuf>,
    repo: PathBuf,
    run_id: Option<String>,
) -> ExitCode {
    use batman_runtime::lifecycle::{self, MonitorOptions};

    let state_dir = match resolve_state_dir(state_dir) {
        Ok(dir) => dir,
        Err(err) => return fail(&err),
    };

    let options = MonitorOptions {
        state_dir,
        repo,
        run_id,
    };

    match lifecycle::monitor(&options).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => fail(&err),
    }
}

/// Runs `batcave schema`: prints the canonical JSON Schema document.
async fn run_schema() -> ExitCode {
    // Read the schema file from the protocol package.
    let schema_path = std::path::Path::new("packages/protocol-ts/schema/batman.schema.json");
    match std::fs::read_to_string(schema_path) {
        Ok(schema) => {
            print!("{schema}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("failed to read schema file: {err}");
            ExitCode::FAILURE
        }
    }
}

/// Runs `batcave audit export`: exports events to a JSONL file.
async fn run_audit_export(
    state_dir: Option<PathBuf>,
    repo: PathBuf,
    from: Option<String>,
    to: Option<String>,
    output: PathBuf,
) -> ExitCode {
    // A failed resolve must not silently fall back to `.batman`: the
    // export would then report success against a directory that may not
    // exist.
    let state_dir_resolved = match resolve_state_dir(state_dir) {
        Ok(dir) => dir,
        Err(err) => return fail(&err),
    };

    let db = match batman_runtime::db::DatabaseHandle::start(state_dir_resolved.join("runtime.db"))
        .await
    {
        Ok(handle) => handle,
        Err(err) => return fail(&format!("failed to open database: {err}")),
    };

    let mut export = batman_runtime::audit::Export::new(
        repo.to_string_lossy().to_string(),
        state_dir_resolved.to_string_lossy().to_string(),
        output.to_string_lossy().to_string(),
    );
    export.from = from;
    export.to = to;

    match export.export(&db).await {
        Ok(count) => {
            println!("exported {count} events to {}", output.display());
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("export failed: {err}");
            ExitCode::FAILURE
        }
    }
}

/// Resolves the state directory, defaulting to `.batman` if `None`.
fn resolve_state_dir(state_dir: Option<PathBuf>) -> Result<PathBuf, String> {
    match state_dir {
        Some(dir) => Ok(dir),
        None => {
            let default = PathBuf::from(".batman");
            if default.exists() {
                Ok(default)
            } else {
                Err(
                    "state directory `.batman` does not exist; use --state-dir to specify it"
                        .to_string(),
                )
            }
        }
    }
}

/// Runs `batcave doctor`: runs the diagnostic check catalog against the
/// same paths `serve` uses, so it diagnoses the state a daemon actually
/// writes rather than a directory only `doctor` believes in.
async fn run_doctor(
    state_dir: Option<PathBuf>,
    repo: PathBuf,
    json: bool,
    org_config: Option<PathBuf>,
    repo_config: Option<PathBuf>,
    user_config: Option<PathBuf>,
) -> ExitCode {
    use batman_runtime::doctor::Doctor;
    use batman_runtime::paths::RuntimePaths;

    /// Reports a fatal condition in the caller's chosen format. Only
    /// conditions that prevent the catalog from running reach here; an
    /// individual check's failure is part of the result.
    fn abort(json: bool, message: &str) -> ExitCode {
        if json {
            println!(
                "{}",
                serde_json::json!({ "healthy": false, "error": message })
            );
        } else {
            eprintln!("{message}");
        }
        ExitCode::FAILURE
    }

    let state_root = match resolve_state_dir(state_dir) {
        Ok(dir) => dir,
        Err(err) => return abort(json, &err),
    };

    let paths = match RuntimePaths::resolve(&state_root, &repo) {
        Ok(paths) => paths,
        Err(err) => return abort(json, &format!("failed to resolve runtime paths: {err}")),
    };

    let db = match batman_runtime::db::DatabaseHandle::start(paths.database.clone()).await {
        Ok(handle) => Some(std::sync::Arc::new(handle)),
        Err(err) => return abort(json, &format!("failed to open database: {err}")),
    };

    // `--repo` names the repository being diagnosed, never a config file.
    // Config layers are explicit flags, exactly as they are for `serve`.
    let policy = match batman_runtime::config::LayeredConfig::load(
        org_config.as_deref(),
        repo_config.as_deref(),
        user_config.as_deref(),
    ) {
        Ok(config) => match config.merge(None) {
            Ok(policy) => Some(policy),
            Err(err) => return abort(json, &format!("failed to merge config: {err}")),
        },
        Err(err) => return abort(json, &format!("failed to load config: {err}")),
    };

    let doctor = Doctor::new(db, Some(paths.root.clone()), policy).with_runtime_context(
        paths.socket.clone(),
        repo,
        paths.project_id,
    );

    match doctor.check().await {
        Ok(result) => {
            if json {
                println!(
                    "{}",
                    serde_json::to_string(&result).expect("DoctorResult serializes")
                );
            } else {
                println!(
                    "doctor check: {}",
                    if result.healthy { "healthy" } else { "failed" }
                );
                if !result.failed_checks.is_empty() {
                    eprintln!("failed checks:");
                    for check in &result.failed_checks {
                        eprintln!("  - {:?}", check);
                    }
                }
            }
            ExitCode::from(if result.healthy { 0 } else { 1 })
        }
        Err(err) => {
            eprintln!("doctor check failed: {err}");
            ExitCode::FAILURE
        }
    }
}

/// Runs `batcave coordination-mcp`: proxies MCP `initialize`/`tools/list`/
/// `tools/call` on stdio to the worker coordination tools over the
/// runtime socket, authenticated with `BATMAN_WORKER_SCOPE_TOKEN` read
/// from (and removed from) this process's own inherited environment. All
/// protocol/auth behavior lives in `batman_runtime::coordination::mcp`;
/// this function only resolves CLI arguments into that call.
async fn run_coordination_mcp(
    state_dir: Option<PathBuf>,
    repo: PathBuf,
    run_id: String,
) -> ExitCode {
    use batman_protocol::RunId;
    use batman_runtime::coordination::mcp::{self, ProcessEnvironment};

    let state_dir = match resolve_state_dir(state_dir) {
        Ok(dir) => dir,
        Err(err) => return fail(&err),
    };
    let run_id = match RunId::parse(&run_id) {
        Ok(id) => id,
        Err(err) => return fail(&err),
    };

    match mcp::run(&state_dir, &repo, run_id, &ProcessEnvironment).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => fail(&err),
    }
}

/// Runs `batcave display probe`: probes one display backend's status and
/// prints it as JSON or human-readable text. Never activates the backend;
/// this only reads availability/version, exactly like `DisplayBackendTrait::status`.
async fn run_display_probe(backend: String, json: bool) -> ExitCode {
    use batman_protocol::{DisplayBackend as ProtoBackend, DisplayConfig};
    use batman_runtime::display::{
        DisplayBackendTrait, HerdrDisplay, TerminalDisplay, TmuxDisplay,
    };

    let display: Box<dyn DisplayBackendTrait> = match backend.as_str() {
        "herdr" => Box::new(HerdrDisplay::new(DisplayConfig {
            backend: ProtoBackend::Herdr,
            width: None,
            height: None,
        })),
        "tmux" => Box::new(TmuxDisplay::new(DisplayConfig {
            backend: ProtoBackend::Tmux,
            width: None,
            height: None,
        })),
        "terminal" => Box::new(TerminalDisplay::new(DisplayConfig {
            backend: ProtoBackend::Terminal,
            width: None,
            height: None,
        })),
        other => {
            return fail(&format!(
                "unknown display backend `{other}`; expected one of herdr, tmux, or terminal"
            ));
        }
    };

    let status = display.status();
    let version = display.version();

    if json {
        let mut value = serde_json::to_value(&status).expect("DisplayStatus serializes");
        if let Some(obj) = value.as_object_mut() {
            obj.insert("version".to_string(), serde_json::json!(version));
        }
        println!("{value}");
    } else {
        println!("backend: {}", display.backend_name());
        println!("available: {}", status.available);
        println!("active: {}", status.active);
        if let Some(v) = version {
            println!("version: {v}");
        }
        if let Some((w, h)) = status.dimensions {
            println!("dimensions: {w}x{h}");
        }
    }
    ExitCode::SUCCESS
}

/// Runs `batcave conformance`: runs one or all adapters' fixture or live
/// conformance suite and writes the resulting report(s) to `output` as a
/// JSON array, printing the exact same JSON to stdout. Exactly one of
/// `fixture`/`live` must be set. Live mode runs by default; when
/// `BATMAN_DISABLE_VENDOR_CLI=1` is set, the adapter's `live_report()`
/// returns `Err`, which this command reports as a `{adapter,
/// mode:"live", passed:false, error}` entry, never a hard process
/// failure.
async fn run_conformance(adapter: String, fixture: bool, live: bool, output: PathBuf) -> ExitCode {
    use batman_runtime::adapter::AdapterKind;
    use batman_runtime::conformance::{
        ConformanceReport, run_fixture_conformance, run_live_conformance,
    };

    if fixture == live {
        return fail(&format!(
            "conformance requires exactly one of --fixture or --live (got fixture={fixture}, live={live})"
        ));
    }

    let kinds: Vec<AdapterKind> = if adapter == "all" {
        vec![
            AdapterKind::Claude,
            AdapterKind::Codex,
            AdapterKind::Copilot,
            AdapterKind::OmpRpc,
        ]
    } else {
        match AdapterKind::from_wire_name(&adapter) {
            Some(kind) => vec![kind],
            None => {
                return fail(&format!(
                    "unknown adapter `{adapter}`; expected one of claude, codex, copilot, ompRpc, or all"
                ));
            }
        }
    };

    let mut reports: Vec<serde_json::Value> = Vec::with_capacity(kinds.len());
    let mut typed_reports: Vec<ConformanceReport> = Vec::new();
    for kind in kinds {
        if fixture {
            let report = run_fixture_conformance(kind).await;
            typed_reports.push(report);
        } else {
            match run_live_conformance(kind).await {
                Ok(report) => typed_reports.push(report),
                Err(err) => {
                    // Live failures are reported as entries, never a hard
                    // process failure.
                    reports.push(serde_json::json!({
                        "adapter": kind.wire_name(),
                        "mode": "live",
                        "passed": false,
                        "error": err,
                    }));
                    continue;
                }
            }
        }
    }
    // Convert typed reports to JSON.
    for report in &typed_reports {
        reports.push(serde_json::to_value(report).expect("ConformanceReport serializes"));
    }

    let rendered = serde_json::to_string_pretty(&reports).expect("reports serialize");
    if let Err(err) = std::fs::write(&output, &rendered) {
        return fail(&format!("failed to write {}: {err}", output.display()));
    }
    println!("{rendered}");

    // In fixture mode, gate against the committed baseline.
    if fixture {
        let baseline_path = PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/conformance/fixture-mode-baseline.json"
        ));
        let baseline_text = match std::fs::read_to_string(&baseline_path) {
            Ok(t) => t,
            Err(err) => {
                return fail(&format!(
                    "cannot read fixture-mode baseline {}: {err}",
                    baseline_path.display()
                ));
            }
        };
        let baseline: FixtureBaseline = match serde_json::from_str(&baseline_text) {
            Ok(b) => b,
            Err(err) => {
                return fail(&format!(
                    "fixture-mode baseline {} is malformed: {err}",
                    baseline_path.display()
                ));
            }
        };
        for report in &typed_reports {
            let adapter = &report.adapter;
            let expected_failures: Vec<String> = baseline
                .expected_failures
                .get(adapter.as_str())
                .cloned()
                .unwrap_or_default();
            let actual_failures: Vec<String> = report
                .scenarios
                .iter()
                .filter(|s| !s.passed)
                .map(|s| s.name.to_string())
                .collect();
            // Check for unexpected failures (not in baseline).
            let unexpected: Vec<&String> = actual_failures
                .iter()
                .filter(|name| !expected_failures.contains(name))
                .collect();
            if !unexpected.is_empty() {
                return fail(&format!(
                    "adapter {} failed scenario(s) not in the fixture-mode baseline: {}",
                    adapter,
                    unexpected
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            // Check for baseline entries that now pass (rotting baseline).
            let now_passing: Vec<&String> = expected_failures
                .iter()
                .filter(|name| !actual_failures.contains(name))
                .collect();
            if !now_passing.is_empty() {
                return fail(&format!(
                    "adapter {} baseline scenario(s) now pass — remove from baseline: {}",
                    adapter,
                    now_passing
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
        }
    }

    ExitCode::SUCCESS
}

/// Runs `batcave adapters`: runs every reserved adapter kind's fixture
/// conformance suite and prints the resulting reports (the only source of
/// truth for OMP-facing effective capabilities) as JSON or human-readable
/// text.
async fn run_adapters(json: bool) -> ExitCode {
    use batman_runtime::adapter::AdapterKind;
    use batman_runtime::conformance::run_fixture_conformance;

    let kinds = [
        AdapterKind::Claude,
        AdapterKind::Codex,
        AdapterKind::Copilot,
        AdapterKind::OmpRpc,
    ];
    let mut reports = Vec::with_capacity(kinds.len());
    for kind in kinds {
        reports.push(run_fixture_conformance(kind).await);
    }

    if json {
        println!(
            "{}",
            serde_json::to_string(&reports).expect("reports serialize")
        );
    } else {
        for report in &reports {
            println!(
                "{}: mode={:?} passed={} scenarios={}",
                report.adapter,
                report.mode,
                report.passed,
                report.scenarios.len()
            );
        }
    }
    ExitCode::SUCCESS
}

/// Runs `batcave capture`: re-records adapter fixtures from a real
/// vendor CLI, scrubs them deterministically, and overwrites the
/// committed files in place.
async fn run_capture(adapter: String, fixture: Option<String>, dry_run: bool) -> ExitCode {
    use batman_runtime::adapter::AdapterKind;
    use batman_runtime::conformance::capture;

    if adapter == "all" {
        return fail(
            &"capture requires a single adapter; \
        \"all\" would spend a real turn on every vendor CLI",
        );
    }

    let kind = match AdapterKind::from_wire_name(&adapter) {
        Some(kind) => kind,
        None => {
            return fail(&format!(
                "unknown adapter `{adapter}`; expected one of \
                claude, codex, copilot, or ompRpc"
            ));
        }
    };

    let only = fixture.as_deref();

    match capture::capture_adapter(kind, only, dry_run).await {
        Ok(outcome) => {
            for cf in &outcome.written {
                let status = if cf.unchanged {
                    "unchanged"
                } else {
                    "rewritten"
                };
                println!("{}: {} frames ({})", cf.fixture, cf.frames, status);
            }
            if let Some(report) = &outcome.report {
                println!(
                    "{}: mode={:?} passed={} scenarios={}",
                    report.adapter,
                    report.mode,
                    report.passed,
                    report.scenarios.len()
                );
                if !report.passed {
                    return ExitCode::FAILURE;
                }
            }
            ExitCode::SUCCESS
        }
        Err(err) => fail(&err),
    }
}

/// Prints an error to stderr and returns `ExitCode::FAILURE`.
fn fail(err: &dyn std::fmt::Display) -> ExitCode {
    eprintln!("{err}");
    ExitCode::FAILURE
}
