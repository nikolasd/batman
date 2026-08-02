# Adapter Registry Test Schema Fix Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix `crates/runtime/tests/adapter_registry.rs`'s `seed_worker_and_run` fixture helper so its raw SQL `INSERT` statements match the real `workers`/`runs` table schema, resolving all 5 currently-failing tests in that file (TODO.md item 1).

**Architecture:** Pure test-fixture correction — no production code changes. The helper's `INSERT INTO workers`/`INSERT INTO runs` statements reference columns that were never migrated (`workers.task_id`/`adapter_kind`/`profile_kind`/`status`; `runs.status`/`updated_at`), and omit two `NOT NULL` columns the real schema requires (`workers.project_id`, `workers.profile_id` — the latter a foreign key into `worker_profiles`, not `adapter_profiles`). Fix by aligning every `INSERT` to the real schema and adding one `worker_profiles` row per fixture to satisfy that foreign key.

**Tech Stack:** Rust, rusqlite, tokio::test.

## Global Constraints

- No production code changes. `crates/runtime/src/db/migrations.rs`'s schema is correct and already used successfully by other passing suites (`domain_repository.rs`, `orchestration_rpc.rs`, `coordination_mcp.rs`); this fix conforms the test fixture to it, never the reverse.
- `foreign_keys=ON` is enforced by `open_and_migrate`'s PRAGMA setup (`journal_mode=WAL`, `foreign_keys=ON`, `busy_timeout=5000`, `synchronous=FULL`) — every foreign key referenced by an `INSERT` must resolve to a real row already committed in the same transaction.
- Correcting TODO.md item 1's own implementation note: the foreign key is `workers.profile_id REFERENCES worker_profiles(id)`, **not** `adapter_profiles(id)` — `adapter_profiles` is an unrelated table (registered adapter profile configuration, migration 3) with no relationship to `workers.profile_id` at all. The satisfying row belongs in `worker_profiles`.
- The only production code this fixture feeds, `resolve_profile` (`crates/runtime/src/adapter/registry.rs:411`), calls `DomainRepository::resolved_profile_snapshot` (`crates/runtime/src/domain/repository.rs:296`), which executes exactly `SELECT resolved_profile_json FROM workers WHERE worker_id = ?1` — it never reads `profile_id`, joins `worker_profiles`, or reads any `runs` column. `profile_id`'s specific value is therefore irrelevant to test behavior as long as it satisfies the foreign key; `resolved_profile_json` is the only column whose *content* the code under test actually consumes.

---

### Task 1: Correct `seed_worker_and_run`'s schema mismatch

**Files:**
- Modify: `crates/runtime/tests/adapter_registry.rs:37-74`

**Interfaces:**
- Consumes: the real schema from `crates/runtime/src/db/migrations.rs` — `workers(worker_id, project_id, profile_id, parent_worker_id, created_at, resolved_profile_json)`, `worker_profiles(id, fingerprint, adapter, model, permission_envelope)`, `runs(run_id, task_id, worker_id, state, flags_*, vendor_session_id, created_at, started_at, completed_at)`, `tasks(task_id, project_id, owner_client_instance_id, revision, created_at, updated_at)` (already correct, unchanged).
- Produces: no new public interface — `seed_worker_and_run`'s signature (`async fn seed_worker_and_run(db: &Arc<DatabaseHandle>, project_id: ProjectId, profile: Option<&WorkerProfile>) -> (RunId, TaskId, WorkerId)`) is unchanged; only its SQL body changes. All 5 existing tests (`a_terminal_profile_uses_terminal_adapter`, `a_terminal_degraded_profile_uses_terminal_adapter`, `authorization_denial_prevents_the_adapter_from_ever_starting`, `duplicate_start_is_rejected`, `running_count_tracks_active_adapters`) call it unmodified and were verified to assert only on `AdapterRegistry::start`'s return value and `running_count()` — none query `worker_profiles`, `workers.profile_id`, or `runs.state` directly, so none require any other change.

- [ ] **Step 1: Confirm the exact current failure**

Run: `cargo test -p batman-runtime --test adapter_registry`

Expected: FAIL — 5 failed, 0 passed, each panicking at `adapter_registry.rs:72:6` with:
```
called `Result::unwrap()` on an `Err` value: Sqlite(SqliteFailure(Error { code: Unknown, extended_code: 1 }, Some("table workers has no column named task_id")))
```

- [ ] **Step 2: Rewrite `seed_worker_and_run`'s body**

