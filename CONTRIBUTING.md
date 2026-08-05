# Contributing to BATMAN

Thank you for your interest in contributing to BATMAN! This document provides guidelines and instructions for contributing to the project.

## Development Environment

### Prerequisites

- **Rust** (version 1.97.1, pinned in `rust-toolchain.toml`)
  - Recommended: install via [rustup](https://rustup.rs) — automatically respects the pinned version
  - Alternative: `brew install rust` (no automatic version pinning; verify with `rustc --version`)
- **Bun** (version 1.3.14 or later)
  - Install via Homebrew: `brew install oven-sh/bun/bun`

### Setup

```bash
# Clone the repository
git clone https://github.com/nikolasd/batman.git
cd batman

# Install JS deps and build the batcave runtime in one step
bun run setup
```

## Running Tests

### Rust Tests

```bash
# Run all Rust tests
cargo test

# Run specific test suite
cargo test --test adapter_contract
cargo test --test approval
cargo test --test audit
# ... (see docs/getting-started.md for full list)

# Run with specific features
cargo test --features "feature1,feature2"
```

### TypeScript Tests

```bash
# Run all TypeScript tests
bun test

# Run specific test file
bun test packages/extension/src/__tests__/approval-ui.test.ts
```

### Full Test Suite

```bash
# Run all tests (Rust + TypeScript)
bun run check
```

## Code Style

### Rust

- Follow [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/)
- Use `cargo fmt` to format code: `cargo fmt --all`
- Use `cargo clippy` to check for common issues: `cargo clippy --all-targets --all-features -- -D warnings`
- Edition 2024, Rust 1.97.1 minimum

### TypeScript

- Use TypeScript with strict mode
- Generate bindings from Rust protocol types (never hand-edit generated files)
- Validate all messages with Ajv before extension logic

## Non-Negotiable Invariants

These hold everywhere in the codebase; changes that weaken them will be rejected in review:

1. **Rust types are canonical.** `packages/protocol-ts/src/generated/` and `packages/protocol-ts/schema/batman.schema.json` are build outputs (`bun run generate`). Generated files are never hand-edited.

2. **TypeScript validates every message** received from the daemon with Ajv before it reaches extension logic.

3. **SQLite runs with WAL**, foreign keys, `synchronous=FULL`, and atomic versioned migrations; the event journal is append-only.

4. **Intent is persisted before side effects; content is redacted before it becomes durable.**

5. **Supported platforms are macOS and glibc Linux on arm64/x64** — everything else is rejected with a typed error, never a silent fallback.

6. **OMP owns the task graph**, scheduling, worker selection, policy, approvals, and merge/synthesis decisions — Rust never creates or edits OMP's task graph; a retry always creates a new run and a harness replacement always creates a new worker and run.

7. **Every domain mutation commits its event and broadcasts the same `EventEnvelope` to live `events/subscribe` listeners in the same call** — a mutation that appends without broadcasting silently breaks the embedded monitor.

## Repository Layout

```
crates/protocol/          Canonical Rust wire types (source of truth for the protocol)
crates/runtime/           The batcave daemon: CLI, lifecycle, IPC server, SQLite journal, security,
                          domain persistence, orchestration/coordination/approval services
crates/xtask/             Codegen (schema + TS bindings) and platform package assembly
packages/extension/       The OMP extension: client, launcher, platform loader, orchestration
                          tools, OMP-native reconciliation, embedded /batman monitor
packages/protocol-ts/     Generated TypeScript bindings + JSON Schema + Ajv validators
packages/batman-*/        Per-platform binary leaf packages (npm optionalDependencies)
fixtures/                 Cross-language golden fixtures (protocol frames, state roots, repo ids)
docs/                     Engineering documentation (start here: docs/getting-started.md)
```

## Making Changes

### Before You Start

1. Check existing issues and PRs to avoid duplicate work
2. Read the relevant documentation in `docs/`
3. Understand the non-negotiable invariants above

### Making Changes

1. Create a new branch for your changes: `git checkout -b feature/my-feature`
2. Make your changes
3. Run tests: `bun run check`
4. Commit with a clear, descriptive commit message
5. Push and create a Pull Request

### Commit Messages

- Use clear, descriptive commit messages
- Reference issue numbers when applicable
- Follow conventional commits format if possible

## Pull Request Process

1. Ensure your PR:
   - Passes all tests (`bun run check`)
   - Follows the non-negotiable invariants
   - Includes documentation updates if needed
   - Has a clear description of what changes and why

2. Submit your PR:
   - Fill out the PR template completely
   - Link any related issues
   - Request review from maintainers

3. Address review feedback:
   - Respond to all comments
   - Make requested changes
   - Update tests if needed

## Releasing

Maintainers publish new versions to the private npm registry via CI, not manually:

```bash
# Bump the version in packages/extension/package.json and every packages/batman-*/package.json first
git tag v<version>
git push origin v<version>
```

Pushing a `v*` tag triggers [`.github/workflows/release.yml`](.github/workflows/release.yml), which:
1. Builds `batcave` for macOS ARM/Intel and Linux x64/ARM
2. Assembles each platform leaf package (`cargo run -p batman-xtask -- package`)
3. Builds the extension bundle (`bun run build`) and publishes all 4 leaf packages plus `@nikolasd/batman` to the private registry

**Requires:** a `NPM_TOKEN` repository secret with publish access to the private registry.

## Documentation

When contributing, consider updating documentation:

- **docs/getting-started.md** - Development setup and workflows
- **docs/manual-testing.md** - Manual testing procedures
- **docs/architecture.md** - System design
- **docs/adr/** - Architectural decisions

## Questions?

- Open an issue for questions or discussions
- Check existing documentation in `docs/`
- Reach out to maintainers

Thank you for helping make BATMAN better!
