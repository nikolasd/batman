//! Regression tests for R74: `task/upsert` and `reconcile/omp` each split
//! their revision check from their write into two separate
//! `run_domain_op` round trips -- a caller-side pre-check that reads the
//! stored revision, then a write whose statement carries no revision
//! predicate of its own:
//!
//! - `OrchestrationService::task_upsert` (`crates/runtime/src/service/orchestration.rs:341-351`)
//!   reads `tasks.revision` and rejects a lower revision in memory, then
//!   calls `DomainRepository::upsert_task`
//!   (`crates/runtime/src/domain/repository.rs:310-357`), whose
//!   `INSERT ... ON CONFLICT(task_id) DO UPDATE` unconditionally
//!   overwrites `owner_client_instance_id`/`revision` -- its own doc
//!   comment says "a lower revision is rejected by the caller (service
//!   layer) before this point", i.e. nothing enforces that inside the
//!   write itself.
//! - `OrchestrationService::reconcile_omp` (`orchestration.rs:1850-1860`)
//!   reads `tasks.revision` and rejects a mismatched revision in memory,
//!   then calls `DomainRepository::reconcile_ownership`
//!   (`repository.rs:1254-1287`), whose `UPDATE tasks SET
//!   owner_client_instance_id = ?1, revision = ?2 ... WHERE task_id = ?4`
//!   has no `AND revision = ?` predicate either.
//!
//! `DatabaseHandle::run_domain_op` sends whole boxed closures to a
//! single-owner actor thread over a FIFO channel, one `oneshot` reply per
//! command (see `approval_decide_race.rs`'s header for the full
//! actor-FIFO argument): the actor never interleaves the *inside* of two
//! closures, only whole closures with each other, in enqueue order.
//! Because each pre-check and its write are two separate closures, two
//! concurrent callers can both enqueue their pre-check-read before either
//! enqueues its write, so both reads observe the same stale stored
//! revision and both pre-checks pass -- and then both writes land,
//! unconditionally, in whatever order the actor happens to process them.
//! A lower revision landing after a higher one moves the stored revision
//! backwards and silently rebinds the owner to a stale client; two
//! concurrent reconciles presenting the same revision both rebind instead
//! of exactly one winning.
//!
//! `service::query` is `pub(crate)` and `task_upsert`/`reconcile_omp` are
//! private methods on `OrchestrationService`, unreachable from an
//! integration test, so this file reproduces their two-round-trip shape
//! directly against the repo/db layers -- the same pattern
//! `approval_owner_race.rs`'s `rebind_owner` uses for
//! `reconcile_ownership` alone.
//!
//! Tests 1 and 2 pin the interleaving with `tokio::join!(biased; ...)`,
//! exactly as `approval_owner_race.rs` does, so the outcome does not
//! depend on real scheduler timing: `biased` polls the first-declared
//! future first on every wave, so both pre-check reads are enqueued --
//! and thus processed -- before either write, and the fix under test
//! (a revision predicate inside the write, checked in the same
//! transaction) is expected to make the outcome hold under every
//! ordering, not only the one `biased` pins. Test 3 is the sequential
//! guard: a stale upsert arriving strictly *after* a newer one, with no
//! concurrency at all, must stay refused both before and after the fix.

use batman_protocol::{ProjectId, TaskId, TaskRef};
use batman_runtime::db::DatabaseHandle;
use batman_runtime::domain::DomainRepository;
use serde_json::{Value, json};
use tempfile::TempDir;

async fn open_db() -> (TempDir, DatabaseHandle) {
    let state_dir = TempDir::new().unwrap();
    let db_path = state_dir.path().join("runtime.db");
    let db = DatabaseHandle::start(db_path).await.unwrap();
    (state_dir, db)
}

