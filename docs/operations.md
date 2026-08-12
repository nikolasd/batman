# BATMAN Operations Guide

**Audience & purpose:** end users and operators who need to run `batcave` by hand, troubleshoot a
stuck daemon, or upgrade/uninstall — a companion to [`plugin-usage.md`](plugin-usage.md), the user
manual (day-to-day tool usage doesn't need this document; troubleshooting does). This guide covers
running and troubleshooting the `batcave` daemon in practice: lifecycle procedures, crash recovery,
upgrades, and the real (as opposed to imagined) install/uninstall path. For the full flag-by-flag
command reference, see [`cli-reference.md`](cli-reference.md) — this guide doesn't repeat flags,
only the procedures around them.

## Daemon Lifecycle

### Starting the runtime

```bash
batcave serve --repo <path> --state-dir "$HOME/.omp/batman" [--idle-seconds <n>]
```

In normal use you don't run this yourself — the OMP extension spawns it on first use per
repository (see [`plugin-usage.md`](plugin-usage.md#how-the-extension-finds-and-starts-batcave)).
Run it by hand for debugging or CI. See [`cli-reference.md`](cli-reference.md#batcave-serve) for
every flag, and the [state-directory note](cli-reference.md#before-you-start-state-directories)
before you pick a `--state-dir` — the CLI's own default when it's omitted (`.batman` in the current
directory) is *not* the same location the extension resolves and uses. Always pass `--state-dir`
explicitly (as above) unless you specifically want the bare `.batman`-in-cwd fallback.

### Single-instance enforcement

`serve` takes an advisory `flock(2)` on `<runtime-dir>/runtime.lock` — not an `O_EXCL` create. The
lock file itself is never deleted; a "stale" lock is simply one whose `flock` the kernel has already
released because its owning process died. If a runtime is already running, the new process exits
**73** (`EX_TEMPFAIL`) and prints the live holder's identity as JSON on stdout.

### Graceful shutdown

```bash
batcave stop --repo <path> --state-dir "$HOME/.omp/batman"
```

The runtime journals a stop record, closes the database actor, removes the socket, then releases
the lock — in that order, so the socket's disappearance is proof the journal already shut down
cleanly. `stop` polls for the socket to disappear for up to 10 seconds after sending `SIGTERM`; if
no runtime is found holding the lock, it exits **1** immediately rather than signalling anything.

### Watching events live

```bash
# Every run in the project, live-tailed after an initial catch-up replay
batcave monitor --repo <path> --state-dir "$HOME/.omp/batman"

# Filtered to one run
batcave monitor --repo <path> --state-dir "$HOME/.omp/batman" --run-id <run-id>
```

There's no separate "replay" vs. "live" mode — `monitor` always replays what's already journaled
and then keeps tailing new events until you interrupt it. If the daemon restarts mid-session,
`monitor` reconnects and resumes from where it left off rather than exiting. For day-to-day use
inside OMP, prefer `/batman` (see [`plugin-usage.md`](plugin-usage.md#the-embedded-monitor-batman))
— this CLI form is for scripting or debugging outside an OMP session.

### Diagnosing a runtime

```bash
batcave doctor --repo <path> --state-dir "$HOME/.omp/batman" [--json]
```

Runs the full check catalog documented in [`cli-reference.md`](cli-reference.md#batcave-doctor)
against the same paths `serve` would use. Exits 0 healthy, 1 unhealthy. Inside OMP, prefer
`batman_doctor` / `/batman-doctor` — same check catalog, no need to resolve paths by hand.

## Crash Recovery

`serve` runs recovery once, synchronously, right after opening the database and before the socket
accepts any connection. It looks for runs stuck in `queued`, `starting`, or `working` whose most
recent journaled event (or creation time, if none) is older than a 5-minute threshold, and
transitions each to `failed` — there's no evidence the work completed, so it can't be resolved any
other way. Runs sitting in `paused`, `waitingUser`, or `waitingPeer` are left alone by default, since
those states mean the run is intentionally waiting on a human or a peer worker, not stuck. Each stuck
run is recovered independently; one failure doesn't abort the sweep for the others.

If you suspect a run is wedged and want to confirm before the next restart naturally sweeps it,
`batcave doctor`'s `stale_runs` check reports the same condition without transitioning anything.

### Recovering from an unexpected crash

1. **Check the log** (only present without `--foreground`): `cat <runtime-dir>/runtime.log`
2. **Check for orphaned processes:** `ps aux | grep batcave`
3. **Restart:** `batcave serve --repo <path> --state-dir "$HOME/.omp/batman"` — there's nothing to clean up
   by hand first. The lock file doesn't need removing (a crashed process's `flock` is already
   released by the kernel), and the next `serve` runs recovery automatically on startup.

## Install, Upgrade, and Uninstall

BATMAN ships as an OMP marketplace plugin (extension + skills, cloned from this repository) plus a
`batcave` binary downloaded on demand as a GitHub Release asset. **There is no Homebrew formula,
apt/deb/rpm package, or any other system package** — don't reach for a package manager here.

### Installing / uninstalling

```bash
/marketplace add nikolasd/batman           # registers this repo as a marketplace source
/marketplace install batman@batman         # installs the extension + skills
/batman-runtime-install                    # downloads and verifies the batcave binary
/marketplace uninstall batman@batman       # removes the extension + skills
```

**This repository is private.** `/marketplace add` git-clones it, so it needs your own GitHub read
access to `nikolasd/batman` (an SSH key registered with GitHub, or a `gh auth login` session backed
by a git credential helper). `/batman-runtime-install` additionally needs a `GITHUB_TOKEN` or
`GH_TOKEN` environment variable, or that same `gh auth login` session, to download the asset — see
the README's [Installation](../README.md#installation) section. The `batcave` binary itself resolves
in two tiers (see
[`plugin-usage.md`](plugin-usage.md#how-the-extension-finds-and-starts-batcave)): `OMP_BATMAN_BINARY`
(a local-development override) if set, otherwise the SHA-256-verified binary
`/batman-runtime-install` cached under the BATMAN state root — there's no separate binary install
step beyond that command.

After uninstalling, confirm no `batcave` process is still running: `ps aux | grep batcave`, and
`kill <pid>` anything that's left (this shouldn't happen in normal operation — the extension doesn't
detach a `batcave` process that outlives every session using it on purpose, but a hard OMP crash can
leave one behind since `serve` is spawned detached).

Removing state (event journal, leases, artifacts) is a separate, optional step, since it's not
part of the package: `rm -rf <state-root>` for the resolved state root (see the
[state-directory precedence](cli-reference.md#before-you-start-state-directories) in the CLI
reference) — or just the one repository's subdirectory under `<state-root>/repos/<repository-id>/`
if you want to keep other repositories' history.

### Upgrading

The extension and the `batcave` binary are no longer one atomic install — upgrading each is a
separate step. `/marketplace upgrade batman@batman` refreshes the extension + skills from this
repository; if that bumps the extension's version, re-run `/batman-runtime-install` to download the
matching binary (a version-mismatched cached binary is rejected rather than silently reused). If
you're testing an unreleased build instead, use the `OMP_BATMAN_BINARY` override described in
[`plugin-usage.md`](plugin-usage.md#how-the-extension-finds-and-starts-batcave) instead of
installing anything.

1. **Stop the running daemon** for any repository you're about to touch: `batcave stop --repo ...`
   (or just let the next `ensureRuntime()` call reconnect after the update — a stale old binary
   still running won't be replaced automatically, so stop it first if you want the new version
   active immediately).
2. **Update**: `/marketplace upgrade batman@batman`, then `/batman-runtime-install` if the version
   changed. Confirm with `batcave version` (should report the new version) and `batcave doctor --json`
   (should report `healthy: true`, or only expected, pre-existing failures).

## Troubleshooting

**Runtime won't start:**
- Check whether one's already running: `batcave status --repo <path> --state-dir "$HOME/.omp/batman"` (exit 73 from `serve`
  means another instance holds the lock — that's not a bug, connect to it instead of restarting).
- Check the log: `cat <runtime-dir>/runtime.log`.
- Run `batcave doctor --json` — it doesn't need a live connection and will usually name the exact
  failing check (permissions, disk space, schema mismatch, unsupported platform, etc.).

**`batman_status` fails but you're not sure why:**
- Run `batman_doctor` (or `/batman-doctor`) — it diagnoses without needing a live connection, which
  is exactly the case `batman_status` can't help with.

**Doctor reports a `state_dir_writable` or `socket_permissions` failure:**
- Check ownership and mode: `ls -ld <runtime-dir>` should be `0700`, owned by you;
  `ls -l <runtime-dir>/runtime.sock` (if present) should be `0600`.

**Adapter conformance failures:**
- `batcave adapters --json` reports each adapter's effective capabilities from a fixture run — start
  there before assuming a live-vendor issue.
- `BATMAN_DISABLE_VENDOR_CLI=1 cargo test --test conformance` runs the same checks offline; drop the
  env var (and set the adapter's live-gate var, e.g. `BATMAN_LIVE_CLAUDE=1`) to exercise the real
  vendor CLI.
- Confirm the vendor CLI itself is installed and authenticated — a conformance failure here is
  usually the vendor CLI, not BATMAN.

For open implementation gaps (as opposed to operational issues), see [`REVIEW.md`](../REVIEW.md) — it's
the single source of truth for what's genuinely unfinished, verified against the current codebase.
