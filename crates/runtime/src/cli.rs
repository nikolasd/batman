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
