# GitHub Release download-cache replaces npm optional leaf packages

* Status: Accepted
* Date: 2026-08-10

## Context and Problem Statement

ADR-0010 chose four npm `optionalDependencies` leaf packages
(`@nikolasd/batman-darwin-arm64`, `-darwin-x64`, `-linux-arm64-gnu`, `-linux-x64-gnu`) so npm's
own dependency resolution would install exactly one platform's binary, verified against a
`manifest.json`. That mechanism only works when the extension itself is installed via `npm
install` (or an npm-compatible package manager resolving `optionalDependencies`).

The extension is not distributed that way. It is installed via the OMP marketplace
(`/marketplace add nikolasd/batman` → `/marketplace install batman@batman`), which git-clones the
repository directly — there is no `npm install` step, so `optionalDependencies` never resolves,
and the four leaf packages would sit on disk unused. Publishing them to an npm registry also meant
maintaining a private registry (`.npmrc`, `NPM_TOKEN`) purely to gate a repository that is already
private on GitHub — the private registry added operational cost (a real host, a token to rotate,
onboarding friction for every new user) without adding any access control the GitHub repo wasn't
already providing.

How should a marketplace-installed extension get the right `batcave` binary for its platform, still
verified and still rejecting unsupported platforms, with no npm publish step at all?

## Decision Drivers

* The marketplace install path never runs `npm install`; any platform-selection mechanism that
  depends on package-manager dependency resolution cannot be reached.
* Only supported platforms (macOS and glibc Linux on arm64/x64) must ever be selected — unchanged
  from ADR-0010.
* A binary that's corrupted, or from a mismatched extension version, must still be caught before
  it's ever spawned — unchanged from ADR-0010.
* No private registry, no `NPM_TOKEN`, no npm publish step of any kind — the private GitHub repo is
  the only access boundary that should exist.

## Considered Options

* **On-demand GitHub Release download, cached under the BATMAN state root.** A `/batman-runtime-install`
  tool/command downloads this extension version's release asset (binary + `manifest.json`) from
  `https://api.github.com/repos/nikolasd/batman/releases/tags/v<version>`, verifies its SHA-256
  against the manifest, and caches it at `<state root>/bin/<version>/batcave`. `resolveBatcave`
  (`platform.ts`) then reads that cache — same integrity check as before (SHA-256 + version match),
  just against a downloaded cache directory instead of an npm leaf package.
* Keep the four npm leaf packages, and have the marketplace-installed extension `npm install` them
  itself at first run.
* Bundle all four platforms' binaries directly into the git-cloned extension source.

## Decision Outcome

Chosen option: on-demand GitHub Release download, cached under the BATMAN state root.
`platform.ts` no longer resolves a leaf package path; it resolves `<state root>/bin/<version>/batcave`
and `<state root>/bin/<version>/manifest.json`, throwing the same `BinarySelectionError`/
`BinaryIntegrityError` typed errors as before (`runtime-not-installed`, `checksum-mismatch`,
`version-mismatch`, `manifest-invalid`, `unsupported-platform`) when the cache is absent or
doesn't check out. `download.ts` is the new module that performs the download (via the GitHub
REST API, using `GITHUB_TOKEN`/`GH_TOKEN`/`gh auth login` for the private repo's read access) and
writes the verified cache; `install.ts` wires it to the `batman_runtime_install` tool and
`/batman-runtime-install` command. The four `packages/batman-*/` directories are now pure release
*build* staging — `batman-xtask package` still writes `bin/batcave` + `manifest.json` into each on
demand, but they are gitignored, never published anywhere, and only ever uploaded as GitHub Release
assets by `release.yml`.

This supersedes ADR-0010, whose distribution assumption (npm-resolved optional dependencies) no
longer holds once the extension isn't installed via `npm install` at all.

### Positive Consequences

* No private registry to host, no `NPM_TOKEN` to rotate or leak — installing the extension needs
  only GitHub read access, which the marketplace clone already requires.
* Same integrity guarantees as ADR-0010's leaf packages: SHA-256 + version match before ever
  returning a usable path, still fully typed and testable (`download.test.ts`, `platform.test.ts`).
* Users who never touch orchestration tools that need the binary never trigger a download —
  `/batman-runtime-install` is explicit, not a postinstall hook, so there's no network dependency
  at plugin-install time either.

### Negative Consequences

* The binary is no longer present the moment the plugin installs; the first orchestration call
  fails with `runtime-not-installed` until `/batman-runtime-install` runs. This is a real UX cost
  compared to npm's automatic optional-dependency resolution, mitigated by `batman-troubleshooting`'s
  skill-level diagnostic ladder and `batman_status`'s actionable failure message.
* Downloading from `api.github.com` at runtime is a real network dependency for the *first* use on
  each machine, the same category of risk ADR-0010 rejected for a postinstall script — accepted
  here because there is no npm-native alternative left once the extension bypasses `npm install`
  entirely.

## Pros and Cons of the Options

### GitHub Release download-cache (chosen)

* Good, because it matches how the extension is actually installed (marketplace git-clone, no
  `npm install`) instead of assuming a package-manager step that never happens.
* Good, because it keeps every integrity property ADR-0010 established, just against a
  downloaded cache instead of a resolved leaf package.
* Bad, because it reintroduces a runtime network dependency ADR-0010 explicitly avoided —
  unavoidable once the extension isn't npm-installed.

### Marketplace-installed extension self-installs npm leaf packages

* Good, because it reuses ADR-0010's leaf-package/manifest infrastructure unchanged.
* Bad, because it requires running `npm install` (or equivalent) from inside a git-cloned plugin
  directory at runtime — a heavier, slower, and more failure-prone operation than a single asset
  download, and still needs a registry (private or public) to install *from*.

### Bundle all four binaries in the git-cloned source

* Good, because it needs no download step and no registry at all.
* Bad, because every clone carries three platforms' worth of binary it will never run, and every
  release commits ~160MB of binaries to git history permanently.

## Links

* Supersedes [ADR-0010](0010-platform-binaries-as-npm-optional-leaf-packages.md).
* `packages/extension/src/platform.ts`, `download.ts`, `install.ts`, `runtime.ts`.
