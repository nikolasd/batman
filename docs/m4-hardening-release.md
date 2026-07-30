# BATMAN M4 Hardening & Release Documentation

This document provides comprehensive documentation for all M4 (Hardening & Release) features of the BATMAN runtime.

## Table of Contents

1. [Configuration & Policy](#1-configuration--policy)
2. [Security & Audit](#2-security--audit)
3. [Crash Recovery](#3-crash-recovery)
4. [Doctor & Rollout Gates](#4-doctor--rollout-gates)
5. [Release Artifacts](#5-release-artifacts)
6. [Conformance Gates](#6-conformance-gates)

---

## 1. Configuration & Policy

### Overview

BATMAN resolves its runtime configuration from multiple YAML layers (org → repo → user → per-run params) with strict precedence. The result is an immutable, SHA-256-fingerprinted `RuntimePolicy` snapshot.

### Configuration Layers

Configuration is resolved from multiple YAML documents in the following precedence order (highest wins):

1. **Org-level** (`org.yml`): Organization-wide policies
2. **Repo-level** (`repo.yml`): Repository-specific overrides
3. **User-level** (`user.yml`): User-specific preferences
4. **Per-run params**: Command-line overrides for a single run

### Configuration Fields

Known configuration fields:

- `retention`: Audit retention period (e.g., "30d", "90d")
- `max_workers`: Maximum number of concurrent workers
- `display`: Display backend preference ("auto", "herdr", "tmux")
- `security`: Organization-defined security redaction patterns
- `models`: Allowed model identifiers (empty = use adapter defaults)
- `concurrency`: Concurrency ceiling (maximum concurrent runs)
- `rollout_gates`: Rollout gate configuration
- `locks`: Field-level locks that prevent lower layers from overriding

### RuntimePolicy

The `RuntimePolicy` is an immutable, SHA-256-fingerprinted snapshot containing:

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

Rollout gates control feature rollout and must be resolved before production use:

- `vendor_terms_accepted`: Vendor terms have been accepted
- `retention_configured`: Audit retention is configured
- `model_allowlist_set`: Model allowlist is set
- `concurrency_explicit`: Concurrency is explicitly configured
- `native_discovery_reviewed`: Native discovery has been reviewed
- `ornith_identity_set`: Ornith identity is set

### Usage

```rust
use batman_runtime::config;

let policy = config::resolve_effective_policy(
    Some(&org_path),
    Some(&repo_path),
    Some(&user_path),
    None,
)?;

if policy.is_rollout_blocked() {
    let gates = policy.unresolved_gates();
    eprintln!("Rollout blocked by gates: {:?}", gates);
}
```

---

## 2. Security & Audit

### Overview

BATMAN provides comprehensive security and audit capabilities, including redaction rules, event retention, and event export.

### Redaction Rules

#### Built-in Rules

The `Redactor` applies built-in redaction rules to raw runtime events:

- **API keys**: Matches patterns like `sk-...` (16+ alphanumeric characters)
- **Bearer tokens**: Matches long bearer-ish tokens (20+ characters)
- **Private keys**: Matches PEM-formatted private keys
- **Thinking blocks**: Drops AI thinking/reasoning content

#### Organization-Configured Rules

Organizations can define custom redaction rules via the `security.patterns` array in their org-level YAML configuration:

```yaml
security:
  patterns:
    - "AKIA[0-9A-Z]{16}  # AWS access key ID"
    - "ghp_[0-9a-zA-Z]{36}  # GitHub PAT"
```

Each pattern can include an inline `# comment` that becomes the rule's human-readable identifier.

### Audit Module

The audit module provides two key capabilities:

#### Retention

The `Retention` struct manages event retention:

```rust
pub struct Retention {
    state_dir: PathBuf,
    retention_period: Duration,
}

impl Retention {
    pub fn new(state_dir: PathBuf, retention_period: Duration) -> Self { ... }
    pub async fn prune(&self) -> Result<usize, RetentionError> { ... }
}
```

The `prune()` method removes events older than the retention period, but only when no migration or recovery transaction is active.

#### Export

The `Export` struct exports events to JSONL format:

```rust
pub struct Export {
    state_dir: PathBuf,
    repo: PathBuf,
    from: Option<String>,
    to: Option<String>,
    output: PathBuf,
}

impl Export {
    pub async fn export(&self) -> Result<usize, ExportError> { ... }
}
```

The `export()` method:
- Reads events from the event journal
- Applies redaction rules to all string fields
- Writes events as JSONL (one JSON object per line)
- Supports filtering by timestamp range (`from`/`to`)

### CLI Usage

```bash
# Export events from the last 24 hours
batcave audit export --state-dir /tmp/bat-state --repo /path/to/repo \
  --from "2026-07-29T00:00:00Z" --output events.jsonl
```

---

## 3. Crash Recovery

### Overview

The `RecoveryCoordinator` provides crash recovery for the BATMAN runtime. After an unclean shutdown (crash, OOM kill, SIGKILL), runs may be left in non-terminal states. The recovery coordinator finds these stuck runs and transitions them to appropriate terminal states.

### RecoveryConfig

```rust
pub struct RecoveryConfig {
    pub stuck_threshold: Duration,
    pub recover_paused: bool,
    pub recover_waiting: bool,
}
```

- `stuck_threshold`: Time threshold for considering a run "stuck" (default: 5 minutes)
- `recover_paused`: Whether to recover runs in `paused` state (default: false)
- `recover_waiting`: Whether to recover runs in `waitingUser`/`waitingPeer` state (default: false)

### RecoveryCoordinator

```rust
pub struct RecoveryCoordinator {
    db: Arc<DatabaseHandle>,
    config: RecoveryConfig,
}

impl RecoveryCoordinator {
    pub fn new(db: Arc<DatabaseHandle>, config: RecoveryConfig) -> Self { ... }
    pub fn with_defaults(db: Arc<DatabaseHandle>) -> Self { ... }
    pub async fn recover(&self) -> Result<RecoveryResult, RecoveryError> { ... }
}
```

### Recovery Logic

The recovery coordinator:

1. Finds all runs in non-terminal states (`queued`, `starting`, `working`, `waitingUser`, `waitingPeer`, `paused`)
2. Checks if they've been stuck for longer than `stuck_threshold`
3. Transitions stuck runs to appropriate terminal states:
   - `queued` → `failed`
   - `starting` → `failed`
   - `working` → `failed`
   - `waitingUser` → `cancelled` (if `recover_waiting` is true)
   - `waitingPeer` → `cancelled` (if `recover_waiting` is true)
   - `paused` → `cancelled` (if `recover_paused` is true)

### Usage

```rust
use batman_runtime::recovery::{RecoveryCoordinator, RecoveryConfig};

let db = Arc::new(DatabaseHandle::start(state_db_path).await?);
let config = RecoveryConfig {
    stuck_threshold: Duration::from_secs(300),
    recover_paused: false,
    recover_waiting: false,
};
let coordinator = RecoveryCoordinator::new(db, config);
let result = coordinator.recover().await?;
println!("Recovered {} runs", result.recovered_count);
```

---

## 4. Doctor & Rollout Gates

### Overview

The `Doctor` provides comprehensive health checking for the BATMAN runtime, including database connectivity, state directory accessibility, rollout gate status, and configuration validity.

### Doctor

```rust
pub struct Doctor {
    db: Option<Arc<DatabaseHandle>>,
    state_dir: Option<PathBuf>,
    policy: Option<RuntimePolicy>,
}

impl Doctor {
    pub fn new(db: Option<Arc<DatabaseHandle>>, state_dir: Option<PathBuf>, policy: Option<RuntimePolicy>) -> Self { ... }
    pub fn empty() -> Self { ... }
    pub async fn check(&self) -> Result<DoctorResult, DoctorError> { ... }
}
```

### DoctorResult

```rust
pub struct DoctorResult {
    pub healthy: bool,
    pub passed_checks: Vec<String>,
    pub failed_checks: Vec<FailedCheck>,
    pub unresolved_gates: Vec<String>,
}

pub struct FailedCheck {
    pub check_name: String,
    pub error: String,
}
```

### Checks Performed

The doctor performs the following checks:

1. **Database connectivity**: Verifies the database is accessible
2. **State directory accessibility**: Verifies the state directory exists and is writable
3. **Rollout gate status**: Checks that all rollout gates are resolved
4. **Configuration validity**: Verifies the configuration is valid

### Usage

```rust
use batman_runtime::doctor::Doctor;

let doctor = Doctor::new(Some(db), Some(state_dir), Some(policy));
let result = doctor.check().await?;

if result.healthy {
    println!("Runtime is healthy");
} else {
    eprintln!("Runtime has issues: {:?}", result.failed_checks);
}
```

---

## 5. Release Artifacts

### Overview

BATMAN provides CI workflows and an xtask for building, packaging, and publishing release artifacts.

### CI Workflow

The `.github/workflows/release.yml` workflow:

1. Triggers on version tags (e.g., `v0.1.0`, `v0.1.0-rc1`)
2. Builds for all target platforms (macOS ARM/x86, Linux, Windows)
3. Packages artifacts with proper naming
4. Creates a GitHub Release with all artifacts

### xtask

The `batman-xtask` crate (in `crates/xtask/`) handles codegen and leaf packaging. Release builds are done with standard Cargo:

```bash
# Build for a target (repeat for each target triple)
cargo build --release --target <triple>

# Package a built binary into a leaf package with manifest.json
cargo run -p batman-xtask -- package --target <triple> --binary target/<triple>/release/batcave

# Verify generated bindings/schema are up to date (CI)
cargo run -p batman-xtask -- generate --check
```

### Supported Targets

- `aarch64-apple-darwin` (macOS ARM)
- `x86_64-apple-darwin` (macOS Intel)
- `x86_64-unknown-linux-gnu` (Linux)
- `x86_64-pc-windows-msvc` (Windows)

### Artifact Naming

Artifacts are named as: `batcave-<target>.<ext>`

Examples:
- `batcave-aarch64-apple-darwin`
- `batcave-x86_64-unknown-linux-gnu`
- `batcave-x86_64-pc-windows-msvc.exe`

---

## 6. Conformance Gates

### Overview

The conformance gates module provides a TypeScript runner for conformance tests, verifying that the runtime behaves correctly across all supported adapter kinds.

### ConformanceConfig

```typescript
export interface ConformanceConfig {
  adapter: AdapterKind;
  mode: ConformanceMode;
  outputFile?: string;
  stateDir: string;
  repo: string;
}

export type AdapterKind = 'claude' | 'codex' | 'copilot' | 'ompRpc' | 'all';
export type ConformanceMode = 'fixture' | 'live';
```

### ConformanceTestResult

```typescript
export interface ConformanceTestResult {
  adapter: string;
  testName: string;
  passed: boolean;
  error?: string;
  duration: number;
}
```

### ConformanceReport

```typescript
export interface ConformanceReport {
  adapter: string;
  mode: ConformanceMode;
  timestamp: string;
  tests: ConformanceTestResult[];
  allPassed: boolean;
}
```

### Usage

```typescript
import { runConformance, formatConformanceSummary } from "@satori/batman/conformance";

const report = await runConformance({
  adapter: 'claude',
  mode: 'fixture',
  stateDir: '/tmp/bat-state',
  repo: '/path/to/repo',
  outputFile: 'conformance-report.json',
});

console.log(formatConformanceSummary(report));
```

### Test Modes

- **Fixture mode**: Zero-model-call tests that verify structural correctness
- **Live mode**: Real model-call tests that verify end-to-end behavior

### Output

Conformance tests produce JSONL output (one JSON object per line) suitable for CI integration. Reports can be written to a file for later analysis.

---

## Migration Guide

### From M3 to M4

If you're upgrading from M3, here's what you need to know:

1. **Configuration**: The new `RuntimePolicy` replaces the old flat configuration. Use `config::resolve_effective_policy()` to load and merge configuration layers.

2. **Security**: The new `Redactor` and `OrgRedactionRule` provide more flexible redaction. Update your YAML configuration to use the new `security.patterns` format.

3. **Recovery**: The new `RecoveryCoordinator` automatically recovers stuck runs after crashes. No migration needed, but you can configure recovery behavior via `RecoveryConfig`.

4. **Doctor**: The new `Doctor` provides comprehensive health checking. Use it in your startup sequence to verify runtime health.

5. **Release**: Build with `cargo build --release --target <triple>` and package with `cargo run -p batman-xtask -- package --target <triple> --binary <path>`.

6. **Conformance**: The new TypeScript conformance gates provide structured testing. Use `runConformance()` in your CI pipeline.

---

## API Reference

### Rust API

```rust
// Configuration
use batman_runtime::config::{self, RuntimePolicy, RolloutGates};

// Security
use batman_runtime::security::{self, Redactor, OrgRedactionRule};
use batman_runtime::audit::{self, Retention, Export};

// Recovery
use batman_runtime::recovery::{self, RecoveryCoordinator, RecoveryConfig};

// Doctor
use batman_runtime::doctor::{self, Doctor};

// CLI
// batcave serve --state-dir <dir> --repo <repo>
// batcave status --state-dir <dir> --repo <repo>
// batcave stop --state-dir <dir> --repo <repo>
// batcave audit export --state-dir <dir> --repo <repo> --output <file>
```

### TypeScript API

```typescript
import { runConformance, formatConformanceSummary } from "@satori/batman/conformance";

// Conformance testing
const report = await runConformance(config);
console.log(formatConformanceSummary(report));
```

---

## Examples

### Example 1: Loading Configuration

```rust
use batman_runtime::config;

let policy = config::resolve_effective_policy(
    Some(&org_path),
    Some(&repo_path),
    Some(&user_path),
    None,
)?;

println!("Fingerprint: {}", policy.fingerprint);
println!("Max workers: {}", policy.max_workers);
println!("Concurrency ceiling: {}", policy.concurrency_ceiling);
```

### Example 2: Applying Redaction

```rust
use batman_runtime::security::Redactor;

let redactor = Redactor::default();
let raw_event = "my_api_key=sk-1234567890abcdef";
let redacted = redactor.redact_text(raw_event);
// redacted = "my_api_key=[REDACTED]"
```

### Example 3: Running Recovery

```rust
use batman_runtime::recovery::{RecoveryCoordinator, RecoveryConfig};

let db = Arc::new(DatabaseHandle::start(state_db_path).await?);
let config = RecoveryConfig::default();
let coordinator = RecoveryCoordinator::new(db, config);
let result = coordinator.recover().await?;

println!("Recovered {} runs", result.recovered_count);
for run in &result.recovered_runs {
    println!("  {} -> {} (success: {})", run.run_id, run.new_state, run.success);
}
```

### Example 4: Health Checking

```rust
use batman_runtime::doctor::Doctor;

let doctor = Doctor::new(Some(db), Some(state_dir), Some(policy));
let result = doctor.check().await?;

if result.healthy {
    println!("Runtime is healthy");
} else {
    eprintln!("Failed checks:");
    for check in &result.failed_checks {
        eprintln!("  {}: {}", check.check_name, check.error);
    }
}
```

---

## Testing

### Running Tests

```bash
# Run all tests
cargo test --workspace

# Run specific test
cargo test -p batman-runtime --test redaction

# Run clippy
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

### Integration Tests

Integration tests are located in `crates/runtime/tests/`:

- `redaction.rs`: Tests for the redaction boundary
- `audit.rs`: Tests for retention and export
- `adapter_registry.rs`: Tests for adapter registry
- `claude_live.rs`: Live conformance tests for Claude adapter (gated on `BATMAN_LIVE_CLAUDE=1`)

---

## Known Gaps

See [known-gaps.md](./known-gaps.md) for a comprehensive list of known gaps and limitations.

---

## Future Work

- [ ] Full database integration for `RecoveryCoordinator`
- [ ] Full database integration for `Doctor`
- [ ] CLI integration for `audit export` command
- [ ] Additional conformance test scenarios
- [ ] Performance optimization for large event journals

---

## Contributing

Contributions are welcome! Please see the [CONTRIBUTING.md](../CONTRIBUTING.md) file for guidelines.

---

## License

See the [LICENSE](../LICENSE) file for details.

---

## Support

For questions or issues, please open a GitHub Issue or contact the BATMAN team.
