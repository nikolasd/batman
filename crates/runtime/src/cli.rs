//! The `batcave` command-line interface: `serve`, `status`, `stop`,
//! `version`, `schema`, `monitor`, `display`, and `audit`. This layer only
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
    /// Audit commands for managing event retention and export.
    Audit {
        #[command(subcommand)]
        command: AuditCommand,
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
        Command::Serve { .. } | Command::Status { .. } | Command::Stop { .. } => {
            // TODO: Implement these commands (currently broken in original CLI)
            eprintln!("command not yet implemented");
            ExitCode::FAILURE
        }
        Command::Version => {
            println!("batcave {VERSION}");
            ExitCode::SUCCESS
        }
        Command::Schema => {
            // TODO: Implement schema command
            eprintln!("schema command not yet implemented");
            ExitCode::FAILURE
        }
        Command::Audit { command: AuditCommand::Export { .. } } => {
            // TODO: Implement audit export command
            eprintln!("audit export command not yet implemented");
            ExitCode::FAILURE
        }
    }
}
