//! Opens the runtime's SQLite database privately, configures its PRAGMAs,
//! and applies its schema migrations atomically.

use std::path::Path;

use rusqlite::Connection;
use rusqlite_migration::{M, Migrations};

use super::actor::DbError;

/// Migration 1: the durable event journal and the operation-intent table.
const MIGRATION_1: &str = "
CREATE TABLE events (
  sequence INTEGER PRIMARY KEY,
  timestamp TEXT NOT NULL,
  project_id TEXT NOT NULL,
  run_id TEXT,
  event_json TEXT NOT NULL
);
CREATE TABLE operations (
  operation_id TEXT PRIMARY KEY,
  kind TEXT NOT NULL,
  intent_json TEXT NOT NULL,
  requested_at TEXT NOT NULL,
  acknowledged_at TEXT,
  acknowledgement_json TEXT
);
";

/// Migration 2: orchestration projections (tasks, workers, worker
/// profiles, runs, messages, approvals). Kept in a normalized shape
/// alongside the append-only `events` journal; every mutation goes
/// through one transaction that appends an event and updates the
/// relevant projection row(s).
const MIGRATION_2: &str = "
CREATE TABLE worker_profiles (
  id TEXT PRIMARY KEY,
  fingerprint TEXT NOT NULL,
  adapter TEXT NOT NULL,
  model TEXT NOT NULL,
  permission_envelope TEXT NOT NULL
);
CREATE TABLE tasks (
  task_id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL,
  owner_client_instance_id TEXT NOT NULL,
  revision INTEGER NOT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
CREATE TABLE workers (
  worker_id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL,
  profile_id TEXT NOT NULL REFERENCES worker_profiles(id),
  parent_worker_id TEXT REFERENCES workers(worker_id),
  created_at TEXT NOT NULL
);
CREATE TABLE runs (
  run_id TEXT PRIMARY KEY,
  task_id TEXT NOT NULL REFERENCES tasks(task_id),
  worker_id TEXT NOT NULL REFERENCES workers(worker_id),
  state TEXT NOT NULL,
  flags_degraded_control INTEGER NOT NULL DEFAULT 0,
  flags_needs_reconciliation INTEGER NOT NULL DEFAULT 0,
  flags_protocol_unhealthy INTEGER NOT NULL DEFAULT 0,
  flags_policy_quarantined INTEGER NOT NULL DEFAULT 0,
  flags_workspace_dirty INTEGER NOT NULL DEFAULT 0,
  flags_children_active INTEGER NOT NULL DEFAULT 0,
  vendor_session_id TEXT,
  created_at TEXT NOT NULL,
  started_at TEXT,
  completed_at TEXT
);
CREATE TABLE messages (
  message_id TEXT PRIMARY KEY,
  run_id TEXT NOT NULL REFERENCES runs(run_id),
  sender_worker_id TEXT NOT NULL,
  recipient_worker_id TEXT,
  task_id TEXT NOT NULL,
  kind TEXT NOT NULL,
  payload TEXT NOT NULL,
  delivery_state TEXT NOT NULL,
  created_at TEXT NOT NULL,
  sent_at TEXT,
  acknowledged_at TEXT,
  reply_to TEXT
);
CREATE TABLE approvals (
  approval_id TEXT PRIMARY KEY,
  run_id TEXT NOT NULL REFERENCES runs(run_id),
  task_id TEXT NOT NULL,
  action TEXT NOT NULL,
  arguments TEXT NOT NULL,
  human_required INTEGER NOT NULL DEFAULT 0,
  policy_reason TEXT NOT NULL,
  created_at TEXT NOT NULL,
  decided_at TEXT,
  decision TEXT
);
";

/// Migration 3: registered adapter worker profiles (Worker Adapters
/// milestone), plus a `workers.resolved_profile_json` column carrying the
/// full resolved `WorkerProfile` snapshot (startup options, environment
/// allowlist, source) for a worker created from a `profileId` -- copied
/// in once at creation time, so it is immune to whatever later happens to
/// the source row in `adapter_profiles`. `adapter_profiles` itself is
/// deliberately outside the append-only `events` journal -- profile
/// registration is configuration, not an orchestration fact, so it is
/// never journaled or broadcast (see
/// `crate::adapter::profile_store::ProfileStore`).
const MIGRATION_3: &str = "
CREATE TABLE adapter_profiles (
  id TEXT PRIMARY KEY,
  adapter TEXT NOT NULL,
  model TEXT NOT NULL,
  permission_envelope TEXT NOT NULL,
  startup_options_json TEXT NOT NULL,
  environment_allowlist_json TEXT NOT NULL,
  source TEXT NOT NULL,
  fingerprint TEXT NOT NULL,
  created_at TEXT NOT NULL
);
ALTER TABLE workers ADD COLUMN resolved_profile_json TEXT;
";

/// Migration 4: mid-run nested-worker policy violations (Hardening plan
/// Task 1). Distinct from the pre-authorization `PolicyViolation` runtime
/// event -- this table tracks a worker that was already running and then
/// unexpectedly reported a child, through to its resolution via
/// `policy/violation/decide`. `action` is the `NestedViolationAction`
/// applied at record time (`quarantine`/`cancel`/`quarantineAndCancel`);
/// `resolution` is `release`/`cancel`, set only once `resolved_at` is set.
const MIGRATION_4: &str = "
CREATE TABLE policy_violations (
  violation_id TEXT PRIMARY KEY,
  run_id TEXT NOT NULL REFERENCES runs(run_id),
  task_id TEXT NOT NULL,
  worker_id TEXT NOT NULL,
  vendor_child_id TEXT NOT NULL,
  vendor_parent_ref TEXT NOT NULL,
  action TEXT NOT NULL,
  created_at TEXT NOT NULL,
  resolved_at TEXT,
  resolution TEXT,
  resolved_by TEXT
);
";

/// Migration 5: enrich the events journal with envelope convenience columns
/// so that events/replay can reconstruct full EventEnvelopes from disk
/// (previously these fields were only available in-memory during live
/// broadcast). The columns are nullable -- existing rows before migration
/// will have NULL here, and replay() handles NULL by returning None.
const MIGRATION_5: &str = "
ALTER TABLE events ADD COLUMN task_id TEXT;
ALTER TABLE events ADD COLUMN worker_id TEXT;
ALTER TABLE events ADD COLUMN parent_worker_id TEXT;
ALTER TABLE events ADD COLUMN vendor_event_ref TEXT;
";

/// Migration 6: the policy snapshot each run was authorized under, so a
/// violation or audit can be resolved against a specific merged policy.
/// Nullable, because rows written before this migration have no
/// fingerprint -- never backfill a fabricated one.
const MIGRATION_6: &str = "
ALTER TABLE runs ADD COLUMN policy_fingerprint TEXT;
";

/// Opens `path` as a private (mode `0600`) SQLite database, configures its
/// PRAGMAs (`journal_mode=WAL`, `foreign_keys=ON`, `busy_timeout=5000`,
/// `synchronous=FULL`), and migrates it to the latest schema. Migrations
/// are applied atomically by `rusqlite_migration`.
///
/// # Errors
/// Returns [`DbError`] if the file cannot be created privately, the
/// connection cannot be opened or configured, or migration fails.
pub(super) fn open_and_migrate(path: &Path) -> Result<Connection, DbError> {
    crate::security::ensure_private_file(path)?;

    let mut conn = Connection::open(path)?;

    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", true)?;
    conn.pragma_update(None, "busy_timeout", 5000_i64)?;
    conn.pragma_update(None, "synchronous", "FULL")?;

    migrate(&mut conn)?;

    Ok(conn)
}

/// Applies every migration to an already-open connection, atomically.
/// The one place the migration list lives: tests open an in-memory
/// connection and call this rather than hand-copying a schema, so a
/// projection table can never drift from what production runs against.
///
/// # Errors
/// Returns [`DbError`] if migration fails.
pub fn migrate(conn: &mut Connection) -> Result<(), DbError> {
    let migrations = Migrations::new(vec![
        M::up(MIGRATION_1),
        M::up(MIGRATION_2),
        M::up(MIGRATION_3),
        M::up(MIGRATION_4),
        M::up(MIGRATION_5),
        M::up(MIGRATION_6),
    ]);
    migrations.to_latest(conn)?;
    Ok(())
}
