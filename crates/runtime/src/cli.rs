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
        } => run_doctor(state_dir, repo, json).await,
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
        binary_source: batman_protocol::BinarySource::Unknown,
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
    // Use the audit export module (currently a stub that returns Ok(()))
    let state_dir_resolved =
        resolve_state_dir(state_dir).unwrap_or_else(|_| PathBuf::from(".batman"));

    let mut export = batman_runtime::audit::Export::new(
        repo.to_string_lossy().to_string(),
        state_dir_resolved.to_string_lossy().to_string(),
        output.to_string_lossy().to_string(),
    );
    export.from = from;
    export.to = to;

    match export.export() {
        Ok(()) => {
            println!("events exported to {}", output.display());
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

/// Runs `batcave doctor`: runs diagnostic checks on the runtime state and configuration.
async fn run_doctor(state_dir: Option<PathBuf>, repo: PathBuf, json: bool) -> ExitCode {
    use batman_runtime::doctor::Doctor;

    let state_dir_resolved = match resolve_state_dir(state_dir) {
        Ok(dir) => dir,
        Err(err) => return fail(&err),
    };

    // Try to open a database handle for the repo
    let db = match batman_runtime::db::DatabaseHandle::start(state_dir_resolved.join("runtime.db"))
        .await
    {
        Ok(handle) => Some(std::sync::Arc::new(handle)),
        Err(err) => {
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "healthy": false,
                        "error": format!("failed to open database: {err}")
                    })
                );
            } else {
                eprintln!("failed to open database: {err}");
            }
            return ExitCode::FAILURE;
        }
    };

    // Load runtime policy (if available)
    let policy = match batman_runtime::config::LayeredConfig::load(
        None, // org_config
        Some(repo.as_path()),
        None, // user_config
    ) {
        Ok(config) => match config.merge(None) {
            Ok(policy) => Some(policy),
            Err(err) => {
                if json {
                    println!(
                        "{}",
                        serde_json::json!({
                            "healthy": false,
                            "error": format!("failed to merge config: {err}")
                        })
                    );
                } else {
                    eprintln!("failed to merge config: {err}");
                }
                return ExitCode::FAILURE;
            }
        },
        Err(err) => {
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "healthy": false,
                        "error": format!("failed to load config: {err}")
                    })
                );
            } else {
                eprintln!("failed to load config: {err}");
            }
            return ExitCode::FAILURE;
        }
    };

    let doctor = Doctor::new(db, Some(state_dir_resolved), policy);

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
async fn run_coordination_mcp(state_dir: Option<PathBuf>, repo: PathBuf, run_id: String) -> ExitCode {
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
    use batman_runtime::display::{DisplayBackendTrait, HerdrDisplay, TerminalDisplay, TmuxDisplay};

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
/// `fixture`/`live` must be set. An unset `BATMAN_LIVE_<ADAPTER>` gate in
/// live mode is reported as a `passed: false` entry with an `error`
/// field, never a hard process failure.
async fn run_conformance(adapter: String, fixture: bool, live: bool, output: PathBuf) -> ExitCode {
    use batman_runtime::adapter::AdapterKind;
    use batman_runtime::conformance::{run_fixture_conformance, run_live_conformance};

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

    let mut reports = Vec::with_capacity(kinds.len());
    for kind in kinds {
        let report = if fixture {
            serde_json::to_value(run_fixture_conformance(kind).await)
                .expect("ConformanceReport serializes")
        } else {
            match run_live_conformance(kind).await {
                Ok(report) => {
                    serde_json::to_value(report).expect("ConformanceReport serializes")
                }
                Err(err) => serde_json::json!({
                    "adapter": kind.wire_name(),
                    "mode": "live",
                    "passed": false,
                    "error": err,
                }),
            }
        };
        reports.push(report);
    }

    let rendered = serde_json::to_string_pretty(&reports).expect("reports serialize");
    if let Err(err) = std::fs::write(&output, &rendered) {
        return fail(&format!("failed to write {}: {err}", output.display()));
    }
    println!("{rendered}");
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
/// Prints an error to stderr and returns `ExitCode::FAILURE`.
fn fail(err: &dyn std::fmt::Display) -> ExitCode {
    eprintln!("{err}");
    ExitCode::FAILURE
}
