# BATMAN Getting Started Guide

This guide covers everything you need to get started with BATMAN, from installation to troubleshooting. It covers all M4 hardening features including configuration, security, recovery, doctor, and release management.

## Prerequisites

Before you begin, ensure you have the following installed:

- **Rust** (version 1.70.0 or later)
  - Install via Homebrew: `brew install rustup` then `rustup-init`
- **Bun** (version 1.0.0 or later)
  - Install via Homebrew: `brew install oven-sh/bun/bun`

## Installation

### Clone the Repository

```bash
git clone https://github.com/your-org/batman.git
cd batman
```

### Install Dependencies

```bash
# Install Rust dependencies
cargo install --path .

# Install TypeScript dependencies
bun install
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

After an unclean shutdown (crash, OOM kill, SIGKILL), runs may be left in non-terminal states. The [`RecoveryCoordinator`] finds stuck runs and transitions them to appropriate terminal states:

- `queued` → `failed`
- `starting` → `failed`
- `working` → `failed`
- `waitingUser` → `cancelled` (if configured)
- `waitingPeer` → `cancelled` (if configured)
- `paused` → `cancelled` (if configured)

### Automatic Recovery

Recovery runs automatically after each `serve` command.

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

…
331:

…
355:

…
441:BATMAN is released under the [MIT License](LICENSE).
## Contributing

We welcome contributions! Please see the [CONTRIBUTING.md](CONTRIBUTING.md) file for guidelines.

### Development Setup

1. Clone the repository
2. Install dependencies: `cargo install --path .`
3. Run tests: `cargo test`
4. Make your changes
5. Submit a pull request

### Code Style

- Follow [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/)
- Use `cargo fmt` to format code: `cargo fmt --all`
- Use `cargo clippy` to check for common issues: `cargo clippy --all-targets --all-features -- -D warnings`

### Running Tests

```bash
# Run all tests
cargo test

# Run specific test
cargo test --test adapter_contract

# Run with specific features
cargo test --features "feature1,feature2"
```

## Getting Help

- **Documentation**: [docs.batman.dev](https://docs.batman.dev)
- **Discord**: [discord.gg/batman](https://discord.gg/batman)
- **GitHub Issues**: [github.com/your-org/batman/issues](https://github.com/your-org/batman/issues)
- **Email**: support@batman.dev

## License

BATMAN is released under the [MIT License](LICENSE).
