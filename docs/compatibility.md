# BATMAN Compatibility Guide

This guide documents BATMAN's compatibility with various platforms, adapters, and configurations.

## Supported Platforms

BATMAN supports the following platform/architecture combinations:

| Platform | Architecture | Status |
|----------|--------------|--------|
| macOS   | arm64 (Apple Silicon) | Supported |
| macOS   | x64 (Intel) | Supported |
| Linux   | arm64 | Supported |
| Linux   | x64 | Supported |

### Platform-Specific Notes

- **macOS**: Requires Xcode Command Line Tools for native compilation
- **Linux**: Requires a modern Linux distribution (glibc version not explicitly pinned; tested on recent distros)
- **Windows**: Not currently supported (future milestone)

## Adapter Compatibility

The table below is **generated from real `--live` conformance runs**, not from prose. Each row
records the version the adapter's own probe observed and how many canonical scenarios that run
proved. The reports themselves are committed under `release/live-<adapter>.json`, so every claim
here is checkable.

Reproduce with (`BATMAN_DISABLE_VENDOR_CLI` must be **unset** — it suppresses vendor invocation):

```bash
./target/debug/batcave conformance --adapter <claude|codex|copilot|ompRpc> --live \
  --output release/live-<adapter>.json
```

| Adapter | Observed version | Scenarios passing | Report |
|---------|------------------|-------------------|--------|
| Claude  | `2.1.222`           | 14 / 14 | `release/live-claude.json` |
| Codex   | `codex-cli 0.146.0` | 9 / 14  | `release/live-codex.json` |
| Copilot | `1.0.78`            | 11 / 14 | `release/live-copilot.json` |
| OMP-RPC | `omp/17.2.7`        | 14 / 14 | `release/live-omp-rpc.json` |

A scenario short of 14 is recorded below with its cause. None of them is an unproven assertion:
each carries the vendor's or the environment's own explanation.

### Claude Adapter
- **Protocol**: Claude Code CLI over stdio
- **Status**: Stable — the only adapter whose live suite is fully green
- **Live result**: 14 / 14 scenarios pass against `2.1.222`

### Codex Adapter
- **Protocol**: `codex app-server` JSON-RPC over stdio (`initialize` → `thread/start` → `turn/start`)
- **Status**: Stable; live turn-dependent scenarios currently unprovable on this account
- **Live result**: 9 / 14 against `codex-cli 0.146.0`. `result_usage_artifacts`, `follow_up`,
  `cancellation_scope`, `session_resume`, and `runtime_restart` all share one cause — the account
  cannot run a turn:

  > `usageLimitExceeded: Your workspace is out of credits. Ask your workspace owner to refill in order to continue.`

  This is an account condition, not an adapter defect: `codex login status` reports
  `Logged in using ChatGPT`, `initialize`/`thread/start` succeed, and the turn is refused
  server-side after ~3s. Refill the workspace and the five scenarios become provable with no code
  change.

### Copilot Adapter
- **Protocol**: Agent Client Protocol (ACP) over NDJSON stdio (`copilot --acp`) — not the GitHub
  Copilot HTTP API
- **Status**: Stable; cross-process session resume is a protocol wall, not a defect
- **Live result**: 11 / 14 against `1.0.78` (`authReady=true`). Three scenarios fail for two
  distinct ACP v1 limitations:
  - `session_resume` and `runtime_restart` — a session that completed a real turn cannot be
    reloaded from a new process: `session/load` answers
    `Resource not found: Session <id> not found`. ACP v1 has no durable session handle, so the
    adapter cannot resume across processes.
  - `unexpected_child_observation` — ACP v1's `session/update` schema has no variant this adapter
    could map to `NestedWorkerObserved`, so vendor-side delegation is unobservable. Real gap,
    pending a newer ACP version.

The CLI version compared against the table below is the `agentInfo.version` field reported by
the real ACP `initialize` handshake, **not** the output of `copilot --version` (which prints, for
example, `GitHub Copilot CLI 1.0.78.` — note the trailing period — plus a separate
`copilot update` notice line). An installed CLI version is trusted only after it has been
empirically verified with a real handshake; a version not in the table is refused, never assumed
"nearby" compatible.

| CLI Version | ACP Protocol Version |
|--------------|----------------------|
| 1.0.73       | 1                    |
| 1.0.75       | 1                    |
| 1.0.78       | 1                    |

Supported ACP protocol version range: 1–1 (`COPILOT_MIN_ACP_PROTOCOL_VERSION` through
`COPILOT_MAX_ACP_PROTOCOL_VERSION`). A negotiated protocol version outside this range is refused
with `AdapterError::incompatible_version`, since this adapter's normalizer only understands the
v1 field names.