Replace the entire function body (lines 41-74, from `let task_id = TaskId::new();` through the closing `}`) with:

```rust
    let task_id = TaskId::new();
    let worker_id = WorkerId::new();
    let run_id = RunId::new();
    let profile_row_id = WorkerId::new().to_string();
    let resolved_profile_json = profile.map(|p| serde_json::to_string(p).unwrap());
    db.run_domain_op(Box::new(move |conn| {
        conn.execute(
            "INSERT INTO tasks (task_id, project_id, owner_client_instance_id, revision, created_at, updated_at)
             VALUES (?1, ?2, ?3, 1, ?4, ?4)",
            rusqlite::params![task_id.to_string(), project_id.to_string(), "test-owner", "2026-01-01T00:00:00Z"],
        )?;
        conn.execute(
            "INSERT INTO worker_profiles (id, fingerprint, adapter, model, permission_envelope)
             VALUES (?1, 'sha256:test', 'fake', 'test-model', '{}')",
            rusqlite::params![profile_row_id],
        )?;
        conn.execute(
            "INSERT INTO workers (worker_id, project_id, profile_id, resolved_profile_json, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                worker_id.to_string(),
                project_id.to_string(),
                profile_row_id,
                resolved_profile_json,
                "2026-01-01T00:00:00Z",
            ],
        )?;
        conn.execute(
            "INSERT INTO runs (run_id, task_id, worker_id, state, created_at)
             VALUES (?1, ?2, ?3, 'queued', ?4)",
            rusqlite::params![run_id.to_string(), task_id.to_string(), worker_id.to_string(), "2026-01-01T00:00:00Z"],
        )?;
        Ok(serde_json::Value::Null)
    }))
    .await
    .unwrap();
    (run_id, task_id, worker_id)
```

Note what changed and why, precisely:
- `tasks` INSERT: **unchanged** — already matches the real schema exactly (verified column-for-column against `migrations.rs:43-50`).
- New: an `INSERT INTO worker_profiles` row using a throwaway unique id (`profile_row_id`, generated the same way the file already mints disposable unique strings elsewhere via `WorkerId::new().to_string()`) — required only to satisfy `workers.profile_id`'s `NOT NULL REFERENCES worker_profiles(id)` constraint under `foreign_keys=ON`. Its content (`fingerprint`/`adapter`/`model`/`permission_envelope`) is never read by any code this test exercises.
- `workers` INSERT: dropped `task_id`, `adapter_kind`, `profile_kind`, `status`, `updated_at` (none exist on this table); added the required `project_id` and `profile_id`; kept `resolved_profile_json` and `created_at` (both real, both already correct).
- `runs` INSERT: renamed `status` → `state` (the real column name) and dropped `updated_at` (doesn't exist on `runs` — only `started_at`/`completed_at` do, both nullable and correctly left unset here).

- [ ] **Step 3: Run the fixed test file**

Run: `cargo test -p batman-runtime --test adapter_registry`

Expected: PASS — `test result: ok. 5 passed; 0 failed`.

- [ ] **Step 4: Run the full crate test suite with `--no-fail-fast` to confirm no regression**

Run: `cargo test -p batman-runtime --no-fail-fast 2>&1 | grep -E "^test result"`

Expected: `adapter_registry` no longer appears among any failing binary. Only the failures already tracked and confirmed pre-existing/unrelated in TODO.md remain: item 6 (`conformance.rs` — missing `batcave conformance`/`adapters` CLI subcommands), item 7 (`claude_adapter.rs`/`codex_adapter.rs`/`copilot_adapter.rs` — `result_usage_artifacts` scenario gap), and item 24 (`copilot_adapter.rs` — installed CLI version not in the known-versions table). No suite that was passing before this change should start failing.

- [ ] **Step 5: Commit**

```bash
git add crates/runtime/tests/adapter_registry.rs
git commit -m "fix(test): align adapter_registry.rs fixture with the real workers/runs schema"
```

---

## Post-completion TODO.md update

Once Task 1's steps all pass, mark TODO.md item 1 as **Closed** (verified `<date>`) with the evidence: `cargo test -p batman-runtime --test adapter_registry` now reports 5/5 passing, where it previously reported 5 failing with a `SqliteFailure` at the shared fixture helper. Renumber the remaining items accordingly (item 1 moves to the Resolved section; items 2-27 shift up by one), following the same pattern used when item 1's predecessor (the `coordination-mcp` CLI gap) was closed.
