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
| CI-2 | `security` (gitleaks) | Open | 3 false-positive secret matches on test fixtures; no allowlist configured |
| CI-3 | `bundle-check` | Open | Committed `dist/index.js` wasn't rebuilt after `fast-uri` dependency bump |
| CI-4 | `test (macos-latest)` | Open | `duplicate_start_is_rejected` test is host-dependent; broke when slot-release-on-failure was fixed |
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

**Root cause:** there is no `.gitleaks.toml` anywhere in the repo, so gitleaks has no allowlist and
flags every string that matches its built-in rules, including deliberately secret-shaped test
fixtures for the redaction feature. This job has no history of passing — the gitleaks step has been
in `.github/workflows/ci.yml` since the workflow was first added (`f20abd3`), with no accompanying
allowlist ever added.

Separately (non-blocking, but dead configuration worth cleaning up in the same pass): the step
passes `with: args: --verbose`, which `gitleaks/gitleaks-action@v2` does not accept as an input —
the action logs `Unexpected input(s) 'args', valid inputs is ['']` and silently ignores it.

**Proposed fix:**
1. Add `.gitleaks.toml` at the repo root with a targeted allowlist (by file path + rule ID, or by
   regex matching the specific fixture strings) covering the three locations above. Prefer scoping
   to exact paths/lines over a blanket rule-ID disable, so a real secret using the same shape
   elsewhere still gets caught.
2. Remove the unsupported `args: --verbose` input from the `gitleaks/gitleaks-action@v2` step in
   `.github/workflows/ci.yml` (verbosity, if wanted, is controlled via the action's supported
   `GITLEAKS_CONFIG`/env inputs, not `args`).

**Status:** Open

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

**Proposed fix:** run `bun run build` for real on a clean checkout and commit the regenerated
`packages/extension/dist/index.js` in full (not a hand-patched version bump). Confirm with
`git diff --exit-code -- packages/extension/dist/index.js` after rebuilding, matching what the CI
step does.

**Status:** Open

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

**Proposed fix:** make the test deterministic instead of host-dependent — inject a fake
authorization/adapter double for this test that blocks on its first `start()` call until explicitly
released, so the "duplicate while running" condition is constructed directly rather than raced
against a real (and here, absent) `omp` binary. This does not require any change to
`crates/runtime/src/adapter/registry.rs` itself — the slot-release fix is correct and should stay as
is.

**Status:** Open

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