### OMP-RPC Adapter
- **Protocol**: BATMAN-driven `omp --mode rpc` NDJSON frames over stdio
- **Status**: Stable — **14 / 14, fully green**
- **Live result**: 14 / 14 against `omp/17.2.7`, `passed: true`, reproduced on three consecutive runs with zero local providers in `omp`'s catalog. Report: `release/live-omp-rpc.json`.

  Two scenarios (`follow_up`, `cancellation_scope`) previously failed, and `probe` was coupled
  to a local model server. The **root cause was a design flaw in the conformance harness**, not an
  environment condition: the harness resolved its stand-in model by picking the first *local*
  (`lm-studio`/`omlx`) selector from `omp models --json`, making three scenarios depend on a
  separate inference server being running — despite none of them ever sending a prompt.

  **Fix:** `resolve_first_local_selector` → `resolve_conformance_selector`, which takes the first
  selector of *any* provider. `omp` ships a cloud catalog of 583 models with the same first entry
  regardless of environment, so conformance is now deterministic.

  A second, independent defect was found: `omp` persists provider discovery, so a
  stripped-environment spawn wrote "no local providers" into the shared catalog, leaving the
  operator's own `omp models` listing empty. The spawn now passes `LM_STUDIO_BASE_URL` (an
  address, not a credential) purely to prevent degrading state BATMAN does not own.

  A third defect surfaced in `tests/omp_rpc_adapter.rs`: the same empty-allowlist bug had been
  silently skipping two real-binary tests *and* poisoning the catalog on every `cargo test`.
  Fixing it exposed a wrong assertion about `omp 17.2.7`'s response format (flat strings vs
  objects with `.scheme`). All 23 tests now pass.

  Real runs were never affected: `OmpRpcAdapter` builds its environment from
  `WorkerProfile::environment_allowlist`. An operator using a local model lists
  `LM_STUDIO_BASE_URL` there, exactly as they would an API key.

## Configuration Compatibility

### Configuration Layers

BATMAN supports three configuration layers with strict precedence:

1. **Org config** (lowest precedence)
   - Location: Specified by `--org-config` flag or `BATMAN_ORG_CONFIG` env var
   - Format: YAML with strict unknown-key rejection

2. **Repo config**
   - Location: `<repo>/.batman/config.yaml`
   - Overrides org config

3. **User config**
   - Location: `~/.batman/config.yaml` or `~/.config/batman/user.yaml`
   - Highest static precedence

4. **Per-run params**
   - CLI flags and environment variables
   - Overrides all config files

### Configuration Validation

All configuration files are validated against strict schemas:
- Unknown keys are rejected with line/column diagnostics
- Required fields are enforced
- Type checking is performed at load time

Known configuration keys: `retention`, `max_workers`, `display`, `security`, `models`, `concurrency`, `rollout_gates`, `locks`.

## Protocol Compatibility

BATMAN uses a versioned JSON-RPC protocol:

- **Current version**: 1.0 (constant `RUNTIME_PROTOCOL_VERSION`)
- **Transport**: Newline-delimited JSON over Unix domain sockets
- **Frame size**: Negotiated during `initialize` handshake
  - Minimum: 64 KiB (`PROTOCOL_MIN_FRAME_BYTES`)
  - Default: 4 MiB (`DEFAULT_MAX_FRAME_BYTES`)

### API Compatibility

The following RPC methods are implemented (per `crates/protocol/src/method.rs`):

**Foundation methods:**
- `initialize` - Handshake and capability negotiation
- `runtime/status` - Query runtime state
- `events/subscribe` - Subscribe to event stream
- `events/replay` - Replay historical events
- `runtime/shutdown` - Graceful shutdown

**Orchestration extension methods:**
- `task/upsert`, `task/get` - Task management
- `worker/create`, `worker/list`, `worker/get` - Worker management
- `run/submit`, `run/list`, `run/get`, `run/retry`, `run/cancel` - Run management
- `message/send`, `message/list` - Message handling
- `approval/list`, `approval/decide` - Approval workflows
- `coordination/child/list`, `coordination/child/decide`, `coordination/task`, `coordination/peers`, `coordination/send`, `coordination/requestChild`, `coordination/publishArtifact`, `coordination/reportBlocked`, `coordination/askPolicy` - Multi-agent coordination
- `reconcile/omp` - OMP reconciliation
- `profile/register` - Profile registration
- `workspace/acquire`, `workspace/get`, `workspace/release`, `workspace/inspect`, `workspace/apply` - Workspace management
- `artifact/list`, `artifact/fetch` - Artifact management
- `policy/violation/decide` - Policy violation decision (stub implementation)

## Backwards Compatibility

### Breaking Changes

Breaking changes are introduced only in major version bumps:
- Major version changes may remove deprecated RPC methods
- Schema migrations are forward-compatible

## Known Incompatibilities

### Unsupported Configurations

- Running multiple BATMAN instances for the same repository (single-instance lock enforced)
- Using non-YAML configuration file formats
- Connecting to runtimes with incompatible protocol versions

### Platform Limitations

- Windows is not currently supported
- Some display backends (Herdr, Tmux) require specific terminal emulators

## Updating BATMAN

When updating BATMAN:

1. **Check protocol version**: Ensure your OMP extension supports the new runtime protocol
2. **Migrate configuration**: Review any deprecated config keys
3. **Test adapters**: Verify your adapters still pass conformance checks
4. **Review changelog**: Check for breaking changes in release notes

For detailed update instructions, see [operations.md](./operations.md#upgrading).
