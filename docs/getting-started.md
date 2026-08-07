# BATMAN Getting Started Guide

This guide covers everything you need to **build BATMAN from source as a contributor** — from setup to troubleshooting. It covers all M4 hardening features including configuration, security, recovery, doctor, and release management.

> **Just want to use BATMAN, not build it?** See [README.md's Installation section](../README.md#installation) — `omp install @nikolasd/batman` installs both the extension and the runtime with no build step. This guide is for developing BATMAN itself.

## Prerequisites

Before you begin, ensure you have the following installed:

- **Rust 1.97.1+** (pinned by `rust-toolchain.toml`)
  - Recommended: install via [rustup](https://rustup.rs) — automatically respects the pinned version
  - Alternative: `brew install rust` (no automatic version pinning; verify with `rustc --version`)
- **Bun 1.3.14+** (pinned by `packageManager` in `package.json`)
  - Install via Homebrew: `brew install oven-sh/bun/bun`

## Installation

### Clone the Repository

```bash
git clone git@github.com:nikolasd/batman.git
cd batman
```

### Build

```bash
# Install JS deps and build the batcave runtime in one step
bun run setup

# Bundle the OMP extension (required before manual testing loads dist/index.js)
bun run build
```

## Configuration

### Configuration Layers

BATMAN uses a layered configuration system with strict precedence (highest wins):

1. **Org config** (lowest precedence)
2. **Repo config**
3. **User config** (highest static precedence)
4. **Per-run params** (overrides everything)

Configuration files are YAML with strict unknown-key rejection (fails closed with line/column diagnostics).

### Configuration File Locations

There is no auto-discovery: each layer is loaded only from the path you pass explicitly via
`--org-config`, `--repo-config`, or `--user-config` (to `serve` or `doctor`) — a layer you don't
pass is simply absent from the merge, and there's no `BATMAN_ORG_CONFIG`-style environment variable
either. `<repo>/.batman/config.yaml` and `~/.batman/config.yaml` below are conventional locations to
point those flags at, not defaults BATMAN looks for on its own.

### Configuration File Example

```yaml
# ~/.batman/config.yaml
max_workers: 4
concurrency_ceiling: 8
retention: "30d"
display:
  backend: auto
models:
  allowed:
    - "gpt-4"
    - "claude-3-opus"
security:
  patterns:
    - "AKIA[0-9A-Za-z]{16}"  # AWS access key pattern
    - "sk-[a-zA-Z0-9]{32}"  # API key pattern
rollout_gates:
  vendor_terms_accepted: true
  retention_configured: true
  model_allowlist_set: true
  concurrency_explicit: true
  native_discovery_reviewed: true
  ornith_identity_set: true
```

### RuntimePolicy

The merged configuration produces an immutable [`RuntimePolicy`] with a SHA-256 fingerprint:

```rust
pub struct RuntimePolicy {
    pub merged: serde_json::Value,
    pub fingerprint: String,
    pub display_backend: String,
    pub retention: String,
    pub max_workers: u32,
    pub concurrency_ceiling: u32,
    pub allowed_models: Vec<String>,
    pub org_security_patterns: Vec<String>,
    pub rollout_gates: RolloutGates,
}
```

### RolloutGates

Production-blocking gates that must be resolved:

```rust
pub struct RolloutGates {
    pub vendor_terms_accepted: bool,
    pub retention_configured: bool,
    pub model_allowlist_set: bool,
    pub concurrency_explicit: bool,
    pub native_discovery_reviewed: bool,
    pub ornith_identity_set: bool,
}
```

All gates must be `true` before production use.

## Usage

### Start the Server

`--repo` is required; `--state-dir` should point at the same state root the OMP extension would
resolve for you (see [`cli-reference.md`](cli-reference.md#before-you-start-state-directories) —
omitting it falls back to a bare `.batman` in the current directory, which is *not* that location):

```bash
batcave serve --repo "$PWD" --state-dir "$HOME/.omp/orchestrator"
```

With explicit configuration files:

```bash
batcave serve --repo "$PWD" --state-dir "$HOME/.omp/orchestrator" \
  --org-config /etc/batman/org.yaml \
  --repo-config .batman/config.yaml \
  --user-config ~/.batman/config.yaml
```

### Run Status Check with Doctor

`batcave status` requires a live runtime — it queries `runtime/status` over the socket:

```bash
batcave status --repo "$PWD" --state-dir "$HOME/.omp/orchestrator"
```

For diagnostics that don't require a live runtime, use `doctor` instead — it runs the full check
catalog (database connectivity, state directory permissions, platform support, schema
compatibility, adapter availability, disk space, stale runs/workspaces, rollout gates, and more —
see [`cli-reference.md`](cli-reference.md#batcave-doctor) for the complete list):

```bash
batcave doctor --repo "$PWD" --state-dir "$HOME/.omp/orchestrator" --json
```

**Note:** there is no `--recover` flag on `status` or `doctor`. Crash recovery is not something you
trigger manually — it runs automatically, once, every time `serve` starts, before the socket
accepts any connection (see [Crash Recovery](#crash-recovery) below). `doctor`'s `stale_runs` check
reports the same stuck-run condition without acting on it, if you want to confirm one exists before
the next restart sweeps it.

### Stop the Server

```bash
batcave stop --repo "$PWD" --state-dir "$HOME/.omp/orchestrator"
```

### Audit Export

Export audit events to JSONL format. Note that `--state-dir` here must be the repository's actual
runtime directory (what `serve`/`status`/`doctor` would derive internally from a state root plus
`--repo`), not a top-level state root — `audit export` opens `<state-dir>/runtime.db` directly and
does not derive it from `--repo` itself (see [`cli-reference.md`](cli-reference.md#batcave-audit-export)):

```bash
batcave audit export --repo "$PWD" --state-dir "$HOME/.omp/orchestrator/repos/<repository-id>" --output /tmp/audit.jsonl
```

## Security Features

### Redaction

BATMAN enforces a strict redaction boundary: raw vendor content (which may contain `Thinking` or `Secret` fragments) is sanitized before persistence. The [`Redactor`] is the sole path from raw content to [`PersistableEvent`]:

- Drops `Thinking` and `Secret` fragments entirely
- Rewrites built-in regex-pattern matches (e.g., API keys) with `[REDACTED:<rule id>]` markers
- [`PersistableEvent`] fields are private with no public constructor

```rust
let redactor = Redactor::new();
let sanitized = redactor.sanitize(raw_event)?;
```

### Org-Configured Redaction Rules

Organizations can define custom redaction patterns in their config:

```yaml
security:
  patterns:
    - "AKIA[0-9A-Za-z]{16}"  # AWS access key
    - "sk-[a-zA-Z0-9]{32}"  # API key
    - "ghp_[a-zA-Z0-9]{36}"  # GitHub personal access token
```

These are compiled once at startup and applied to every redaction call.

### File Security

BATMAN ensures all on-disk state is private (mode `0700`/`0600`, owned by current user) before writing:

```rust
// Ensures directory is mode 0700 and owned by current user
ensure_private_dir(&state_root)?;

// Ensures file is mode 0600 and owned by current user
ensure_private_file(&lock_file)?;
```

### Event Retention

Configure event retention period:

```yaml
retention: "30d"  # 30 days
# or
retention: "90d"  # 90 days
```

Events older than the retention period are automatically purged.

### Export

Export audit events to JSONL format for offline analysis — see the note above about `--state-dir`
meaning the repository's runtime directory here, not a state root:

```bash
batcave audit export --repo "$PWD" --state-dir "$HOME/.omp/orchestrator/repos/<repository-id>" --output /tmp/audit.jsonl
```

## Crash Recovery

### RecoveryCoordinator

`RecoveryCoordinator` is wired into `lifecycle::serve()` and runs automatically at daemon startup. It scans for runs stuck in non-terminal states past a configurable `stuck_threshold` (default: 5 minutes) and transitions them to terminal states. 13 kill-point tests verify the recovery matrix.

**References:** `crates/runtime/src/recovery.rs`, `crates/runtime/src/lifecycle.rs`

### Manual Recovery

There's no flag to trigger recovery on demand — it only ever runs automatically, once, inside
`batcave serve` at startup, before the socket accepts any connection. To confirm a run is actually
stuck without waiting for (or forcing) a restart, check `doctor`'s `stale_runs` check, which reports
the same condition read-only:

```bash
batcave doctor --repo "$PWD" --state-dir "$HOME/.omp/orchestrator" --json
```

### Recovery Configuration

```rust
pub struct RecoveryConfig {
    pub stuck_threshold: Duration,  // Default: 5 minutes
    pub recover_paused: bool,       // Default: false
    pub recover_waiting: bool,      // Default: false
}
```

## Doctor (Health Checks)

The [`Doctor`] provides comprehensive health checking:

```rust
let doctor = Doctor::new(Some(db), Some(state_dir), Some(policy))
    .with_runtime_context(socket_path, repo, project_id);
let result = doctor.check().await?;

if result.healthy {
    println!("Runtime is healthy");
} else {
    println!("Failed checks: {:?}", result.failed_checks);
}
```

### Health Checks

The catalog runs 13 checks in total — database connectivity, configuration validity, state
directory permissions, platform support, binary integrity, socket permissions, schema
compatibility, per-adapter availability (Claude/Codex/Copilot/OMP-RPC), display backend
availability, disk space, stale workspaces, stale runs, and rollout gate resolution. See
[`cli-reference.md`](cli-reference.md#batcave-doctor) for what each one actually verifies — it's
the single source of truth for the catalog, kept alongside the CLI flags it's exposed through.

### DoctorResult

```rust
pub struct DoctorResult {
    pub healthy: bool,
    pub passed_checks: Vec<String>,
    pub failed_checks: Vec<FailedCheck>,
    pub unresolved_gates: Vec<String>,
}
```

## Testing

### Run All Tests

```bash
cargo test
```

### Run Specific Test Suite

```bash
cargo test --test adapter_contract
cargo test --test adapter_registry
cargo test --test approval
cargo test --test audit
cargo test --test claude_adapter
cargo test --test claude_live -- --ignored   # #[ignore]d; real model call; skips if BATMAN_DISABLE_VENDOR_CLI=1
cargo test --test codex_adapter
cargo test --test config
cargo test --test conformance
cargo test --test coordination
cargo test --test coordination_mcp
cargo test --test copilot_adapter
cargo test --test database
cargo test --test display_registry
cargo test --test display_selector
cargo test --test doctor
cargo test --test domain_repository
cargo test --test herdr_display
cargo test --test ipc
cargo test --test lifecycle
cargo test --test monitor_cli
cargo test --test omp_rpc_adapter
cargo test --test orchestration_rpc
cargo test --test paths
cargo test --test recovery
cargo test --test redaction
cargo test --test redaction_boundary
cargo test --test supervisor
cargo test --test terminal_adapter
cargo test --test tmux_display
cargo test --test vendor_cli_availability
cargo test --test workspace_apply
cargo test --test workspace_lease
cargo test --test workspace_materialize
```

### Test Coverage

The test suite includes 34 Rust integration test files (`crates/runtime/tests/`) covering:
- Adapter contract and registry
- Approval workflows
- Audit and redaction
- All four worker adapters (Claude, Codex, Copilot, OMP-RPC)
- Configuration and merging
- Conformance testing
- Coordination and MCP integration
- Database operations
- Display registry and selection
- Doctor diagnostics and crash recovery
- Domain repository
- IPC and lifecycle
- Supervisor and terminal adapters
- Tmux display management
- Vendor CLI availability probing
- Workspace operations (apply, lease, materialize)

## Troubleshooting

### `serve` exits with code 73

There is no TCP port to conflict on — `batcave` communicates over a Unix domain socket, not a
network port, and `serve` has no `--port` flag. Exit code 73 (`EX_TEMPFAIL`) means the repository's
`runtime.lock` is already held by a live `batcave serve` process; it prints that process's identity
(pid, project id, socket path) as JSON on stdout when it happens.

**Solution**:
1. This usually isn't an error to fix — connect to the existing runtime instead of starting another:
   `batcave status --repo <path> --state-dir <same state-dir>`.
2. If you're sure it should have exited (e.g. a previous test run leaked it), find and stop it:
   `ps aux | grep batcave`, then `batcave stop --repo <path> --state-dir <state-dir>` or `kill <pid>`.

### Database Connection Errors

BATMAN has no configurable database URL — the SQLite file is always `<runtime-dir>/runtime.db`,
where `<runtime-dir>` is `<state-dir>/repos/<repository-id>/`, derived automatically from
`--state-dir` and `--repo`. A "failed to open database" error almost always means that directory
doesn't exist or isn't writable.

**Solution**:
1. Confirm the state dir resolves the way you expect (see
   [`cli-reference.md`](cli-reference.md#before-you-start-state-directories)).
2. Ensure it exists and is writable — `batcave doctor --json` reports this directly as the
   `state_dir_writable` and `database_connectivity` checks.

### Rollout Gates Unresolved

If `doctor` reports unresolved rollout gates, `native_discovery_reviewed` specifically blocks
authorization outright; the rest are advisory.

**Solution**:
1. Review your configuration files.
2. Ensure the relevant `rollout_gates` fields are set to `true` in whichever config layer you
   control.
3. Check current status: `batcave doctor --repo <path> --state-dir <state-dir> --json` — the
   `unresolved_gates` array and the per-gate `rollout_gate_<gate>` checks name exactly what's left.

### Permission Errors

**Solution**:
1. Check file permissions: `ls -ld <state-dir>` — BATMAN expects its state directory at mode
   `0700` and its socket at `0600`, both owned by the user running `batcave`.
2. **Do not widen permissions** (e.g. `chmod 755`) to work around a permission error — that defeats
   the same-user isolation the redaction/security boundary depends on, and `doctor`'s
   `state_dir_writable`/`socket_permissions` checks will simply fail again with a different reason.
   If ownership is wrong, fix ownership (`chown`) or remove and let `batcave` recreate the directory
   at the correct mode.

### Recovery Issues

Recovery isn't independently configurable from the CLI — `stuck_threshold`,
`recover_paused`, and `recover_waiting` are constructor parameters on `RecoveryConfig`, used by
whatever code calls `RecoveryCoordinator` (currently just `lifecycle::serve()`'s own defaults). If a
run looks like it should have been recovered and wasn't:

**Solution**:
1. Confirm it's actually eligible: recovery only touches `queued`/`starting`/`working` runs whose
   last event is older than the stuck threshold (5 minutes by default) — `paused`/`waitingUser`/
   `waitingPeer` runs are left alone unless recovery was built with `recover_paused`/
   `recover_waiting` enabled.
2. Recovery only runs at `serve` startup, not continuously — a run that just became stale won't be
   recovered until the next restart.
3. Use `batcave doctor --json`'s `stale_runs` check to confirm a run is actually stuck, independent
   of whether recovery has swept it yet.

## Contributing

We welcome contributions! Please see the [CONTRIBUTING.md](../CONTRIBUTING.md) file for guidelines.

### Development Setup

1. Clone the repository
2. Run `bun run setup` — installs JS deps, builds the batcave runtime
3. Run tests: `bun run check`
4. Make your changes
5. Submit a pull request

### Code Style

- Follow [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/)
- Use `cargo fmt` to format code: `cargo fmt --all`
- Use `cargo clippy` to check for common issues: `cargo clippy --all-targets --all-features -- -D warnings`

## Getting Help

- **Documentation**: See the other files in [`docs/`](.) — start with [architecture.md](architecture.md) and [code-walkthrough.md](code-walkthrough.md). For running `batcave` day-to-day, see [`cli-reference.md`](cli-reference.md) (full flag reference) and [`operations.md`](operations.md) (lifecycle procedures).
- **Issues**: Open a GitHub Issue on this repository

## License

This project is licensed under the [MIT License](../LICENSE). See the LICENSE file for full terms.
