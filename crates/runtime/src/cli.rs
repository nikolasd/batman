//! The `batcave` command-line interface: `serve`, `status`, `stop`,
//! `version`, and `schema`. This layer only parses arguments, resolves the
//! state root when `--state-dir` is omitted, and maps
//! [`crate::lifecycle`] outcomes to process exit codes; all behaviour lives
//! in the library.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::ExitCode;

use batman_protocol::BinarySource;
use clap::{Parser, Subcommand};

use batman_runtime::VERSION;
use batman_runtime::lifecycle::{
    self, ServeError, ServeOptions, StatusOptions, StopOptions, StopOutcome,
};
use batman_runtime::security::StateRoot;

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
    /// Print the runtime version.
    Version,
    /// Print the canonical JSON Schema document to stdout.
    Schema,
    /// Serve the coordination MCP tools over stdio for one supervised run.
    CoordinationMcp {
        /// The exact BATMAN state root the launching adapter resolved --
        /// required, never defaulted: this subprocess must reach its
        /// launching runtime's own socket, not whatever ambient state
        /// root this process's own environment happens to resolve to.
        #[arg(long)]
        state_dir: PathBuf,
        /// The repository this run belongs to.
        #[arg(long)]
        repo: PathBuf,
        /// The run this MCP server is scoped to.
        #[arg(long)]
        run_id: String,
    },
    /// Reports probe facts and conformance-effective capabilities for
    /// every worker adapter kind (`claude`, `codex`, `copilot`,
    /// `ompRpc`), never a raw declared claim.
    Adapters {
        /// Always emits JSON; accepted for the plan's own documented
        /// invocation shape (`batcave adapters --json`).
        #[arg(long)]
        json: bool,
    },
    /// Runs one adapter's (or every adapter's, with `--adapter all`)
    /// fixture or live conformance suite and writes a machine-readable
    /// report to `--output`.
    Conformance {
        /// `claude`, `codex`, `copilot`, `ompRpc`, or `all`.
        #[arg(long)]
        adapter: String,
        /// Runs the zero-model-call fixture suite. Mutually exclusive
        /// with `--live`; exactly one is required.
        #[arg(long)]
        fixture: bool,
        /// Runs the live suite against the installed vendor CLI.
        /// Mutually exclusive with `--fixture`; exactly one is required.
        /// Each adapter's own suite still checks its own
        /// `BATMAN_LIVE_<ADAPTER>` gate internally and reports (never
        /// hard-fails the whole command for) an unset gate.
        #[arg(long)]
        live: bool,
        /// Where to write the JSON report array.
        #[arg(long)]
        output: PathBuf,
    },
}

/// The canonical protocol JSON Schema, embedded at compile time so the binary
/// is self-contained. Byte-identical to what `xtask generate` commits.
const SCHEMA: &str = include_str!("../../../packages/protocol-ts/schema/batman.schema.json");

/// Parses arguments and runs the selected command, returning a process exit
/// code (73 when a `serve` loses the single-instance race).
pub async fn run() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Serve {
            state_dir,
            repo,
            idle_seconds,
            foreground,
        } => run_serve(state_dir, repo, idle_seconds, foreground).await,
        Command::Status {
            wait_seconds,
            state_dir,
            repo,
        } => run_status(wait_seconds, state_dir, repo).await,
        Command::Stop { state_dir, repo } => run_stop(state_dir, repo).await,
        Command::Version => {
            println!("batcave {VERSION}");
            ExitCode::SUCCESS
        }
        Command::Schema => {
            print!("{SCHEMA}");
            ExitCode::SUCCESS
        }
        Command::CoordinationMcp {
            state_dir,
            repo,
            run_id,
        } => run_coordination_mcp(state_dir, repo, run_id).await,
        Command::Adapters { json: _ } => run_adapters().await,
        Command::Conformance {
            adapter,
            fixture,
            live,
            output,
        } => run_conformance(adapter, fixture, live, output).await,
    }
}

async fn run_serve(
    state_dir: Option<PathBuf>,
    repo: PathBuf,
    idle_seconds: Option<u64>,
    foreground: bool,
) -> ExitCode {
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
    };

    match lifecycle::serve(&options).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(ServeError::AlreadyRunning(already)) => {
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

async fn run_status(
    wait_seconds: Option<u64>,
    state_dir: Option<PathBuf>,
    repo: PathBuf,
) -> ExitCode {
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
                serde_json::to_string_pretty(&value).expect("status value serializes")
            );
            ExitCode::SUCCESS
        }
        Err(err) => fail(&err),
    }
}

async fn run_stop(state_dir: Option<PathBuf>, repo: PathBuf) -> ExitCode {
    let state_dir = match resolve_state_dir(state_dir) {
        Ok(dir) => dir,
        Err(err) => return fail(&err),
    };

    let options = StopOptions { state_dir, repo };

    match lifecycle::stop(&options).await {
        Ok(StopOutcome::Stopped) => {
            println!("{}", serde_json::json!({ "stopped": true }));
            ExitCode::SUCCESS
        }
        Ok(StopOutcome::NotRunning) => {
            println!(
                "{}",
                serde_json::json!({ "stopped": false, "reason": "not_running" })
            );
            ExitCode::SUCCESS
        }
        Err(err) => fail(&err),
    }
}