/// The task's current `(revision, owner_client_instance_id)`, or `None`
/// if it has never been upserted. A standalone read, not
/// `service::query::task_get_op` (`pub(crate)`, unreachable here).
async fn stored_task(db: &DatabaseHandle, task_id: TaskId) -> Option<(u64, String)> {
    db.run_domain_op(Box::new(move |conn| {
        let result: Result<(i64, String), rusqlite::Error> = conn.query_row(
            "SELECT revision, owner_client_instance_id FROM tasks WHERE task_id = ?1",
            [task_id.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        );
        match result {
            Ok((revision, owner)) => Ok(json!({ "revision": revision, "owner": owner })),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(Value::Null),
            Err(err) => Err(err.into()),
        }
    }))
    .await
    .expect("read stored task")
    .as_object()
    .map(|obj| {
        (
            obj["revision"].as_u64().expect("revision is a u64"),
            obj["owner"]
                .as_str()
                .expect("owner is a string")
                .to_string(),
        )
    })
}

/// Seeds a task at `revision`, owned by `owner`, via the direct repo
/// write -- bypassing every pre-check, exactly like
/// `approval_owner_race.rs`'s `seed_pending_approval`.
async fn seed_task(
    db: &DatabaseHandle,
    project_id: ProjectId,
    task_id: TaskId,
    owner: &str,
    revision: u64,
) {
    let owner = owner.to_string();
    db.run_domain_op(Box::new(move |conn| {
        let mut repo = DomainRepository::new(conn, project_id);
        repo.upsert_task(
            task_id,
            &TaskRef {
                owner_client_instance_id: owner,
                revision,
            },
        )
        .map(|_| json!({}))
    }))
    .await
    .expect("seed task");
}

/// Mirrors `OrchestrationService::task_upsert`'s two round trips: a
/// snapshot read of the stored revision (the caller-side pre-check,
/// `orchestration.rs:341-351`), then -- only if it passes -- the write
/// via `DomainRepository::upsert_task`.
async fn task_upsert_round_trips(
    db: &DatabaseHandle,
    project_id: ProjectId,
    task_id: TaskId,
    owner: &str,
    revision: u64,
) -> Result<(), String> {
    if let Some((stored_revision, _)) = stored_task(db, task_id).await {
        if revision < stored_revision {
            return Err(format!(
                "revision {revision} is lower than stored revision {stored_revision}"
            ));
        }
    }
    let task_ref = TaskRef {
        owner_client_instance_id: owner.to_string(),
        revision,
    };
    db.run_domain_op(Box::new(move |conn| {
        let mut repo = DomainRepository::new(conn, project_id);
        repo.upsert_task(task_id, &task_ref).map(|_| json!({}))
    }))
    .await
    .map_err(|err| err.to_string())?;
    Ok(())
}

/// Mirrors `OrchestrationService::reconcile_omp`'s two round trips: a
/// snapshot read of the stored revision (the exact-match pre-check,
/// `orchestration.rs:1850-1860`), then -- only if it matches -- the
/// write via `DomainRepository::reconcile_ownership`.
async fn reconcile_omp_round_trips(
    db: &DatabaseHandle,
    project_id: ProjectId,
    task_id: TaskId,
    new_owner: &str,
    revision: u64,
) -> Result<(), String> {
    let stored_revision = stored_task(db, task_id)
        .await
        .map(|(revision, _)| revision)
        .unwrap_or(0);
    if revision != stored_revision {
        return Err(format!(
            "revision {revision} does not match stored revision {stored_revision}"
        ));
    }
    let new_owner = new_owner.to_string();
    db.run_domain_op(Box::new(move |conn| {
        let mut repo = DomainRepository::new(conn, project_id);
        repo.reconcile_ownership(task_id, &new_owner, revision)
            .map(|_| json!({}))
    }))
    .await
    .map_err(|err| err.to_string())?;
    Ok(())
}

/// The number of journaled `ReconcileEvent`s across the whole events
/// table: `RuntimeEvent`'s `#[serde(tag = "type", rename_all =
/// "camelCase")]` renders the variant as `"type":"reconcileEvent"`, and
/// this file uses exactly one task per test, so a substring match is
/// unambiguous (mirrors `approval_owner_race.rs`'s
/// `decided_event_count`).
async fn reconcile_event_count(db: &DatabaseHandle) -> i64 {
    db.run_domain_op(Box::new(|conn| {
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM events WHERE event_json LIKE '%reconcileEvent%'",
            [],
            |row| row.get(0),
        )?;
        Ok(json!(count))
    }))
    .await
    .expect("count reconcile events")
    .as_i64()
    .expect("count is an integer")
}

