# BATMAN Operations Guide

This guide covers BATMAN daemon lifecycle management, including start/stop/restart procedures, Herdr coordinated restarts, package rollback, and uninstall semantics.

## Daemon Lifecycle

### Starting the Runtime

Start a BATMAN runtime for a repository:

```bash
batcave serve --repo /path/to/repo [--state-dir /path/to/state] [--idle-seconds 1800]
```

**Options:**
- `--repo`: Required. Path to the repository to serve
- `--state-dir`: Optional. State directory (defaults to `<repo>/.batman`)
- `--idle-seconds`: Optional. Auto-shutdown after N seconds with no connections (default: 1800)
- `--foreground`: Optional. Log to stderr instead of `runtime.log`
- `--org-config`, `--repo-config`, `--user-config`: Optional. Configuration file paths

**Example:**
```bash
batcave serve --repo ~/projects/my-repo --idle-seconds 3600
```

### Single-Instance Enforcement

BATMAN enforces exactly one runtime per repository via an `O_EXCL` lock file:

```
<state-dir>/runtime.lock
```

If a runtime is already running:
- The second instance exits with code 73 (`EX_TEMPFAIL`)
- Output includes the running runtime's identity as JSON

### Graceful Shutdown

Signal a running runtime to shut down:

```bash
batcave stop --repo /path/to/repo [--state-dir /path/to/state]
```

The runtime:
1. Refuses new connections
2. Waits for active runs to complete (with timeout)
3. Flushes event log
4. Removes the lock file
5. Exits cleanly

### Monitoring Events

Replay or subscribe to runtime events:

```bash
# Replay all events
batcave monitor --repo /path/to/repo

# Replay events for a specific run
batcave monitor --repo /path/to/repo --run-id <run-id>

# Subscribe to live events (until interrupted)
batcave monitor --repo /path/to/repo --live
```

## Doctor Command

Run diagnostic checks on the runtime:

```bash
batcave doctor --repo /path/to/repo [--state-dir /path/to/state] [--json]
```

**Output:**
- Plain text: Human-readable summary
- `--json`: Machine-readable JSON with `healthy`, `passed_checks`, `failed_checks`, `unresolved_gates`

**Example JSON output:**
```json
{
  "healthy": false,
  "failed_checks": [
    {
      "check_name": "database",
      "error": "failed to open database: No such file or directory"
    }
  ],
  "passed_checks": ["state_dir", "config"],
  "unresolved_gates": []
}
```

## Herdr Coordinated Restart

When BATMAN is running inside Herdr (the OMP terminal multiplexer), special care is needed for restarts:

### Automatic Restart

Herdr detects BATMAN crashes and automatically restarts the daemon:
- Preserves the repository and state directory
- Maintains the same Unix socket path
- Re-subscribes to event streams

### Manual Restart

To manually restart a Herdr-managed BATMAN:

1. **Stop the current runtime:**
   ```bash
   batcave stop --repo /path/to/repo
   ```

2. **Verify no processes remain:**
   ```bash
   ps aux | grep batcave
   ```

3. **Start a new runtime:**
   ```bash
   batcave serve --repo /path/to/repo
   ```

### Recovery from Crash

If BATMAN crashes unexpectedly:

1. **Check the log:**
   ```bash
   cat <state-dir>/runtime.log
   ```

2. **Check for orphaned processes:**
   ```bash
   ps aux | grep batcave
   ```

3. **Remove stale lock (if no process is running):**
   ```bash
   rm <state-dir>/runtime.lock
   ```

4. **Restart:**
   ```bash
   batcave serve --repo /path/to/repo
   ```

## Package Rollback

When updating BATMAN, you may need to roll back to a previous version:

### Rolling Back the Runtime

1. **Identify the target version:**
   ```bash
   batcave --version
   ```

2. **Install the previous version:**
   ```bash
   # From private registry
   bun install @satori/batman@<previous-version> --force
   
   # Or from source (see building from source below)
   ```

3. **Verify the rollback:**
   ```bash
   batcave --version
   batcave status --repo /path/to/repo
   ```

### Rolling Back the Extension

The OMP extension is version-locked to the runtime:

1. **Install the matching extension version:**
   ```bash
   omp install @satori/batman@<previous-version>
   ```

2. **Restart OMP** to load the rolled-back extension.

### Compatibility Matrix

Always ensure the extension and runtime versions match:

| Runtime Version | Extension Version | Status |
|----------------|-------------------|--------|
| 0.1.0 | 0.1.0 | Compatible |
| 0.1.0 | 0.2.0 | Incompatible (protocol mismatch) |

## Uninstall

To completely remove BATMAN:

### Removing the Runtime

```bash
# Remove the binary (if installed via package manager)
# macOS (Homebrew)
brew uninstall batman

# Linux (package manager specific)
# Debian/Ubuntu
sudo apt remove batman

# Remove state directory (optional, contains event log)
rm -rf ~/.batman

# Remove configuration (if any)
rm -rf ~/.config/batman
```

### Removing the Extension

```bash
# Uninstall from OMP
omp uninstall @satori/batman

# Or manually remove from OMP extensions directory
rm -rf ~/.omp/extensions/@satori/batman
```

### Cleaning Up

After uninstall, verify no BATMAN processes remain:

```bash
ps aux | grep batcave
# If any remain, kill them:
kill <pid>
```

## Upgrading

### Prerequisites

Before upgrading:

1. **Check the changelog** for breaking changes
2. **Back up your state directory** (contains event log):
   ```bash
   cp -r <state-dir> <state-dir>.backup
   ```
3. **Note your configuration** (org/repo/user config paths)

### Upgrade Steps

1. **Stop the current runtime:**
   ```bash
   batcave stop --repo /path/to/repo
   ```

2. **Update the package:**
   ```bash
   # From private registry
   bun update @satori/batman
   
   # Or from source (see building from source below)
   ```

3. **Verify the upgrade:**
   ```bash
   batcave --version
   batcave status --repo /path/to/repo
   ```

4. **Restart the runtime:**
   ```bash
   batcave serve --repo /path/to/repo
   ```

5. **Run the doctor command** to verify health:
   ```bash
   batcave doctor --repo /path/to/repo --json
   ```

### Post-Upgrade Checks

- [ ] Runtime starts successfully
- [ ] Event log is accessible
- [ ] Adapters pass conformance checks
- [ ] Configuration loads without errors
- [ ] Doctor command reports healthy (or expected failures)

## Troubleshooting

### Common Issues

**Runtime won't start:**
- Check for existing runtime: `batcave status --repo /path/to/repo`
- Remove stale lock: `rm <state-dir>/runtime.lock`
- Check logs: `cat <state-dir>/runtime.log`

**Doctor reports database error:**
- Ensure state directory exists: `ls -la <state-dir>`
- Check permissions: `ls -ld <state-dir>`
- Run doctor with verbose output

**Adapter conformance failures:**
- Run conformance tests: `bun test tests/conformance`
- Check adapter-specific logs
- Verify vendor CLI is installed and accessible

For more help, see [`TODO.md`](../TODO.md) or open an issue on GitHub.
