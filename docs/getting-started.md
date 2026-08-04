# BATMAN Getting Started Guide

This guide covers everything you need to **build BATMAN from source as a contributor** — from setup to troubleshooting. It covers all M4 hardening features including configuration, security, recovery, doctor, and release management.

> **Just want to use BATMAN, not build it?** See [README.md's Installation section](../README.md#installation) — `omp install @satori/batman` installs both the extension and the runtime with no build step. This guide is for developing BATMAN itself.

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

  - Org config: file path (or path specified by `BATMAN_ORG_CONFIG`)
  - Repo config: `<repo>/.batman/config.yaml`
  - User config: `~/.batman/config.yaml`

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

```bash
batcave serve
```

This starts the BATMAN server with default configuration. To use custom configuration files:

```bash
batcave serve \
  --org-config /etc/batman/org.yaml \
  --repo-config .batman/config.yaml \
  --user-config ~/.batman/config.yaml
```

### Run Status Check with Doctor

```bash
batcave status
```

The `status` command runs a comprehensive health check including:
- Database connectivity
- State directory accessibility
- Rollout gate status
- Configuration validity

To enable crash recovery during status check:

```bash
batcave status --recover
```

### Stop the Server

```bash
batcave stop
```

### Audit Export

Export audit events to JSONL format:

```bash
batcave audit export --state-dir ~/.batman/state --output /tmp/audit.jsonl
```

## Security Features

### Redaction

BATMAN enforces a strict redaction boundary: raw vendor content (which may contain `Thinking` or `Secret` fragments) is sanitized before persistence. The [`Redactor`] is the sole path from raw content to [`PersistableEvent`]:

- Drops `Thinking` and `Secret` fragments entirely
- Rewrites built-in regex-pattern matches (e.g., API keys) with `[REDACTED:<rule id>]` markers
- [`PersistableEvent`] fields are private with no public constructor

```rust
let redactor = Redactor::new(builtin_rules);
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

Export audit events to JSONL format for offline analysis:

```bash
batcave audit export --state-dir ~/.batman/state --output /tmp/audit.jsonl
```

## Crash Recovery

### RecoveryCoordinator

`RecoveryCoordinator` is wired into `lifecycle::serve()` and runs automatically at daemon startup. It scans for runs stuck in non-terminal states past a configurable `stuck_threshold` (default: 5 minutes) and transitions them to terminal states. 13 kill-point tests verify the recovery matrix.

**References:** `crates/runtime/src/recovery.rs`, `crates/runtime/src/lifecycle.rs`

### Manual Recovery

Trigger recovery manually via the `status` command:

```bash
batcave status --recover
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
let doctor = Doctor::new(db, state_dir, policy);
let result = doctor.check().await?;

if result.healthy {
    println!("Runtime is healthy");
} else {
    println!("Failed checks: {:?}", result.failed_checks);
}
```

### Health Checks

1. **Database connectivity** - Verifies database is accessible
2. **State directory accessibility** - Checks state directory exists and is writable
3. **Rollout gate status** - Verifies all gates are resolved
4. **Configuration validity** - Validates configuration is valid

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
cargo test --test claude_live      # gated on BATMAN_LIVE_CLAUDE=1, real model call
cargo test --test codex_adapter
cargo test --test config
cargo test --test conformance
cargo test --test coordination
cargo test --test coordination_mcp
cargo test --test copilot_adapter
cargo test --test database
cargo test --test display_registry
cargo test --test display_selector
cargo test --test domain_repository
cargo test --test herdr_display
cargo test --test ipc
cargo test --test lifecycle
cargo test --test monitor_cli
cargo test --test omp_rpc_adapter
cargo test --test orchestration_rpc
cargo test --test paths
cargo test --test redaction
cargo test --test redaction_boundary
cargo test --test supervisor
cargo test --test terminal_adapter
cargo test --test tmux_display
cargo test --test workspace_apply
cargo test --test workspace_lease
cargo test --test workspace_materialize
```

### Test Coverage

The test suite includes 31 Rust integration test files (`crates/runtime/tests/`) covering:
- Adapter contract and registry
- Approval workflows
- Audit and redaction
- All four worker adapters (Claude, Codex, Copilot, OMP-RPC)
- Configuration and merging
- Conformance testing
- Coordination and MCP integration
- Database operations
- Display registry and selection
- Domain repository
- IPC and lifecycle
- Supervisor and terminal adapters
- Tmux display management
- Workspace operations (apply, lease, materialize)

## Troubleshooting

### Port Already in Use

If you see an error like `Address already in use`, another process is using the configured port.

**Solution**:
1. Check what's using the port: `lsof -i :8080` (macOS/Linux) or `netstat -ano | findstr :8080` (Windows)
2. Kill the process or use a different port: `batcave serve --port 8081`

### Database Connection Errors

If you see database-related errors, ensure the database URL in your configuration is correct and the database file is accessible.

**Solution**:
1. Check the database path in your state directory
2. Ensure the directory exists and is writable
3. Run status check: `batcave status`

### Rollout Gates Unresolved

If the doctor reports unresolved rollout gates, you cannot use the runtime in production.

**Solution**:
1. Review your configuration files
2. Ensure all `rollout_gates` fields are set to `true` in your config
3. Check the doctor output: `batcave status`

### Permission Errors

If you see permission errors, ensure BATMAN has the necessary permissions to access the configured paths.

**Solution**:
1. Check file permissions: `ls -la ~/.batman/`
2. Adjust permissions if necessary: `chmod 755 ~/.batman/`

### Recovery Issues

If recovery is not working as expected, check the recovery configuration:

**Solution**:
1. Verify `stuck_threshold` is set appropriately (default: 5 minutes)
2. Check `recover_paused` and `recover_waiting` settings
3. Review the status output: `batcave status --recover`

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

- **Documentation**: See the other files in [`docs/`](.) — start with [architecture.md](architecture.md) and [code-walkthrough.md](code-walkthrough.md)
- **Issues**: Open a GitHub Issue on this repository

## License

This project is licensed under the [MIT License](../LICENSE). See the LICENSE file for full terms.
