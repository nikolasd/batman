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
    ApprovalId, ArtifactId, BatmanMethod, BinarySource, Classified, ClientAuth, ClientCapabilities,
    ClientInfo, ClientPrincipalSummary, ClientRole, ContentClass, DiagnosticLevel, EventEnvelope,
    EventSource, InitializeParams, InitializeResult, JsonRpcError, JsonRpcErrorResponse,
    JsonRpcNotification, JsonRpcRequest, JsonRpcResponse, MessageId, OperationId, ProjectId,
    ProtocolVersion, RepositoryIdentity, RequestId, RunId, RuntimeCapabilities, RuntimeEvent,
    RuntimeInfo, RuntimeStatus, TaskId, Timestamp, VersionRange, WorkerId,
    DisplayBackend, DisplayConfig, DisplayStatus,
};
use clap::Subcommand;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
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
    /// Assemble a platform leaf package: copy `--binary` into the matching
    /// `packages/batman-<target>/bin/batcave` and emit its `manifest.json`.
    Package {
        /// One of the four supported target triples: `darwin-arm64`,
        /// `darwin-x64`, `linux-arm64-gnu`, `linux-x64-gnu`.
        #[arg(long)]
        target: String,
        /// Path to the built `batcave` binary to package.
        #[arg(long)]
        binary: PathBuf,
    },
    /// Create a git tag and push it to trigger the release CI/CD pipeline.
    /// The release.yml workflow builds binaries for all platforms and publishes
    /// them as GitHub Release assets.
    Publish,
}

/// The target triples the foundation ships prebuilt `batcave` leaves for.
/// Windows and any musl libc are explicitly unsupported.
const SUPPORTED_TARGETS: &[&str] = &[
    "darwin-arm64",
    "darwin-x64",
    "linux-arm64-gnu",
    "linux-x64-gnu",
];

/// The deterministic checksum/provenance payload written to each leaf
/// package's `manifest.json`. Field order here is the JSON key order: serde
/// serializes struct fields in declaration order, so this is stable across
/// runs without needing a `preserve_order` feature. Carries no timestamp or
/// other non-reproducible data, so packaging the same binary twice produces
/// byte-identical output. Unsigned: the release plan signs this payload
/// later.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct LeafManifest {
    name: String,
    version: String,
    target: String,
    sha256: String,
    #[serde(rename = "sizeBytes")]
    size_bytes: u64,
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
    display_backend: DisplayBackend,
    display_config: DisplayConfig,
    display_status: DisplayStatus,
    json_rpc_request: JsonRpcRequest<serde_json::Value>,
    json_rpc_response: JsonRpcResponse<serde_json::Value>,
    json_rpc_error_response: JsonRpcErrorResponse,
    json_rpc_notification: JsonRpcNotification<serde_json::Value>,
    runtime_status: RuntimeStatus,
}

