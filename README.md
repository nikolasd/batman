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

### For users (just want to use it)

**Install from local repository (works NOW for macOS ARM):**
```bash
# Clone the repo first, then run from repo root:
./scripts/install-local.sh
```

This installs:
- `batcave` runtime to `~/.batman/bin/batcave` (from `packages/batman-darwin-arm64/bin/batcave`)
- OMP extension to `~/.batman/lib/node_modules/@satori/batman`
- No root privileges required

**Note:** This currently only supports macOS ARM (pre-built binary exists at `packages/batman-darwin-arm64/bin/batcave`). For other platforms, build from source first: `cargo build -p batman-runtime`, then copy the binary to the appropriate `packages/batman-*/bin/` directory.

**To uninstall:**
```bash
rm -rf ~/.batman
rm -f /usr/local/bin/batcave 2>/dev/null || true
```

**Future: Install from GitHub Releases (once first release is published):**
```bash
curl -fsSL https://raw.githubusercontent.com/nikolasd/batman/main/scripts/install.sh | bash
```

**Future: Install via Homebrew (once a tap is created):**
```bash
brew tap nikolasd/batman
brew install batman
```

## Publishing a Release

To publish a new release (which triggers automatic binary building and publishing):

```bash
# Update the version in packages/extension/package.json first
# Then publish:
cargo run -p batman-xtask -- publish
```

This creates a git tag `v<version>` and pushes it to origin, triggering the [release.yml](.github/workflows/release.yml) CI/CD pipeline which:
1. Builds `batcave` for macOS ARM/Intel, Linux x64/ARM, and Windows
2. Creates a GitHub Release with the binaries as assets
3. Makes the install script work for end users

Once the release is published, users can install via:
```bash
curl -fsSL https://raw.githubusercontent.com/nikolasd/batman/main/scripts/install.sh | bash
```

### For developers (want to build/modify)

**Prerequisites:** Rust 1.97+, Bun 1.3.14+, macOS or glibc Linux on arm64/x64. For the full OMP integration you also need OMP ≥ 17.0.7.

```bash
git clone https://github.com/nikolasd/batman.git
cd batman
bun install                 # link workspaces, install extension deps
bun run check               # schema drift check + build + all tests
cargo build -p batman-runtime
```

This builds the `batcave` daemon binary in `target/debug/batcave`.

### Get started in 5 minutes

1. **Build the daemon:**
   ```bash
   cargo build -p batman-runtime
   ```

2. **Start the daemon** (replaces `$PWD` with your repo path):
   ```bash
   ./target/debug/batcave serve --state-dir /tmp/batman-state --repo "$PWD" --idle-seconds 30 &
   ```

3. **Check runtime status** (no model call required):
   ```bash
   ./target/debug/batcave status --state-dir /tmp/batman-state --repo "$PWD"
   ```

You now have a running BATMAN daemon with a durable event journal. Done.

### Full OMP integration

To use orchestration tools (task/worker/run management), start an interactive OMP session:

```bash
OMP_BATMAN_BINARY="$PWD/target/debug/batcave" \
  omp --extension ./packages/extension/src/index.ts
```

Then ask the model to use `batman_task`, `batman_worker`, and `batman_run`, and open `/batman` to watch runs live. See [docs/manual-testing.md](docs/manual-testing.md) for the full walkthrough.

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
