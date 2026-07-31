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

BATMAN supports the following AI adapters, each with their own conformance profile:

### Claude Adapter
- **Protocol**: Claude API v1
- **Status**: Stable
- **Features**: Streaming responses, tool use, image inputs

### Codex Adapter
- **Protocol**: OpenAI-compatible API
- **Status**: Stable
- **Features**: Code generation, file operations

### Copilot Adapter
- **Protocol**: GitHub Copilot API
- **Status**: Stable
- **Features**: Code completion, chat, inline edits

### OMP-RPC Adapter
- **Protocol**: BATMAN native RPC
- **Status**: Stable
- **Features**: Full BATMAN protocol support

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
- `workspace/acquire`, `workspace/get` - Workspace management
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
