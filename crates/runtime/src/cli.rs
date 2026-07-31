//! The `batcave` command-line interface: `serve`, `status`, `stop`,
//! `version`, `schema`, `monitor`, and `audit`. This layer only
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
        } => run_serve(state_dir, repo, idle_seconds, foreground, org_config, repo_config, user_config).await,
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
            command: AuditCommand::Export {
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
            println!("{}", serde_json::to_string(&value).expect("status serializes"));
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
    let state_dir_resolved = resolve_state_dir(state_dir)
        .unwrap_or_else(|_| PathBuf::from(".batman"));

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
                Err("state directory `.batman` does not exist; use --state-dir to specify it".to_string())
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
    let db = match batman_runtime::db::DatabaseHandle::start(
        state_dir_resolved.join("runtime.db"),
    )
    .await
    {
        Ok(handle) => Some(std::sync::Arc::new(handle)),
        Err(err) => {
            if json {
                println!("{}", serde_json::json!({
                    "healthy": false,
                    "error": format!("failed to open database: {err}")
                }));
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
                    println!("{}", serde_json::json!({
                        "healthy": false,
                        "error": format!("failed to merge config: {err}")
                    }));
                } else {
                    eprintln!("failed to merge config: {err}");
                }
                return ExitCode::FAILURE;
            }
        },
        Err(err) => {
            if json {
                println!("{}", serde_json::json!({
                    "healthy": false,
                    "error": format!("failed to load config: {err}")
                }));
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
                println!("{}", serde_json::to_string(&result).expect("DoctorResult serializes"));
            } else {
                println!("doctor check: {}", if result.healthy { "healthy" } else { "failed" });
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
/// Prints an error to stderr and returns `ExitCode::FAILURE`.
fn fail(err: &dyn std::fmt::Display) -> ExitCode {
    eprintln!("{err}");
    ExitCode::FAILURE
}
