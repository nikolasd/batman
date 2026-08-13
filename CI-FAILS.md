# CI Failures — main CI run 31595771218

**Source run:** https://github.com/nikolasd/batman/actions/runs/31595771218
**Triggered by:** push to `main`, commit `e65a5a5` ("docs: restructure REVIEW.md as open-items-only backlog...")
**Investigated:** 2026-08-12

**Purpose:** track each independent CI failure from this run through to resolution. Each item below
was traced to a root cause (not just a symptom) before a fix was proposed. Items are unrelated to
each other — they are fixed and verified one at a time, not batched.

## Summary

| ID | Job | Status | One-line cause |
|----|-----|--------|-----------------|
| CI-1 | `clippy` | Fixed | `useless_conversion` lint: `u64::from()` widening is a no-op on Linux (where clippy runs) but required on macOS; suppressed with `#[allow]` instead of removed |
| CI-2 | `security` (gitleaks) | Fixed | Shallow clone made gitleaks re-scan the whole repo as "new" on every push; no allowlist configured either |
| CI-3 | `bundle-check` | Fixed | Committed `dist/index.js` wasn't rebuilt after `fast-uri` dependency bump |
| CI-4 | `test (macos-latest)` | Fixed | `duplicate_start_is_rejected` test is host-dependent; broke when slot-release-on-failure was fixed |
| CI-5 | `test (ubuntu-latest)` | Open | Cancelled as a side effect of CI-4 (matrix `fail-fast: true`); not independent |

---

## CI-1. `clippy` — useless conversion in `doctor.rs`

**Job:** `clippy` · step "Run Clippy with warnings as errors" · exit code 101

**Location:** `crates/runtime/src/doctor.rs:462`

**Evidence:**
```rust
// `blocks_available()` is platform-dependent in width; `fragment_size()`
// is already `u64`, so only the former needs widening.
let free = u64::from(stat.blocks_available()) * stat.fragment_size();
```
```
error: useless conversion to the same type: `u64`
   --> crates/runtime/src/doctor.rs:462:20
    |
462 |         let free = u64::from(stat.blocks_available()) * stat.fragment_size();
    |                    ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ help: consider removing `u64::from()`: `stat.blocks_available()`
    |
    = note: `-D clippy::useless-conversion` implied by `-D warnings`
```

**Root cause:** the original comment was actually correct, not vague — `nix`'s
`Statvfs::blocks_available()` returns `fsblkcnt_t`, which really is narrower than `u64` on some
platforms this workspace supports. Verified per-platform against `nix` 0.30.1 / `libc` type
definitions for the full support matrix (macOS + glibc Linux, arm64/x64 — the invariant in
`CLAUDE.md`):

| Platform | `blocks_available()` → `fsblkcnt_t` | `fragment_size()` → `c_ulong` |
|---|---|---|
| `x86_64-apple-darwin` | `c_uint` = `u32` | `u64` |
| `aarch64-apple-darwin` | `c_uint` = `u32` | `u64` |
| `x86_64-unknown-linux-gnu` | `u64` | `u64` |
| `aarch64-unknown-linux-gnu` | `u64` | `u64` |

The widening is required on macOS to avoid a `u32 * u64` type mismatch, and is a genuine no-op only
on Linux — which is exactly the one platform the `clippy` CI job runs on (`ubuntu-latest`). So
`clippy::useless_conversion` is correct about Linux and would be wrong to appease by deleting the
conversion, which would break the macOS build.

**Fix applied:** keep the widening, suppress the lint locally with
`#[allow(clippy::useless_conversion)]`, and rewrite the comment to state the concrete per-platform
types instead of the vaguer original wording:
```rust
// `blocks_available()` returns `fsblkcnt_t`, which is `u32` on macOS/Darwin
// (x86_64 and aarch64) but already `u64` on glibc Linux (x86_64 and aarch64) —
// the only platforms this workspace targets. `fragment_size()` returns
// `c_ulong`, which is `u64` on all four. The widening below is required on
// macOS and a no-op on Linux; the allow covers that Linux no-op.
#[allow(clippy::useless_conversion)]
let free = u64::from(stat.blocks_available()) * stat.fragment_size();
```
This matches existing repo style: other `#[allow(clippy::...)]` annotations already cover
known-intentional lint triggers elsewhere (e.g. `crates/runtime/src/coordination/broker.rs:182`,
`crates/runtime/src/adapter/copilot/client.rs:189`).

