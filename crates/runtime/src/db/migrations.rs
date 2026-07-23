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

    let migrations = Migrations::new(vec![M::up(MIGRATION_1)]);
    migrations.to_latest(&mut conn)?;

    Ok(conn)
}