fn main() -> Result<()> {
    let args = <Args as clap::Parser>::parse();
    match args.command {
        Command::Generate { check } => run_generate(check),
        Command::Package { target, binary } => package_leaf(&workspace_root(), &target, &binary),
        Command::Publish => publish(),
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
        BinarySource,
        ClientAuth,
        ClientCapabilities,
        ClientInfo,
        ClientPrincipalSummary,
        ClientRole,
        InitializeParams,
        InitializeResult,
        JsonRpcError,
        JsonRpcErrorResponse,
        JsonRpcNotification<ts_rs::Dummy>,
        JsonRpcRequest<ts_rs::Dummy>,
        JsonRpcResponse<ts_rs::Dummy>,
        RepositoryIdentity,
        RequestId,
        RuntimeCapabilities,
        RuntimeInfo,
        RuntimeStatus,
        ProtocolVersion,
        VersionRange,
        Classified<ts_rs::Dummy>,
        ContentClass,
        DiagnosticLevel,
        EventEnvelope,
        EventSource,
        RuntimeEvent,
        Timestamp,
        DisplayBackend,
        DisplayConfig,
        DisplayStatus,
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

/// This leaf package's `name` field for a given target triple, e.g.
/// `@satori/batman-darwin-arm64` for `darwin-arm64`.
fn leaf_package_name(target: &str) -> String {
    format!("@satori/batman-{target}")
}

/// The leaf package directory for a given target triple, rooted at
/// `packages/batman-<target>` under the workspace root.
fn leaf_package_dir(root: &Path, target: &str) -> PathBuf {
    root.join("packages").join(format!("batman-{target}"))
}

/// Reads the `version` field out of `packages/extension/package.json`; every
/// leaf manifest's `version` must equal it so `resolveBatcave` (the
/// TypeScript loader) can require an exact match before running a packaged
/// binary.
fn read_extension_version(root: &Path) -> Result<String> {
    let package_json_path = root.join("packages/extension/package.json");
    let raw = fs::read_to_string(&package_json_path)
        .with_context(|| format!("reading {}", package_json_path.display()))?;
    let value: serde_json::Value = serde_json::from_str(&raw)
        .with_context(|| format!("parsing {}", package_json_path.display()))?;
    value
        .get("version")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .with_context(|| {
            format!(
                "{} has no string `version` field",
                package_json_path.display()
            )
        })
}

/// Copies `binary` into the leaf package directory matching `target` as
/// `bin/batcave` (mode `0755` on Unix) and writes its deterministic
/// `manifest.json` (SHA-256 + size + target + version provenance).
///
/// `root` is the workspace root, parameterized so this is independently
/// testable against a temporary fixture root rather than the real workspace.
fn package_leaf(root: &Path, target: &str, binary: &Path) -> Result<()> {
    if !SUPPORTED_TARGETS.contains(&target) {
        bail!(
            "unsupported target {target:?}; supported targets are: {}",
            SUPPORTED_TARGETS.join(", ")
        );
    }

    let leaf_dir = leaf_package_dir(root, target);
    if !leaf_dir.is_dir() {
        bail!(
            "leaf package directory does not exist: {} (expected one package.json per supported \
             target under packages/)",
            leaf_dir.display()
        );
    }

    let bin_dir = leaf_dir.join("bin");
    fs::create_dir_all(&bin_dir).with_context(|| format!("creating {}", bin_dir.display()))?;
    let bin_path = bin_dir.join("batcave");

    let bytes =
        fs::read(binary).with_context(|| format!("reading binary at {}", binary.display()))?;
    fs::write(&bin_path, &bytes).with_context(|| format!("writing {}", bin_path.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&bin_path, fs::Permissions::from_mode(0o755))
            .with_context(|| format!("setting permissions on {}", bin_path.display()))?;
    }

    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let sha256 = hex::encode(hasher.finalize());

    let manifest = LeafManifest {
        name: leaf_package_name(target),
        version: read_extension_version(root)?,
        target: target.to_string(),
        sha256,
        size_bytes: bytes.len() as u64,
    };

    let mut manifest_json =
        serde_json::to_string_pretty(&manifest).context("serializing leaf manifest")?;
    manifest_json.push('\n');

    let manifest_path = leaf_dir.join("manifest.json");
    fs::write(&manifest_path, manifest_json.as_bytes())
        .with_context(|| format!("writing {}", manifest_path.display()))?;

    println!(
        "package: wrote {} and {}",
        bin_path.display(),
        manifest_path.display()
    );
    Ok(())
}

/// Creates a git tag for the current extension version and pushes it to origin.
/// This triggers the release.yml CI/CD pipeline which builds binaries for all
/// supported platforms and publishes them as GitHub Release assets.
fn publish() -> Result<()> {
    let root = workspace_root();
    let version = read_extension_version(&root)?;
    let tag = format!("v{version}");

    println!("Publishing release {tag}...");

    // Create the tag
    let status = std::process::Command::new("git")
        .args(["tag", &tag])
        .status()
        .with_context(|| "failed to create git tag")?;

    if !status.success() {
        anyhow::bail!("failed to create git tag {tag}");
    }

    println!("Created tag {tag}");

    // Push the tag to origin
    let status = std::process::Command::new("git")
        .args(["push", "origin", &tag])
        .status()
        .with_context(|| "failed to push tag to origin")?;

    if !status.success() {
        anyhow::bail!("failed to push tag {tag} to origin");
    }

    println!("Pushed tag {tag} to origin. The release CI/CD pipeline will now build and publish binaries.");
    println!("Once complete, users can install via:");
    println!("  curl -fsSL https://raw.githubusercontent.com/nikolasd/batman/main/scripts/install.sh | bash");

    Ok(())
}

#[cfg(test)]
mod package_tests {
    use super::*;

    /// Builds a fixture workspace root with a `packages/extension/package.json`
    /// declaring `version` and an empty `packages/batman-<target>` leaf
    /// directory, mirroring just enough of the real workspace layout for
    /// `package_leaf` to operate on.
    fn fixture_root(version: &str, target: &str) -> tempfile::TempDir {
        let root = tempfile::tempdir().expect("creating fixture workspace root");

        let extension_dir = root.path().join("packages/extension");
        fs::create_dir_all(&extension_dir).expect("creating fixture extension dir");
        fs::write(
            extension_dir.join("package.json"),
            format!(r#"{{"name": "@satori/batman", "version": "{version}"}}"#),
        )
        .expect("writing fixture extension package.json");

        let leaf_dir = root
            .path()
            .join("packages")
            .join(format!("batman-{target}"));
        fs::create_dir_all(&leaf_dir).expect("creating fixture leaf dir");

        root
    }

    #[test]
    fn package_leaf_rejects_unsupported_targets() {
        let root = fixture_root("0.1.0", "darwin-arm64");
        let binary = root.path().join("batcave-built");
        fs::write(&binary, b"binary-bytes").unwrap();

        let err = package_leaf(root.path(), "windows-x64", &binary).unwrap_err();
        assert!(err.to_string().contains("unsupported target"));
    }

    #[test]
    fn package_leaf_writes_binary_and_manifest() {
        let target = "darwin-arm64";
        let root = fixture_root("0.1.0", target);
        let binary = root.path().join("batcave-built");
        let bytes = b"pretend-this-is-a-batcave-binary";
        fs::write(&binary, bytes).unwrap();

        package_leaf(root.path(), target, &binary).expect("package_leaf should succeed");

        let leaf_dir = leaf_package_dir(root.path(), target);
        let bin_path = leaf_dir.join("bin").join("batcave");
        assert_eq!(fs::read(&bin_path).unwrap(), bytes);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&bin_path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o755);
        }

        let manifest_path = leaf_dir.join("manifest.json");
        let manifest_bytes = fs::read(&manifest_path).unwrap();
        assert!(manifest_bytes.ends_with(b"\n"));

        let manifest: LeafManifest = serde_json::from_slice(&manifest_bytes).unwrap();
        assert_eq!(manifest.name, "@satori/batman-darwin-arm64");
        assert_eq!(manifest.version, "0.1.0");
        assert_eq!(manifest.target, target);
        assert_eq!(manifest.size_bytes, bytes.len() as u64);

        let mut hasher = Sha256::new();
        hasher.update(bytes);
        assert_eq!(manifest.sha256, hex::encode(hasher.finalize()));
    }

    #[test]
    fn package_leaf_manifest_is_byte_identical_across_runs() {
        let target = "linux-x64-gnu";
        let root = fixture_root("0.1.0", target);
        let binary = root.path().join("batcave-built");
        fs::write(&binary, b"deterministic-fixture-bytes").unwrap();

        package_leaf(root.path(), target, &binary).unwrap();
        let manifest_path = leaf_package_dir(root.path(), target).join("manifest.json");
        let first = fs::read(&manifest_path).unwrap();

        package_leaf(root.path(), target, &binary).unwrap();
        let second = fs::read(&manifest_path).unwrap();

        assert_eq!(
            first, second,
            "packaging the same binary twice must be byte-identical"
        );
    }
}