**Status:** Fixed — verified locally with `cargo clippy --workspace --all-targets --all-features -- -D warnings`
(macOS side) and `cargo fmt --all --check`; `cargo test --workspace` run as a regression sanity
check. The Linux side of the `#[allow]` (where it is load-bearing) is confirmed once pushed and the
`clippy` GitHub Actions job goes green.

---

## CI-2. `security` (gitleaks) — false-positive secret matches, no allowlist

**Job:** `security` · step "Scan for secrets" · exit code 2 ("🛑 Leaks detected")

**Findings (all three are fabricated strings inside tests of the redaction/security-rule feature
itself, not leaked credentials):**

| RuleID | File | Line | Context |
|---|---|---|---|
| `aws-access-token` | `crates/runtime/src/security/rules.rs` | 162 | `test_org_rule_applies_correctly` — fixture text asserting a fake AWS-shaped key gets redacted |
| `generic-api-key` | `crates/runtime/tests/adapter_contract.rs` | 503 | `fixture_profile_rejects_secret_shaped_permission_envelope` — fake `sk-...` key used to prove secret-shaped fields are rejected |
| `generic-api-key` | `crates/runtime/tests/redaction.rs` | 8 | `redactor_removes_api_keys` — fake `sk-...` key used to prove the redactor strips it |

**Root cause:** there is no `.gitleaks.toml` anywhere in the repo (true, but incomplete — see below),
*and* the missing allowlist isn't even the reason this job has never once passed.

Reading `gitleaks/gitleaks-action`'s actual source (`src/gitleaks.js`), not just its docs: for a
same-ref push it runs

```
gitleaks detect --redact -v --exit-code=2 --report-format=sarif --report-path=results.sarif \
  --log-level=debug --log-opts=-1
```

`--log-opts=-1` scans only the tip commit's diff (`git log -1 -p`). `.github/workflows/ci.yml`'s
`security` job checked out with plain `actions/checkout@v4` — no `fetch-depth` override, so it was a
**shallow clone at depth 1**. In a depth-1 clone the tip commit has no accessible parent, so `git log
-1 -p` can't diff against a real parent — git treats the entire tree as if every line were newly
added *in that one commit*. That made gitleaks re-scan the whole repository as "brand new" on
**every single push**, regardless of what the push actually touched — which is exactly why this job
has no history of ever passing, and why 3 old, unrelated, long-since-committed test fixtures showed
up as "leaks" on a push (`e65a5a5`) that only touched `REVIEW.md`.

`gitleaks-action`'s own README recommends `fetch-depth: 0` in the checkout step preceding it, for
exactly this reason. Fixing only the allowlist would make a given push go green but leave the job
re-scanning the entire repo as "new" forever, breaking again the moment any latent secret-shaped
string exists anywhere in the codebase; fixing only `fetch-depth` would make routine pushes
correctly diff-scoped but leaves no defense if these exact fixture lines are ever touched again by
an unrelated change, with no allowlist to match against. Both were needed.

