# Coordination-MCP CLI Subcommand Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the missing `batcave coordination-mcp` CLI subcommand so every worker adapter's already-configured MCP launch (`coordination_mcp_argv`) can actually reach the fully-built, fully-tested `batman_runtime::coordination::mcp::run` proxy instead of failing with clap's unrecognized-subcommand error.

**Architecture:** `cli.rs` gains one new `Command::CoordinationMcp` variant and one new `run_coordination_mcp` dispatch function that parses its three flags, resolves the state directory the same way every other subcommand does, and calls the existing, unmodified `batman_runtime::coordination::mcp::run(...)` — all real behavior (socket auth, MCP proxying) already lives in that function; this task only wires the binary's argument parser to it.

**Tech Stack:** Rust, `clap` derive macros (`Subcommand`), `batman_protocol::RunId`, `batman_runtime::coordination::mcp`.

## Global Constraints

- `cli.rs` only parses arguments, resolves paths, and maps library outcomes to process exit codes — no proxy/protocol behavior is added here; it all already lives in `batman_runtime::coordination::mcp::run` (per `cli.rs`'s own module doc, unchanged).
- The subcommand's flags must be exactly `--state-dir <path>` (optional), `--repo <path>` (required), `--run-id <string>` (required) — this exact shape is already asserted by `mcp_config.rs::coordination_mcp_argv`'s tests and consumed by `crates/runtime/tests/coordination_mcp.rs`'s subprocess harness (`launch`/`spawn_without_scope_token`). Renaming or reordering breaks both without any change to those files.
- No new test file: the full behavior contract (7-tool listing, real broker fulfillment, smuggled-sender rejection, missing/expired/mismatched/revoked token rejection, live-vendor reconnect) is already codified in `crates/runtime/tests/coordination_mcp.rs`'s 9 tests. This task's only job is to make the compiled binary satisfy them.
- Never persist or log the scope token — already handled entirely inside the unmodified `ProcessEnvironment`/`mcp::run`; this task must not read `BATMAN_WORKER_SCOPE_TOKEN` itself.

---

### Task 1: Wire `coordination-mcp` to the existing MCP proxy

**Files:**
- Modify: `crates/runtime/src/cli.rs`
- Modify: `crates/runtime/src/main.rs:5` (stale module-doc command list — drive-by, one line)
- Test: `crates/runtime/tests/coordination_mcp.rs` (already exists, unmodified — this task makes it pass)

**Interfaces:**
- Consumes: `batman_protocol::RunId::parse(&str) -> Result<RunId, uuid::Error>`; `batman_runtime::coordination::mcp::run(state_dir: &Path, repo: &Path, run_id: RunId, token_source: &dyn ScopeTokenSource) -> Result<(), McpProxyError>`; `batman_runtime::coordination::mcp::ProcessEnvironment` (unit struct implementing `ScopeTokenSource`); the existing `resolve_state_dir(Option<PathBuf>) -> Result<PathBuf, String>` and `fail(&dyn std::fmt::Display) -> ExitCode` helpers already in `cli.rs`.
- Produces: `Command::CoordinationMcp { state_dir: Option<PathBuf>, repo: PathBuf, run_id: String }` variant and `async fn run_coordination_mcp(state_dir: Option<PathBuf>, repo: PathBuf, run_id: String) -> ExitCode`. No other task depends on these names — this is the terminal consumer of the existing `mcp::run` seam.

- [ ] **Step 1: Confirm the existing test file currently fails as expected**

Run: `cargo test -p batman-runtime --test coordination_mcp`

Expected: 5 pass, 4 FAIL with `panicked at ... coordination-mcp closed the connection before responding` (`coordination_mcp_lists_all_seven_tools`, `coordination_mcp_fulfills_batman_task_and_batman_send_against_the_real_broker`, `coordination_mcp_rejects_a_smuggled_sender_worker_id_over_real_stdio`, `coordination_mcp_descendant_may_reconnect_with_the_same_token_while_the_vendor_lives`). The 5 that currently pass (`..._fails_fast_with_no_scope_token_at_all`, `..._rejects_an_expired_token`, `..._rejects_a_run_id_the_token_is_not_bound_to`, `..._rejects_an_unrelated_or_already_exited_vendor_pid`, `..._rejects_after_the_real_vendor_exits_and_is_revoked`) only assert `!status.success()` — right now that's a **false pass**: clap's unrecognized-subcommand error also exits non-zero, coincidentally matching the assertion for the wrong reason. Step 6 re-verifies these are still failing, now for the *right* reason.

- [ ] **Step 2: Add the `CoordinationMcp` variant to `Command`**

In `crates/runtime/src/cli.rs`, add this variant as the last entry inside `enum Command { ... }` (immediately after the `Doctor { ... }` variant, before the closing `}` at line 104):

```rust
    /// Serve the worker-coordination MCP proxy for one run over stdio.
    CoordinationMcp {
        /// The BATMAN state root. Defaults to the resolved state root.
        #[arg(long)]
        state_dir: Option<PathBuf>,
        /// The repository this run belongs to.
        #[arg(long)]
        repo: PathBuf,
        /// The run this MCP proxy is scoped to.
        #[arg(long)]
        run_id: String,
    },
```

- [ ] **Step 3: Add the dispatch arm in `run()`**

In the same file's `match cli.command { ... }` block (inside `pub async fn run() -> ExitCode`), add this arm immediately after the existing `Command::Doctor { ... } => run_doctor(state_dir, repo, json).await,` arm, before the closing `}` at line 184:

```rust
        Command::CoordinationMcp {
            state_dir,
            repo,
            run_id,
        } => run_coordination_mcp(state_dir, repo, run_id).await,
```

- [ ] **Step 4: Implement `run_coordination_mcp`**

Add this function after `run_doctor` (which ends at line 476) and before the `fail` helper (which starts at line 477):

```rust
/// Runs `batcave coordination-mcp`: proxies MCP `initialize`/`tools/list`/
/// `tools/call` on stdio to the worker coordination tools over the
/// runtime socket, authenticated with `BATMAN_WORKER_SCOPE_TOKEN` read
/// from (and removed from) this process's own inherited environment. All
/// protocol/auth behavior lives in `batman_runtime::coordination::mcp`;
/// this function only resolves CLI arguments into that call.
async fn run_coordination_mcp(state_dir: Option<PathBuf>, repo: PathBuf, run_id: String) -> ExitCode {
    use batman_protocol::RunId;
    use batman_runtime::coordination::mcp::{self, ProcessEnvironment};

    let state_dir = match resolve_state_dir(state_dir) {
        Ok(dir) => dir,
        Err(err) => return fail(&err),
    };
    let run_id = match RunId::parse(&run_id) {
        Ok(id) => id,
        Err(err) => return fail(&err),
    };

    match mcp::run(&state_dir, &repo, run_id, &ProcessEnvironment).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => fail(&err),
    }
}
```

- [ ] **Step 5: Fix the two stale module-doc command lists this change makes worse**

In `crates/runtime/src/cli.rs`, line 1-2 currently reads:

```rust
//! The `batcave` command-line interface: `serve`, `status`, `stop`,
//! `version`, `schema`, `monitor`, and `audit`. This layer only
```

Replace with:

```rust
//! The `batcave` command-line interface: `serve`, `status`, `stop`,
//! `version`, `schema`, `monitor`, `audit`, `doctor`, and
//! `coordination-mcp`. This layer only
```

In `crates/runtime/src/main.rs`, line 5 currently reads:

```rust
//! Commands: `serve`, `status`, `stop`, `version`, and `schema`.
```

Replace with:

```rust
//! Commands: `serve`, `status`, `stop`, `version`, `schema`, `monitor`,
//! `audit`, `doctor`, and `coordination-mcp`.
```

- [ ] **Step 6: Run the existing integration test file and verify all 9 pass for the right reason**

Run: `cargo test -p batman-runtime --test coordination_mcp -- --nocapture`

Expected: `test result: ok. 9 passed; 0 failed`. Specifically confirm:
- The 4 previously-EOF-panicking tests now pass because `coordination-mcp` actually serves stdio (`coordination_mcp_lists_all_seven_tools`, `coordination_mcp_fulfills_batman_task_and_batman_send_against_the_real_broker`, `coordination_mcp_rejects_a_smuggled_sender_worker_id_over_real_stdio`, `coordination_mcp_descendant_may_reconnect_with_the_same_token_while_the_vendor_lives`).
- The 5 previously-coincidentally-passing tests still pass — now for the real reason (a genuine `McpProxyError` exit, not a clap parse error). Spot-check one directly: `cargo test -p batman-runtime --test coordination_mcp coordination_mcp_fails_fast_with_no_scope_token_at_all -- --nocapture` and confirm the subprocess's stderr (visible with `--nocapture`) shows `BATMAN_WORKER_SCOPE_TOKEN is not set in the environment` rather than a clap usage/error message.

- [ ] **Step 7: Run the full workspace test suite to confirm no regression**

Run: `cargo test --workspace 2>&1 | grep -E "^test result"`

Expected: every suite reports `ok` except the pre-existing, already-tracked `adapter_registry` failure (5 failed — TODO.md item 2, a `workers` table schema mismatch unrelated to this change). No suite that previously passed should now fail.

- [ ] **Step 8: Commit**

```bash
git add crates/runtime/src/cli.rs crates/runtime/src/main.rs
git commit -m "feat(cli): wire coordination-mcp subcommand to the existing MCP proxy"
```

---

## Post-completion TODO.md update

Once Task 1's steps all pass, mark TODO.md item 1 as **Closed** (verified `<date>`) with the evidence: `cargo test -p batman-runtime --test coordination_mcp` now reports 9/9 passing, where it previously reported 4 failing (EOF) and 5 coincidentally-passing (clap-rejection false positives).
