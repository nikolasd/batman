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

BATMAN is distributed as an OMP plugin. Install it with OMP's own plugin manager:

```bash
omp install @satori/batman
```

This is OMP's native package manager (`omp plugin install`, equivalent to `omp plugin link` for local paths). One command:

- Installs to `~/.omp/plugins` — user-owned directory, **no root privileges required**
- Resolves `@satori/batman` and its matching platform binary package (`@satori/batman-<platform>`, containing `batcave`) together via npm `optionalDependencies` — the extension and the runtime install as one unit
- Registers the extension for **automatic discovery** on every future `omp` launch — no `--extension` flag needed

**Requires:** access to the private npm registry `@satori/*` packages are published to. Configure the `@satori` scope once — see [`.npmrc`](.npmrc) for the registry template (replace the placeholder URL and set `SATORI_NPM_TOKEN`), or use your organization's standard registry auth setup.

**To uninstall:**
```bash
omp plugin uninstall @satori/batman
```

**To upgrade:**
```bash
omp plugin upgrade @satori/batman
```

## Development

For contributors building or modifying BATMAN itself (not for end users — see [Installation](#installation) above):

**Prerequisites:** Rust 1.97+, Bun 1.3.14+, macOS or glibc Linux on arm64/x64. For the full OMP integration you also need OMP ≥ 17.0.7.

```bash
git clone https://github.com/nikolasd/batman.git
cd batman
bun install                 # link workspaces, install extension deps
bun run check               # schema drift check + build + all tests
cargo build -p batman-runtime
```

To exercise the extension against your local changes before publishing, load it from its source path directly:

```bash
OMP_BATMAN_BINARY="$PWD/target/debug/batcave" \
  omp --extension ./packages/extension/src/index.ts
```

Ask the model to use `batman_task`, `batman_worker`, and `batman_run`, then open `/batman` to watch runs live. See [docs/manual-testing.md](docs/manual-testing.md) for the full walkthrough.

## Publishing

Maintainers publish new versions to the private npm registry via CI, not manually:

```bash
# Bump the version in packages/extension/package.json and every packages/batman-*/package.json first
git tag v<version>
git push origin v<version>
```

Pushing a `v*` tag triggers [`.github/workflows/release.yml`](.github/workflows/release.yml), which:
1. Builds `batcave` for macOS ARM/Intel and Linux x64/ARM
2. Assembles each platform leaf package (`cargo run -p batman-xtask -- package`)
3. Publishes all 4 leaf packages and `@satori/batman` to the private registry

**Requires:** a `SATORI_NPM_TOKEN` repository secret with publish access to the private registry.

## Contributing

Contributions are welcome. Before submitting a PR:

1. Read [`docs/getting-started.md`](docs/getting-started.md) and [`docs/architecture.md`](docs/architecture.md).
2. Run `bun run check` — schema drift, build, and all tests must pass.
3. Follow the [Non-negotiable invariants](#non-negotiable-invariants) — changes that weaken them will be rejected.
4. Use descriptive commit messages. Reference issue numbers when applicable.
5. Fill out the PR template completely, link related issues, and request review.

For detailed guidelines, see [`CONTRIBUTING.md`](CONTRIBUTING.md).

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

These are tracked in [`docs/known-limitations.md`](docs/known-limitations.md) and [`TODO.md`](TODO.md). If you're evaluating BATMAN for production, review those first.