Separately (non-blocking, but dead configuration cleaned up in the same pass): the step passed
`with: args: --verbose`. Confirmed via `gh api repos/gitleaks/gitleaks-action/contents/action.yml`
that the action declares **zero inputs** (it's a Node action, `runs: main: dist/index.js`) — `args`
was never read, and the action already hardcodes `-v` itself.

One more thing verified locally (`gitleaks` 8.30.1 installed on this machine): the current default
`aws-access-token` rule's regex charset is base32 (`[A-Z2-7]`), which the `rules.rs:162` fixture
(`AKIA1234567890ABCDEF`, containing `0/1/8/9`) no longer matches at all — that specific finding may
already be moot under whatever "latest" gitleaks the action resolves at CI run time (the workflow
pins no `GITLEAKS_VERSION`). Allowlisted anyway as cheap, harmless insurance against the ruleset
reverting or the pinned version differing.

**Fix applied:**
1. `.github/workflows/ci.yml`: gave the `security` job's checkout `fetch-depth: 0` (only that job's
   checkout — the other 5 jobs don't run gitleaks and have no reason to pay for a deeper clone).
2. Added `.gitleaks.toml` at the repo root: extends (not replaces) the default ruleset, with a
   path + exact-literal (`condition = "AND"`) allowlist entry per finding, covering the three
   locations above. Validated locally with `gitleaks detect --no-git --config .gitleaks.toml -v`
   that the findings disappear, and that a planted decoy with the identical secret text in a
   *different* file still gets flagged — the scoping doesn't over-match.
3. Removed the dead `args: --verbose` input from the `gitleaks/gitleaks-action@v2` step.

**Status:** Fixed — verified locally with `gitleaks detect --no-git --source . --config
.gitleaks.toml -v` ("no leaks found") and the over-match sanity check above. Final confirmation
(shallow-clone diff semantics and the action's resolved "latest" gitleaks version can only be
proven on GitHub's runner) is the `security` job going green on the push — same caveat pattern as
CI-1.

---

## CI-3. `bundle-check` — committed extension bundle is stale

**Job:** `bundle-check` · step "Fail if the committed bundle is stale" · exit code 1

**Evidence:** a clean `bun install && bun run build` produces a `packages/extension/dist/index.js`
that differs from the committed one. The diff is entirely inside the vendored `fast-uri` code path
bundled via `ajv`: the freshly built bundle contains a new `AUTHORITY_INTRODUCER_REGION` URI-authority
validation block that the committed bundle lacks.

**Root cause:** `bun.lock` bumped `fast-uri` `3.1.4` → `3.1.5` in commit `38c8c3f` ("chore(tooling):
add Biome formatter and CI format gate", 2026-08-06). The dist bundle was not fully rebuilt against
that lockfile change afterward. The most recent commit that touched
`packages/extension/dist/index.js` (`3f7014d`, "chore: bump version to 0.3.0", 2026-08-12) changed
only 4 lines — the embedded version string — not a full rebuild, so the bundle still reflects the
older `fast-uri@3.1.4` output while `bun.lock` already points at `3.1.5`.

**Fix applied:** ran `bun install && bun run build` for real and committed the regenerated
`packages/extension/dist/index.js` in full (not a hand-patched version bump). The resulting diff is
localized entirely to the vendored `fast-uri`/`ajv` region: three `require_...` source-comment path
updates (`fast-uri@3.1.4` → `fast-uri@3.1.5`) plus the new `AUTHORITY_INTRODUCER_REGION` URI-authority
validation block and its call site in `resolve()`, exactly as anticipated above.

**Status:** Fixed — verified locally by confirming `git diff --exit-code -- packages/extension/dist/index.js`
exited non-zero (a diff existed to commit) before committing, and exits `0` after — the literal
`bundle-check` CI step. `bun test packages/extension` run as a regression sanity check against the
rebuilt bundle. Final confirmation is the `bundle-check` job going green on push — same caveat
pattern as CI-1/CI-2.

---

## CI-4. `test (macos-latest)` — `duplicate_start_is_rejected` is host-dependent

**Job:** `test (macos-latest)` · step "Run Rust tests" · exit code 101

**Location:** `crates/runtime/tests/adapter_registry.rs:232-259`

**Evidence:**
```
thread 'duplicate_start_is_rejected' (15969) panicked at crates/runtime/tests/adapter_registry.rs:253:5:
unexpected error message: adapter ompRpc operation start failed (process): failed to spawn "omp": No such file or directory (os error 2)
```
The test's own comment anticipates partial host-dependence:
```rust
// First start should succeed (or fail based on host, but not with "duplicate" error)
let _result1 = registry.start(ctx(...)).await;

// Second start with same worker_id should fail with "duplicate" error
let err = registry.start(ctx(...)).await.expect_err("duplicate start must be rejected");
assert!(err.contains("already has a running adapter instance"), ...);
```

**Root cause:** commit `86244da` ("fix(runtime): release concurrency slots on adapter settlement")
correctly made `run_one` release the registry slot on every post-authorize failure path, including
`adapter.start()` failure — closing a real slot-leak bug. On a runner without `omp` on `PATH` (true
for both GitHub-hosted runners here), the *first* `start()` call now fails immediately with `ENOENT`
and its slot is released before the *second* `start()` call executes. The second call therefore also
fails with the same `ENOENT`, instead of hitting the intended duplicate-detection path the test
asserts on. `BATMAN_DISABLE_VENDOR_CLI=1` is set for this job but doesn't cover the OMP-RPC adapter's
spawn path used by this specific test harness.

In other words: this test's assertion on "duplicate" detection was only ever incidentally true when
`omp` happened to be present on the host running it (so the first `start()` stayed "running" long
enough for the second call to collide with it) — it was never a reliable test of duplicate-detection
under CI's real conditions, and the recent (correct) slot-release fix exposed that.

**Fix applied:** made the test deterministic instead of host-dependent. `AdapterRegistry`'s one
production dependency-injection seam is `AdapterAuthorization` (`registry.rs`'s own
`FixtureAuthorization`/`CountingAuthorization` are prior test doubles for it); added a third,
`BlockingAuthorization`, local to `adapter_registry.rs`. Its `authorize()` blocks the calling thread
until the test explicitly releases it. `authorize()` runs synchronously *after*
`AdapterRegistry::start` has already inserted the run's reservation into its `running` map and
*before* the real adapter is constructed or spawned, so blocking there deterministically holds that
reservation open for as long as the test needs, regardless of whether `omp` is installed. The first
`start()` now runs on its own `tokio::spawn`ed task; the test waits for a signal that it has reached
`authorize()` (guaranteeing the reservation is held), fires the second `start()` for the same
`run_id`, asserts the duplicate rejection deterministically, then releases the first call and lets it
finish (its own eventual outcome no longer matters to the assertions). Blocking a thread inside an
`async fn` needs a second worker thread to make progress on anything else, so this one test switches
to `#[tokio::test(flavor = "multi_thread", worker_threads = 2)]` (`rt-multi-thread` is already an
enabled tokio feature workspace-wide; no dependency change needed). This requires zero changes to
`crates/runtime/src/adapter/registry.rs` — the slot-release fix (`86244da`) is correct and untouched;
only the test file changed.

An alternative was considered and rejected: the repo already has a genuine controllable fake for the
OMP-RPC wire protocol (`crates/fake-worker`'s `omp-rpc-host-tool` mode, driven through
`OmpRpcAdapter::with_binary(...)` — see `tests/omp_rpc_adapter.rs`), which blocks mid-`prompt` until
fed a response on stdin. It would exercise a more realistic "genuinely still running" condition, but
`AdapterRegistry::build_adapter` hardcodes `OmpRpcAdapter::new` with no seam to substitute
`with_binary`'s alternate binary path — using it would require adding a test-only override to
`registry.rs` itself, which the `AdapterAuthorization`-blocking approach avoids entirely.

**Status:** Fixed — verified locally in both directions: `cargo test --test adapter_registry
duplicate_start_is_rejected` passes with the real `omp` on `PATH` (12s, since the first call's
adapter genuinely probes/spawns it), *and* passes running the already-built test binary directly
with `omp`'s directory stripped from `PATH` (0.02s, `ENOENT` exactly as on both CI runners) —
reproducing the CI condition on this machine and confirming the fix no longer depends on host
`omp` availability either way. Full file (`cargo test --test adapter_registry`, 6/6) and
`cargo test --workspace` (all green, one unrelated pre-existing local-only failure in
`copilot_adapter.rs` excluded — an installed Copilot CLI version not yet in that test's hardcoded
known-versions list, confirmed via `git stash` to predate this change and unconnected to it) run as
regression sanity checks, plus `cargo clippy --workspace --all-targets --all-features -- -D warnings`
and `cargo fmt --all --check`. Final confirmation is the `test (macos-latest)` job going green on
push — same caveat pattern as CI-1/2/3.

---

## CI-5. `test (ubuntu-latest)` — cancelled, not an independent failure

**Job:** `test (ubuntu-latest)` · annotation: "The strategy configuration was canceled because
`test.macos-latest` failed" / "The operation was canceled."

**Root cause:** the `test` job's matrix (`.github/workflows/ci.yml:52-56`, `os: [ubuntu-latest,
macos-latest]`) has no `fail-fast: false`, so it defaults to `fail-fast: true`. When
`test (macos-latest)` failed (CI-4), GitHub Actions cancelled the sibling `test (ubuntu-latest)` job
outright. There is no ubuntu-specific defect here.

**Proposed fix:** no code fix needed — this resolves automatically once CI-4 is fixed and the
matrix run completes normally. Separately worth considering: add `fail-fast: false` to this matrix so
a failure on one platform doesn't hide whether the other platform would have passed or failed on its
own merits (useful signal when debugging exactly this kind of situation).

**Status:** Open (tracked for visibility; no independent action beyond CI-4, and the optional
`fail-fast: false` follow-up)
