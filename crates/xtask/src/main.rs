//! Build tooling for the `batman` workspace.
//!
//! `cargo run -p batman-xtask -- generate` regenerates the canonical JSON
//! Schema document and TypeScript bindings from `batman-protocol`, the sole
//! source of truth for every BATMAN wire type. `--check` verifies the
//! committed outputs are up to date without modifying them.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use batman_protocol::{
    ApprovalId, ArtifactId, BatmanMethod, Classified, ClientAuth, ClientCapabilities, ClientInfo,
    ClientPrincipalSummary, ClientRole, ContentClass, DiagnosticLevel, EventEnvelope, EventSource,
    InitializeParams, InitializeResult, JsonRpcError, JsonRpcErrorResponse, JsonRpcRequest,
    JsonRpcResponse, MessageId, OperationId, ProjectId, ProtocolVersion, RepositoryIdentity,
    RequestId, RunId, RuntimeCapabilities, RuntimeEvent, RuntimeInfo, TaskId, Timestamp,
    VersionRange, WorkerId,
};
use clap::Subcommand;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(clap::Parser)]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Generate the canonical JSON Schema document and TypeScript bindings
    /// from `batman-protocol`.
    Generate {
        /// Verify the committed outputs are up to date without modifying
        /// them. Exits non-zero if generation would produce different
        /// output.
        #[arg(long)]
        check: bool,
    },
}

/// Root schema document referencing every exported request/result/event
/// type, so that a single `schemars` invocation produces one JSON Schema
/// with everything reachable from the wire protocol in `$defs`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct ProtocolDocument {
    initialize_params: InitializeParams,
    initialize_result: InitializeResult,
    event_envelope: EventEnvelope,
    runtime_event: RuntimeEvent,
    json_rpc_request: JsonRpcRequest<serde_json::Value>,
    json_rpc_response: JsonRpcResponse<serde_json::Value>,
    json_rpc_error_response: JsonRpcErrorResponse,
}

fn main() -> Result<()> {
    let args = <Args as clap::Parser>::parse();
    match args.command {
        Command::Generate { check } => run_generate(check),
    }
}

/// The workspace root, resolved from the location of this crate at compile
/// time so generation behaves the same regardless of the process's current
/// directory.
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crates/xtask is nested two directories below the workspace root")
        .to_path_buf()
}

/// Renders the root `ProtocolDocument` schema as pretty JSON with a trailing
/// newline.
fn render_schema() -> Result<Vec<u8>> {
    let schema = schemars::schema_for!(ProtocolDocument);
    let mut text = serde_json::to_string_pretty(&schema).context("serializing schema to JSON")?;
    text.push('\n');
    Ok(text.into_bytes())
}

/// Exports every `batman-protocol` wire type's TypeScript binding into
/// `dir`, alongside all of its dependencies. Idempotent and order
/// independent: `ts-rs` merges declarations into their target files sorted
/// by type name regardless of call order.
fn export_bindings(dir: &Path) -> Result<()> {
    fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;

    macro_rules! export {
        ($($ty:ty),+ $(,)?) => {
            $(<$ty as TS>::export_all_to(dir).with_context(|| {
                format!("exporting {} bindings to {}", stringify!($ty), dir.display())
            })?;)+
        };
    }

    export!(
        ApprovalId,
        ArtifactId,
        MessageId,
        OperationId,
        ProjectId,
        RunId,
        TaskId,
        WorkerId,
        BatmanMethod,
        ClientAuth,
        ClientCapabilities,
        ClientInfo,
        ClientPrincipalSummary,
        ClientRole,
        InitializeParams,
        InitializeResult,
        JsonRpcError,
        JsonRpcErrorResponse,
        JsonRpcRequest<ts_rs::Dummy>,
        JsonRpcResponse<ts_rs::Dummy>,
        RepositoryIdentity,
        RequestId,
        RuntimeCapabilities,
        RuntimeInfo,
        ProtocolVersion,
        VersionRange,
        Classified<ts_rs::Dummy>,
        ContentClass,
        DiagnosticLevel,
        EventEnvelope,
        EventSource,
        RuntimeEvent,
        Timestamp,
    );

    Ok(())
}

