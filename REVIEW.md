# BATMAN Codebase Review

**Reviewed:** 2026-08-06  
**Commit:** `3907e8fb8d31f5d275293a9e9302600d436cee44`

## Scope and method

The committed tree was split across four parallel reviews: runtime core; adapters/policy/security; TypeScript/OMP integration; and build/docs/release. Scout output was treated as raw input. Every Critical and High finding below was re-read against the cited source before inclusion. Unconfirmed leads and findings that were actually strengths were removed. The earlier `.git` symlink lead was rejected: TypeScript and Rust both test marker presence without dereferencing, and both have coverage for a broken `.git` symlink.

## Baseline

The reviewed code had the following verified baseline before this document was written:

- `cargo check --workspace --all-targets` — clean
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` — clean
- `cargo fmt --all --check` — clean
- `cargo test --workspace` — all suites passed
- `bun run generate --check` — generated artifacts current
- `bun run format:check` — clean
- `bun test packages` — 123 passed, 0 failed
- `bun test tests/conformance` — 10 passed, 0 failed

## Findings

### Critical

#### R1. Extension identity and task ownership can never match

**Location:** `packages/extension/src/runtime.ts:249-258`; `packages/extension/src/tools/tasks.ts:41-47`; `crates/runtime/src/approval/service.rs:145-159`; `crates/runtime/src/policy/violation.rs:487-525`

**Evidence:** The extension authenticates every runtime connection with the constant `instanceId: "batman-extension"`, but `batman_task upsert` records the OMP session ID as `ownerClientInstanceId`. Approval and violation decisions require exact equality between those values.

**Impact:** A session cannot decide approvals or release/cancel policy violations for tasks it created. Reconciliation can then rebind tasks to the shared constant, weakening isolation between OMP sessions using the same daemon.

**Resolution (2026-08-07):** ✅ Fixed, integration test pending. `EnsureRuntimeOptions` gains optional `sessionId` field; `initParams` uses it for `instanceId` when provided. `tryConnect`, `connectWithBackoff`, `ensureRuntime`, `getClient`, `statusContextFor`, and all tool call sites now thread `sessionId` from `extCtx.sessionManager.getSessionId()`. The status path gap was also fixed. Remaining: end-to-end extension test covering task upsert, approval decide, and violation decide.

#### R2. Concurrency slots are never released in production

**Location:** `crates/runtime/src/policy/evaluate.rs:271-275,371-401`; `crates/runtime/src/adapter/registry.rs:48-63,188-268`; `crates/runtime/src/lifecycle.rs:258-270`

**Evidence:** Each successful authorization increments `active_runs`. `PolicyEvaluator::release` is the only decrement path, but `PolicyEvaluator` is immediately erased behind `AdapterAuthorization`, whose trait has no release method. The adapter completion watcher removes and disposes the adapter without releasing the slot.

**Impact:** After `concurrency_ceiling` cumulative runs—not concurrent runs—the daemon rejects every new run until restart. Ordinary use permanently disables the runtime's core function.

**Resolution (2026-08-07):** ✅ Fixed. Added `release()` method to `AdapterAuthorization` trait. `FixtureAuthorization::release()` is a no-op; `PolicyEvaluator::release()` calls `decrement_runs()`. The adapter completion watcher clones `authorization` and calls `release()` after evicting the adapter. `run_one` releases the slot on all post-authorize error paths (availability probe, build_adapter, adapter.start). The watcher handles `Lagged` broadcast errors by continuing, releasing only on `Closed`. Defended with a real-`PolicyEvaluator` registry integration test (`releasing_a_policy_evaluator_slot_frees_the_registry_ceiling` in `crates/runtime/tests/adapter_registry.rs`) that books a `concurrency_ceiling: 1` slot, proves `registry.start()` denies a second run, releases through the trait object, and proves the ceiling denial clears.

#### R3. Linux ARM64 release builds lack a cross-linker

**Location:** `.github/workflows/release.yml:29-50`; `release/targets.json:5`; `Cargo.toml:18`

**Evidence:** `aarch64-unknown-linux-gnu` is built on an x86_64 `ubuntu-latest` runner. The workflow installs the Rust target only; it installs no AArch64 GNU compiler/linker and configures no target linker. The bundled SQLite dependency also requires a C compiler for the target.

**Impact:** The Linux ARM64 matrix leg cannot link, so `build` fails and blocks package assembly, conformance, and publish for every tagged release.

**Resolution (2026-08-07):** ✅ Fixed. Added `gcc-aarch64-linux-gnu` installation step for linux-arm64-gnu target. Set `CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER`, `CC_aarch64_unknown_linux_gnu`, and `AR_aarch64_unknown_linux_gnu` env vars. Added dry-run CI build workflow (`.github/workflows/ci-release.yml`) for every release target.

#### R4. GitHub artifact transfer strips the executable bit required by package validation

**Location:** `.github/workflows/release.yml:74-78,129-152,167-179`; `crates/xtask/src/main.rs:426-438,620-644`

**Evidence:** `xtask package` writes `bin/batcave` with mode `0755`, then `actions/upload-artifact` uploads the directory. The action documents that zipped artifact uploads restore files as `0644`. `xtask package-set` rejects any downloaded `bin/batcave` without an execute bit. No step restores permissions.

**Impact:** Even after R3 is fixed, every assembled package set fails before publish. Removing the assertion instead would publish non-executable binaries.

**Resolution (2026-08-07):** ✅ Fixed. Removed broken flatten loops from both `package-set` and `publish` jobs in release.yml. Both jobs now use `find ... -name batcave -exec chmod +x {} +` to restore executable bits after download. The `package-set` executable-mode assertion is preserved.

### High

#### R5. `humanRequired` approvals can be model-approved without a human

**Location:** `packages/extension/src/tools/approvals.ts:53-95`; `crates/runtime/src/approval/service.rs:145-216`; `crates/runtime/src/service/query.rs:243-255`

**Evidence:** The extension opens a human dialog only when `extCtx.hasUI` is true. In headless mode it falls through and sends the model-supplied decision. The server stores and returns `humanRequired` but does not enforce it in `ApprovalService::decide`, despite the tool description claiming server-side enforcement.

**Impact:** In non-interactive OMP modes, a model can approve an operation explicitly marked as requiring a human. This bypasses the safety property at the point where no human is likely to be watching.

**Suggested fix:** Enforce the invariant server-side with an authenticated human-decision channel/marker, and fail closed in the extension when a human-required approval is encountered without UI.

#### R6. A dead cached runtime client breaks all tools until status is called

**Location:** `packages/extension/src/index.ts:44-56`; `packages/extension/src/client.ts:135-167`; `packages/extension/src/status.ts:63-93`; `packages/extension/src/context.ts:14-19`

**Evidence:** `getClient` returns any defined cached client without checking whether its socket is closed. `BatmanClient` tracks closed state privately and rejects every later send. Only the status tool clears the cache after a failed request. The daemon has a routine 30-minute idle shutdown.

**Impact:** After daemon exit or a socket failure, all eleven orchestration tools and the monitor remain broken for the session. Running status happens to repair the cache; other tools cannot recover themselves.

**Suggested fix:** Expose client liveness, clear the shared cache on close/error, and make `getClient` reconnect when the cached client is closed. Add an idle-exit/reconnect integration test through a non-status tool.

#### R7. `run/retry` creates a queued run but never starts its adapter

**Location:** `crates/runtime/src/service/orchestration.rs:634-691,742-798`; `crates/runtime/src/recovery.rs:348-355`; `packages/extension/src/tools/runs.ts:24-30`

**Evidence:** `run_submit` constructs `RunDriverContext` and calls `driver.start`. `run_retry` only writes a fresh queued run and returns; it never calls `run_driver`, and no scheduler consumes queued runs. Recovery later converts queued runs to failed.

**Impact:** The extension reports retry success with a new run ID, but no work executes. The run remains queued until recovery marks it failed.

**Suggested fix:** Persist or require the prompt and route retry through the same authorization, workspace, display, and driver-start path as submit. Until then, do not describe retry as execution.

#### R8. The release conformance gate ignores aggregate failure

**Location:** `crates/runtime/src/conformance/report.rs:99-117`; `crates/runtime/src/cli.rs:693-717`; `tests/conformance/assert-report.ts:101-148`; `.github/workflows/release.yml:80-123,160-163`

**Evidence:** Reports compute `passed` as all scenarios passing. The CLI always exits success after writing reports, and the TypeScript validator never checks `report.passed`; it only requires one passing scenario and internally consistent capability downgrades.

**Impact:** A real canonical scenario can regress, produce `passed: false`, and still pass the release job because the failed capability is downgraded consistently.

**Suggested fix:** Reject any adapter report whose aggregate `passed` is not true, list failed scenarios, and make the CLI return a non-zero exit code for failed reports. Add a gate test with one failed canonical scenario.

#### R9. Release version checks do not validate the packages npm publishes

**Location:** `.github/workflows/release.yml:144-152,195-206`; `crates/xtask/src/main.rs:578-611`; `packages/extension/package.json:3`; `packages/batman-darwin-arm64/package.json:3` and peer leaf packages

**Evidence:** The workflow derives `--version` from `packages/extension/package.json`, and `package-set` compares it to the same file. Leaf manifests also derive from that file, but no check reads each leaf package's own npm `version`, and the tag is not compared to the release version.

**Impact:** A missed leaf version bump can pass package-set and then fail partway through sequential publishing, or publish an extension whose installed leaf version fails runtime integrity checks.

**Suggested fix:** Validate every leaf `package.json` version and the `v<version>` tag before building or publishing. Publish only after all package metadata passes as one set.

#### R10. Artifact APIs are project-scoped despite claiming task isolation

**Location:** `packages/extension/src/tools/artifacts.ts:25-42`; `crates/runtime/src/coordination/mcp_protocol.rs:133-156`; `crates/runtime/src/service/orchestration.rs:1136-1177`; `crates/runtime/src/workspace/artifact_store.rs:160-222`

**Evidence:** Both OMP and worker MCP descriptions say artifacts are limited to the current task. `artifact_list` forwards only an optional kind; the store lists every artifact in the repository daemon. `artifact_fetch` accepts any artifact ID in that store. Neither path filters by task, run, or principal.

**Impact:** One task can enumerate and read another task's patches, conflict reports, and workspace manifests. A model trusting the documented boundary can apply an unrelated patch to the working tree.

**Suggested fix:** Carry task scope from the authenticated principal/request into list and fetch, reject cross-task IDs, and add two-task isolation tests for both OMP and worker MCP callers.

#### R11. Copilot turn stop reasons are discarded

**Location:** `crates/runtime/src/adapter/copilot/client.rs:315-344`; `crates/runtime/src/adapter/copilot/mod.rs:422-426,462-489`; `crates/runtime/src/adapter/copilot/normalize.rs:18-89`

**Evidence:** `session_prompt` returns ACP's `stopReason`, but both initial and follow-up callers discard the returned string. The normalizer handles update notifications only and emits no terminal outcome for refusal, cancellation, token exhaustion, or maximum-turn termination.

**Impact:** Refused or limit-terminated turns are indistinguishable from successful completion to the journal, monitor, and retry/alerting automation.

**Suggested fix:** Map every supported stop reason to an explicit final/health event and treat non-success reasons as failures. Add fixture coverage for refusal, cancellation, and limit termination.

### Medium

#### R12. Claude error result subtypes are normalized as usage only

**Location:** `crates/runtime/src/adapter/claude/protocol.rs:183-194`; `crates/runtime/src/adapter/claude/normalize.rs:254-269`; `crates/runtime/tests/claude_adapter.rs:611-639`

`RawResult` omits `subtype` and `is_error`. The committed `error_max_turns` fixture therefore emits only `UsageReported`, with no diagnostic or unhealthy event. Model the failure discriminators and emit an explicit terminal failure.

#### R13. Policy cancellation records success after a process-kill failure

**Location:** `crates/runtime/src/policy/violation.rs:446-478`; `crates/runtime/src/adapter/registry.rs:308-321`

A failed adapter cancellation is logged, then the run is durably transitioned to `cancelled`. For cost-ceiling enforcement, the subprocess may continue spending while state says it stopped. Distinguish no-running-adapter from kill failure and persist/propagate genuine cancellation failure.

#### R14. Per-run redactor construction has a fail-open fallback

**Location:** `crates/runtime/src/lifecycle.rs:194-205`; `crates/runtime/src/adapter/event_sink.rs:152-168`

Startup correctly refuses invalid org regexes, but event-sink construction falls back to built-ins. Current wiring reuses the startup-validated list, so this path is not presently reachable with different patterns. Remove the trap by passing a prevalidated `Arc<Redactor>` or propagating construction failure.

#### R15. `batman_task.description` is silently discarded

**Location:** `packages/extension/src/tools/tasks.ts:20-47`; `crates/runtime/src/service/orchestration.rs:300-315`

The tool advertises task text but does not send it, and the RPC has no field for it. Remove the parameter and state that executable task text belongs in `run/submit.prompt`.

#### R16. Violation resolution schema accepts prose the runtime rejects

**Location:** `packages/extension/src/tools/violations.ts:14-24`; `crates/runtime/src/policy/violation.rs:487-497`

The tool accepts any string while the runtime accepts only `release` or `cancel`. Use a closed Zod enum.

#### R17. Generated TypeScript exports and hand-written enums can drift

**Location:** `packages/protocol-ts/src/index.ts:1-50`; `crates/xtask/src/main.rs:206-247`; `packages/extension/src/tools/workspaces.ts:20-24`; `packages/extension/src/tools/artifacts.ts:16-20`

The barrel claims to export every generated type but omits generated workspace/display enums, while tools hand-copy their literals. Export all generated modules and tie tool constants to generated types with `satisfies` checks.

#### R18. Detached runtime spawn has no `error` listener

**Location:** `packages/extension/src/runtime.ts:101-106`; `packages/extension/src/status.ts:57-72`

An asynchronous `ChildProcess` error such as `EAGAIN` or `EMFILE` has no listener and can escape the status path documented as never throwing. Attach an error listener and let the bounded connection loop return the sanitized failure.

#### R19. Role documentation understates the worker-accessible surface

**Location:** `docs/architecture.md:750-758`; `crates/runtime/src/ipc/mod.rs:226-300`

The document says 22 extension methods and 9 worker methods; code allows 30 and 12. It omits peer workspace and artifact list/fetch from the worker row. Generate or update the table from `allowed_methods`.

#### R20. Installed users cannot run the documented bare `batcave` commands

**Location:** `packages/batman-linux-x64-gnu/package.json:1-17` and peer leaves; `README.md:28-47`; `docs/operations.md:7-75`

Leaf packages export the binary for `import.meta.resolve` but declare no npm `bin` shim. The operations guide uses bare `batcave` throughout. Either publish a bin shim or document a supported binary-location command and use that path consistently.

#### R21. Documentation names CLI flags that do not exist

**Location:** `docs/getting-started.md:148,239,355,390`; `docs/code-walkthrough.md:396`; `docs/operations.md:66`; `crates/runtime/src/cli.rs:29-152`

`status --recover`, `serve --port`, and `monitor --live` are rejected by clap. Remove the flags and the TCP port troubleshooting text.

#### R22. Getting-started command examples omit required `--repo`

**Location:** `docs/getting-started.md:121-162`; `crates/runtime/src/cli.rs:29-165`; `docs/manual-testing.md:157`

Serve, status, stop, and audit examples omit a required argument. Add `--repo` to each example.

#### R23. Tool documentation describes eight tools while eleven are registered

**Location:** `docs/architecture.md:196-207`; `docs/code-walkthrough.md:124-135`; `docs/manual-testing.md:202`; `packages/extension/src/tools/index.ts:38-51`; `packages/extension/src/index.ts:99-103`

The docs omit artifact, child, violation, and the registered doctor surface. Update current-state docs; leave ADRs and journal history unchanged.

#### R24. Current docs name two deleted TypeScript modules

**Location:** `docs/code-walkthrough.md:125`; `docs/architecture.md:207`

`packages/extension/src/config.ts` and `packages/extension/src/conformance/index.ts` were removed. Delete or retarget these rows to the current config and `tests/conformance/run.ts` implementations.

#### R25. The current release checklist and compatibility guide retain disproven stub claims

**Location:** `release/0.1.0-checklist.json`; `docs/compatibility.md:189`; `crates/runtime/src/service/orchestration.rs:289-291`; `.github/workflows/ci.yml:23-37`

The checklist still says policy decisions and conformance are stubs, formatting is absent, and known tests fail. Mark it historical/superseded or regenerate it; remove the compatibility guide's policy stub label.

#### R26. Compatibility docs omit three shipped coordination methods

**Location:** `docs/compatibility.md:172-189`; `crates/protocol/src/method.rs:79-84`

Add `coordination/peerWorkspace`, `coordination/artifactList`, and `coordination/artifactFetch`.

#### R27. Uninstall and rollback instructions use nonexistent distribution channels

**Location:** `docs/operations.md:180-231`; `README.md:28-47`

The guide cites Homebrew, apt, `omp uninstall`, and the wrong plugin directory, while the repository ships only the npm/OMP plugin path. Rewrite rollback and uninstall around the supported private-registry package.

#### R28. Manual-testing guidance contradicts itself about CLI conformance

**Location:** `docs/manual-testing.md:341-342`; `crates/runtime/src/cli.rs:139-152`

Adjacent sentences say the subcommands are wired and that conformance is not available through a CLI. Delete the stale sentence.

### Low

#### R29. `workspaceMode` is an open string

**Location:** `packages/extension/src/tools/runs.ts:13-21`; `crates/runtime/src/service/orchestration.rs:649-656`

The runtime rejects unknown values safely, but a closed Zod enum would prevent avoidable round trips and align with the workspace tool.

#### R30. Local Bun scripts omit top-level conformance and install tests

**Location:** `package.json:10-13`; `.github/workflows/ci.yml:75-83`

`bun run test` and `bun run check` run `bun test packages`, while CI runs all Bun tests. Include `tests/` in local scripts so local success matches CI.

#### R31. CONTRIBUTING references nonexistent tests, features, and PR template

**Location:** `CONTRIBUTING.md:34,47,140`; `.github/`

Correct the colocated approval test path, remove feature-placeholder guidance, and either add a PR template or stop requiring one.

#### R32. The extension header lists only six of eleven orchestration tools

**Location:** `packages/extension/src/index.ts:1-4,83-99`

Update the current-source comment to match the registry.

## Strengths

- `DomainRepository::append_and_apply` keeps event append and projection updates in one SQLite transaction; migration ordering is sequential and atomic.
- Recovery completes before the runtime binds its socket, so clients cannot mutate pre-recovery state.
- Item 49 now exits subscription forwarders on writer closure, with a regression test that fails against the prior behavior.
- Binary selection verifies package version and SHA-256 before spawn.
- TypeScript and Rust derive repository IDs from the same fixtures, including worktree and broken-symlink cases.
- Inbound client frames are size-checked and schema-validated before dispatch.
- OMP-native reconciliation prevents stale coalesce timers from overwriting terminal facts and marks orphaned non-terminal runs as lost.
- The policy evaluator checks model, adapter, required-capability, nested-discovery, cost, and concurrency dimensions; the slot-release lifecycle is the defect captured in R2.
- Release targets and reproducible manifest timestamps have single sources of truth; package-set independently verifies target, checksum, schema fingerprint, and executable mode.
- Committed live conformance evidence matches the compatibility table: Claude 14/14, Codex 9/14, Copilot 11/14, OMP-RPC 14/14.

## Areas reviewed

- Rust runtime: IPC, services, domain repository, database migrations, recovery, lifecycle, workspace/artifacts, adapters, policy, security, supervisor, conformance, CLI, xtask.
- TypeScript: extension runtime/client/status, all orchestration tools, OMP-native persistence/reconciliation, generated protocol package, conformance/install tests.
- Delivery: CI/release workflows, target matrix, npm leaf packages, provenance/package-set logic.
- Current documentation: README, CONTRIBUTING, architecture, operations, getting started, compatibility, manual testing, and code walkthrough. ADRs and journal entries were treated as immutable point-in-time records rather than current-state docs.