async fn run_coordination_mcp(state_dir: PathBuf, repo: PathBuf, run_id: String) -> ExitCode {
    let run_id = match batman_protocol::RunId::parse(&run_id) {
        Ok(run_id) => run_id,
        Err(err) => return fail(format!("--run-id {run_id:?} is not a valid run id: {err}")),
    };
    match batman_runtime::coordination::mcp::run(
        &state_dir,
        &repo,
        run_id,
        &batman_runtime::coordination::mcp::ProcessEnvironment,
    )
    .await
    {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => fail(&err),
    }
}

const ALL_ADAPTER_KINDS: [batman_runtime::adapter::AdapterKind; 4] = [
    batman_runtime::adapter::AdapterKind::Claude,
    batman_runtime::adapter::AdapterKind::Codex,
    batman_runtime::adapter::AdapterKind::Copilot,
    batman_runtime::adapter::AdapterKind::OmpRpc,
];

async fn run_adapters() -> ExitCode {
    let mut reports = Vec::with_capacity(ALL_ADAPTER_KINDS.len());
    for kind in ALL_ADAPTER_KINDS {
        reports.push(batman_runtime::conformance::run_fixture_conformance(kind).await);
    }
    match serde_json::to_string_pretty(&reports) {
        Ok(json) => {
            println!("{json}");
            ExitCode::SUCCESS
        }
        Err(err) => fail(err),
    }
}

/// Parses `--adapter`'s raw value into the specific kinds to run:
/// `"all"` means every reserved kind; anything else must be one of
/// `AdapterKind::from_wire_name`'s exact wire names.
fn parse_adapter_selection(
    adapter: &str,
) -> Result<Vec<batman_runtime::adapter::AdapterKind>, String> {
    if adapter == "all" {
        return Ok(ALL_ADAPTER_KINDS.to_vec());
    }
    batman_runtime::adapter::AdapterKind::from_wire_name(adapter)
        .map(|kind| vec![kind])
        .ok_or_else(|| {
            format!(
                "unknown --adapter {adapter:?}; expected one of claude, codex, copilot, ompRpc, or all"
            )
        })
}

async fn run_conformance(adapter: String, fixture: bool, live: bool, output: PathBuf) -> ExitCode {
    if fixture == live {
        return fail(if fixture {
            "exactly one of --fixture or --live is required, not both"
        } else {
            "exactly one of --fixture or --live is required"
        });
    }
    let kinds = match parse_adapter_selection(&adapter) {
        Ok(kinds) => kinds,
        Err(err) => return fail(err),
    };

    let mut reports = Vec::with_capacity(kinds.len());
    for kind in kinds {
        let report = if fixture {
            serde_json::to_value(batman_runtime::conformance::run_fixture_conformance(kind).await)
        } else {
            match batman_runtime::conformance::run_live_conformance(kind).await {
                Ok(report) => serde_json::to_value(report),
                Err(err) => Ok(serde_json::json!({
                    "adapter": kind.wire_name(),
                    "mode": "live",
                    "passed": false,
                    "error": err,
                })),
            }
        };
        match report {
            Ok(value) => reports.push(value),
            Err(err) => return fail(err),
        }
    }

    let json = match serde_json::to_string_pretty(&reports) {
        Ok(json) => json,
        Err(err) => return fail(err),
    };
    if let Err(err) = std::fs::write(&output, &json) {
        return fail(format!("failed to write {}: {err}", output.display()));
    }
    println!("{json}");
    ExitCode::SUCCESS
}

/// Resolves the state directory: the explicit `--state-dir`, or
/// [`StateRoot::resolve`] from the real environment and home.
fn resolve_state_dir(explicit: Option<PathBuf>) -> anyhow::Result<PathBuf> {
    if let Some(dir) = explicit {
        return Ok(dir);
    }
    let env: HashMap<String, String> = std::env::vars().collect();
    let home = std::env::var("HOME")
        .map(PathBuf::from)
        .map_err(|_| anyhow::anyhow!("HOME is not set; pass --state-dir explicitly"))?;
    let root = StateRoot::resolve(&env, &home)?;
    Ok(root.path().to_path_buf())
}

/// Reports the running binary's origin from the `BATMAN_BINARY_SOURCE` env var
/// set by the launcher. Never logs any override path.
fn binary_source_from_env() -> BinarySource {
    match std::env::var("BATMAN_BINARY_SOURCE").as_deref() {
        Ok("override") => BinarySource::Override,
        Ok("package") => BinarySource::Package,
        _ => BinarySource::Unknown,
    }
}

/// Prints an error to stderr and returns a failure exit code.
fn fail(err: impl std::fmt::Display) -> ExitCode {
    eprintln!("batcave: {err}");
    ExitCode::FAILURE
}