/// Removes every `*.ts` file directly inside `dir`, so that types renamed or
/// removed from `batman-protocol` don't leave stale bindings behind.
/// `dir` is treated as fully owned by the generator.
fn clear_ts_files(dir: &Path) -> Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "ts") {
            fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
        }
    }
    Ok(())
}

/// Returns the sorted set of `*.ts` file names directly inside `dir`.
fn sorted_ts_file_names(dir: &Path) -> Result<Vec<String>> {
    let mut names = BTreeSet::new();
    if dir.exists() {
        for entry in fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "ts") {
                names.insert(entry.file_name().to_string_lossy().into_owned());
            }
        }
    }
    Ok(names.into_iter().collect())
}

/// Byte-compares two files, producing a clear error naming the drifted file.
fn compare_files(fresh: &Path, committed: &Path, what: &str) -> Result<()> {
    let fresh_bytes = fs::read(fresh)
        .with_context(|| format!("reading freshly generated {}", fresh.display()))?;
    let committed_bytes = fs::read(committed).with_context(|| {
        format!(
            "reading committed {what} at {} (has `bun run generate` been run and committed?)",
            committed.display()
        )
    })?;
    if fresh_bytes != committed_bytes {
        bail!(
            "generated output drift detected: committed {what} at {} does not match freshly \
             generated output; run `bun run generate` and commit the result",
            committed.display()
        );
    }
    Ok(())
}

/// Byte-compares every generated `*.ts` file in `fresh_dir` against
/// `committed_dir`, requiring the exact same set of files.
fn compare_dirs(fresh_dir: &Path, committed_dir: &Path) -> Result<()> {
    let fresh_names = sorted_ts_file_names(fresh_dir)?;
    let committed_names = sorted_ts_file_names(committed_dir)?;

    if fresh_names != committed_names {
        bail!(
            "generated output drift detected: committed {} contains {:?}, but generation now \
             produces {:?}; run `bun run generate` and commit the result",
            committed_dir.display(),
            committed_names,
            fresh_names,
        );
    }

    for name in &fresh_names {
        compare_files(
            &fresh_dir.join(name),
            &committed_dir.join(name),
            "TypeScript binding",
        )?;
    }

    Ok(())
}

fn run_generate(check: bool) -> Result<()> {
    let root = workspace_root();
    let schema_path = root.join("packages/protocol-ts/schema/batman.schema.json");
    let generated_dir = root.join("packages/protocol-ts/src/generated");

    let schema_bytes = render_schema()?;

    if check {
        let temp = tempfile::tempdir().context("creating temporary directory for --check")?;

        let temp_schema_path = temp.path().join("batman.schema.json");
        fs::write(&temp_schema_path, &schema_bytes)
            .with_context(|| format!("writing {}", temp_schema_path.display()))?;

        let temp_generated_dir = temp.path().join("generated");
        export_bindings(&temp_generated_dir)?;

        compare_files(&temp_schema_path, &schema_path, "schema")?;
        compare_dirs(&temp_generated_dir, &generated_dir)?;

        println!("generate --check: schema and TypeScript bindings are up to date");
        return Ok(());
    }

    if let Some(parent) = schema_path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    fs::write(&schema_path, &schema_bytes)
        .with_context(|| format!("writing {}", schema_path.display()))?;

    fs::create_dir_all(&generated_dir)
        .with_context(|| format!("creating {}", generated_dir.display()))?;
    clear_ts_files(&generated_dir)?;
    export_bindings(&generated_dir)?;

    println!(
        "generate: wrote {} and TypeScript bindings to {}",
        schema_path.display(),
        generated_dir.display()
    );
    Ok(())
}
