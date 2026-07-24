# Platform binaries as npm optional leaf packages

* Status: Accepted
* Date: 2026-07-24

## Context and Problem Statement

The daemon is a native binary, but the extension ships as a normal npm package supporting four
target triples (macOS/Linux × arm64/x64). Bundling every platform's binary into the main package
bloats every install regardless of platform; a postinstall download step introduces a network
dependency and a supply-chain surface at install time. How should a packaged (non-dev) install
get the right binary for its platform, and know it hasn't been tampered with or gotten out of sync
with the extension version?

## Decision Drivers

* Only supported platforms (macOS and glibc Linux on arm64/x64) should ever be selected — anything
  else must fail with a typed error, never a silent fallback.
* Installing the extension shouldn't require a network round-trip beyond what `npm install` itself
  already does.
* A binary that's corrupted, or from a mismatched extension version, must be caught before it's
  ever spawned.

## Considered Options

* Four npm `optionalDependencies` leaf packages (`@satori/batman-darwin-arm64`, `-darwin-x64`,
  `-linux-arm64-gnu`, `-linux-x64-gnu`), each containing one platform's binary plus a
  `manifest.json` (name, version, target, SHA-256, size); the extension resolves and verifies the
  matching leaf at runtime.
* A `postinstall` script that downloads the correct binary from a release URL.
* Ship all four binaries inside the main `@satori/batman` package.

## Decision Outcome

Chosen option: four optional leaf packages with an integrity manifest.
`resolveBatcave(platform, arch, libc, env)` maps the runtime's own tuple to exactly one leaf,
rejecting every other tuple with a typed `UnsupportedPlatformError`. For a packaged binary, it
verifies the leaf's `manifest.json` SHA-256 against the actual file and requires the leaf's
version to equal the extension's own version before ever returning a usable path — a mismatched or
corrupted binary fails with `BinaryIntegrityError` before any spawn is attempted.

### Positive Consequences

* npm's own dependency resolution skips incompatible-platform optional dependencies for free — no
  custom download logic needed.
* Integrity verification catches "half-updated `node_modules`" or corruption before it ever
  reaches a `spawn()` call.
* `OMP_BATMAN_BINARY` remains a clean escape hatch for development, explicitly bypassing all of
  this (and explicitly documented as doing so) rather than needing its own parallel integrity
  story.

### Negative Consequences

* Publishing a release means publishing five packages (the main extension plus four leaves), not
  one — a real, ongoing release-process cost.
* The integrity check only runs for the *packaged* path; the override path trusts whatever
  absolute, executable file it's pointed at, which is the correct tradeoff for development but
  worth remembering when reasoning about what's actually verified in a given run.

## Pros and Cons of the Options

### Optional leaf packages + integrity manifest (chosen)

* Good, because platform selection and integrity verification both become mechanical, testable
  steps with no network dependency beyond the initial install.
* Bad, because it multiplies the number of packages that must be published together, correctly,
  every release.

### Postinstall download script

* Good, because it keeps the main package small regardless of how many platforms are supported.
* Bad, because it introduces a runtime network dependency at install time and a real
  supply-chain-attack surface (a compromised download URL or CDN) that this project's "no cloud
  dependency" instinct argues against.

### Bundle all binaries in one package

* Good, because it's the simplest possible distribution model — one package, no leaf resolution
  logic at all.
* Bad, because every install downloads three platforms' worth of binary it will never run,
  multiplying package size for no benefit.

## Links

* Narrated in `../journal.md`, commit `39596bc`
