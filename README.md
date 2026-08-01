# B.A.T.M.A.N.

**B**orderline **A**wesome **T**ool for **M**ultiagent **A**utomation by **N**ikolas.

BATMAN is an [Oh My Pi (OMP)](https://github.com/can1357/oh-my-pi) extension backed by a durable, repository-scoped local daemon. OMP stays the brain — task intake, scheduling, worker selection, approvals, merge decisions, synthesis. BATMAN is the hands: it supervises worker processes, speaks harness adapter protocols, persists a durable event journal, recovers after crashes, and feeds display backends.

Everything is delivered as an external npm package (`@satori/batman`) plus a Rust daemon binary (`batcave`) — no OMP fork, no private APIs.

## Why BATMAN?

Multiagent automation is hard. Most frameworks either:

- **Put all the intelligence in the agent** (risky, hard to debug, no recovery)
- **Put all the intelligence in your code** (complex, brittle, no replay)

BATMAN splits the difference: **OMP decides what to do, BATMAN ensures it happens and can be replayed.**

Key benefits:

- **Durable event journal.** Every action is persisted before it executes. Crash? Replay from the last known state.
- **Redaction by construction.** Secrets never reach the journal. Enforced at the type level.
- **SQLite-backed.** Query your automation history with SQL. No proprietary format.
- **Adapter-agnostic.** Claude, Codex, Copilot, OMP-RPC — plug in any worker.
- **No model calls required for monitoring.** Check runtime status, task state, or run history without spending a token.

If you're building multiagent systems that need to be auditable, recoverable, and debuggable, BATMAN is your foundation.

## Installation

BATMAN consists of two components, both installed in one step:
- **Plugin**: TypeScript extension loaded by OMP
- **Binary**: `batcave` runtime daemon

```bash
omp install @satori/batman
```

This installs:
- Plugin: `~/.omp/plugins/node_modules/@satori/batman` (TypeScript extension)
- Runtime: `batcave` binary from the installed leaf package (discovered by the extension via `import.meta.resolve`)

**To uninstall:**
```bash
omp plugin uninstall @satori/batman
```

Requires access to the private npm registry `@satori/*` packages are published to. Configure the `@satori` scope once — see [`.npmrc`](.npmrc) for the registry template (replace the placeholder URL and set `SATORI_NPM_TOKEN`), or use your organization's standard registry auth setup.

## Development

For contributors building or modifying BATMAN itself (not for end users — see [Installation](#installation) above):

**Prerequisites:** Bun 1.3.14+, macOS or glibc Linux on arm64/x64, and Rust — via [rustup](https://rustup.rs) (recommended: automatically respects the pinned `1.97.1` in `rust-toolchain.toml`) or your system package manager. For the full OMP integration you also need OMP ≥ 17.0.7.

```bash
git clone https://github.com/nikolasd/batman.git
cd batman
bun run setup               # installs JS deps + builds the batcave runtime
bun run check               # schema drift check + build + all tests
```

To exercise the extension against your local changes before publishing, load it from its source path directly:

```bash
OMP_BATMAN_BINARY="$PWD/target/debug/batcave" \
  omp --extension ./packages/extension/src/index.ts
```

Ask the model to use `batman_task`, `batman_worker`, and `batman_run`, then open `/batman` to watch runs live. See [docs/manual-testing.md](docs/manual-testing.md) for the full walkthrough.

## Contributing

Contributions are welcome. Before submitting a PR:

1. Read [`docs/getting-started.md`](docs/getting-started.md) and [`docs/architecture.md`](docs/architecture.md).
2. Run `bun run check` — schema drift, build, and all tests must pass.
3. Follow the [Non-Negotiable Invariants](CONTRIBUTING.md#non-negotiable-invariants) — changes that weaken them will be rejected.
4. Use descriptive commit messages. Reference issue numbers when applicable.
5. Fill out the PR template completely, link related issues, and request review.

For detailed guidelines, see [`CONTRIBUTING.md`](CONTRIBUTING.md). For the release/publishing process, see [CONTRIBUTING.md's Releasing section](CONTRIBUTING.md#releasing).

## Author

BATMAN was created by **Nikolas Demiridis** as part of the [Oh My Pi](https://github.com/can1357/oh-my-pi) ecosystem.

For questions, issues, or contributions, please open a GitHub Issue on this repository.

## License

This project is licensed under the [MIT License](LICENSE). See the LICENSE file for full terms.

## Known Limitations

This is a pre-1.0 project. Some things don't work yet:

- **No adapter is wired in production.** The `AdapterRegistry` exists, but production config uses `DenyByDefaultAuthorization`. You can't actually run a model call through this repo — only simulate with fixtures.
- **Credential store for `workerMcp` is not implemented.** `RejectAllWorkerVerifier` is the default.
- **OMP-RPC approval flow is not normalized.** The adapter's `extension_ui_request` frame is silently dropped.
- **No artifact tracking for OMP-RPC.** The `ArtifactProduced` payload is never constructed.
- **PolicyViolationDecide is a stub.** `OrchestrationService` has no `policy` field; quarantine-decision logic deferred.
- **Conformance tests are stubs.** `tests/conformance/run.ts` and `assert-report.ts` write empty reports; the conformance gate in `release.yml` always passes.
- **No JS/TS formatter.** CI format job only checks Rust (`cargo fmt`).

These are tracked in [`docs/known-limitations.md`](docs/known-limitations.md) and [`TODO.md`](TODO.md). If you're evaluating BATMAN for production, review those first.
