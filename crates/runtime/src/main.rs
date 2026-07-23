//! `batcave`: the BATMAN runtime daemon.
//!
//! At foundation scope this exposes a single `serve --foreground` command that
//! resolves the per-repository state paths, opens the durable database, and
//! serves the JSON-RPC socket protocol until it is signalled. Task 7 extends
//! this with background/detached operation, single-instance locking, and the
//! `status`/`stop` lifecycle commands; the command layout below is structured
//! so that work slots in without reshaping the entry point.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "batcave", version, about = "The BATMAN runtime daemon")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Serve the runtime socket protocol.
    Serve {
        /// Run in the foreground, logging to stderr, until signalled. The
        /// only supported mode at foundation scope.
        #[arg(long)]
        foreground: bool,
        /// The BATMAN state root directory.
        #[arg(long)]
        state_dir: PathBuf,
        /// The repository this runtime instance serves.
        #[arg(long)]
        repo: PathBuf,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Serve {
            foreground,
            state_dir,
            repo,
        } => {
            if !foreground {
                anyhow::bail!(
                    "only `serve --foreground` is supported at foundation scope; \
                     background operation arrives in a later task"
                );
            }
            let _ = tracing_subscriber::fmt()
                .with_writer(std::io::stderr)
                .with_env_filter(
                    tracing_subscriber::EnvFilter::try_from_default_env()
                        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
                )
                .try_init();

            batman_runtime::ipc::serve_foreground(&state_dir, &repo).await?;
            Ok(())
        }
    }
}