/// RED: a lower revision must never land after -- and thus overwrite -- a
/// higher one, even when both callers' pre-checks raced against the same
/// stale stored revision. Seeds revision 3 (owner `omp-1`); two
/// concurrent `task/upsert`-shaped calls present revision 5 (owner
/// `omp-2`, declared first) and revision 4 (owner `omp-3`, declared
/// second). `biased` enqueues both reads before either write, so both
/// pre-checks read stored revision 3 and pass. Against today's
/// unconditional write, the actor processes the writes in enqueue order
/// (5 then 4), so revision 4's write lands last and wins -- the assertion
/// below fails, observing final revision 4 owned by `omp-3` instead of
/// the required revision 5 owned by `omp-2`.
#[tokio::test]
async fn concurrent_upserts_cannot_move_a_revision_backwards() {
    let (_state_dir, db) = open_db().await;
    let project_id = ProjectId::new();
    let task_id = TaskId::new();
    seed_task(&db, project_id, task_id, "omp-1", 3).await;

    let (higher, lower) = tokio::join!(
        biased;
        task_upsert_round_trips(&db, project_id, task_id, "omp-2", 5),
        task_upsert_round_trips(&db, project_id, task_id, "omp-3", 4),
    );

    assert!(higher.is_ok(), "revision 5's pre-check reads stored revision 3 and must pass: {higher:?}");
    assert!(
        lower.is_ok(),
        "revision 4's pre-check races the same stale stored revision 3 and must also pass \
         (that is the race under test, not a bug in this helper): {lower:?}"
    );

    let (final_revision, final_owner) = stored_task(&db, task_id).await.expect("task exists");
    assert_eq!(
        (final_revision, final_owner.as_str()),
        (5, "omp-2"),
        "the higher revision must win regardless of write order; a guarded write must refuse \
         the lower revision's write once it observes revision 5 is already stored"
    );
}

/// RED: two concurrent `reconcile/omp`-shaped calls presenting the same
/// revision must admit exactly one rebind, not both. Seeds revision 3
/// (owner `omp-1`); two concurrent reconciles from `omp-2` and `omp-3`
/// both present revision 3. `biased` enqueues both reads before either
/// write, so both pre-checks match and pass. Against today's unconditional
/// write, both succeed and both journal a `ReconcileEvent` -- the
/// assertions below fail, observing two successes and two events instead
/// of exactly one of each.
#[tokio::test]
async fn concurrent_reconciles_with_the_same_revision_admit_exactly_one_rebind() {
    let (_state_dir, db) = open_db().await;
    let project_id = ProjectId::new();
    let task_id = TaskId::new();
    seed_task(&db, project_id, task_id, "omp-1", 3).await;

    let (first, second) = tokio::join!(
        biased;
        reconcile_omp_round_trips(&db, project_id, task_id, "omp-2", 3),
        reconcile_omp_round_trips(&db, project_id, task_id, "omp-3", 3),
    );

    let successes = [&first, &second].into_iter().filter(|r| r.is_ok()).count();
    assert_eq!(
        successes, 1,
        "exactly one concurrent reconcile presenting the same revision must be admitted, \
         the other refused: first={first:?} second={second:?}"
    );
    assert_eq!(
        reconcile_event_count(&db).await,
        1,
        "exactly one ReconcileEvent must be journaled, not one per accepted write"
    );
}

/// GREEN guard: a stale revision arriving strictly *after* a newer one,
/// with no concurrency at all, is already refused by today's in-memory
/// pre-check and must stay refused once the fix moves that check inside
/// the guarded write.
#[tokio::test]
async fn a_stale_upsert_arriving_after_a_newer_one_is_refused_sequentially() {
    let (_state_dir, db) = open_db().await;
    let project_id = ProjectId::new();
    let task_id = TaskId::new();
    seed_task(&db, project_id, task_id, "omp-1", 1).await;

    task_upsert_round_trips(&db, project_id, task_id, "omp-2", 5)
        .await
        .expect("revision 5 must be accepted");

    let stale = task_upsert_round_trips(&db, project_id, task_id, "omp-3", 4).await;

    assert!(
        stale.is_err(),
        "a strictly sequential stale revision must stay refused: {stale:?}"
    );
    let (final_revision, final_owner) = stored_task(&db, task_id).await.expect("task exists");
    assert_eq!((final_revision, final_owner.as_str()), (5, "omp-2"));
}
