//! Workspace lease arbitration.

use batman_protocol::{IsolationKind, LeaseMode, ProjectId, RunId, WorkspaceInfo, WorkspaceState};
use rusqlite::params;
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum LeaseError {
    #[error("database error: {0}")]
    Db(String),
    #[error("lease not found: {lease_id}")]
    NotFound { lease_id: String },
    #[error("conflict: another lease exists for this project")]
    Conflict,
    #[error("lease already released: {lease_id}")]
    AlreadyReleased { lease_id: String },
}

#[derive(Debug, Clone)]
pub struct CreatedLease {
    pub lease_id: String,
    pub run_id: RunId,
    pub mode: LeaseMode,
    pub path: String,
    pub isolation_kind: IsolationKind,
    pub base_revision: String,
    pub state: WorkspaceState,
    pub acquisition_sequence: u64,
}

pub struct LeaseService {
    db_path: std::path::PathBuf,
}

impl LeaseService {
    pub fn open_in_memory(_project_id: ProjectId) -> Result<Self, LeaseError> {
        let db_path = std::env::temp_dir().join(format!("batman-lease-{}.db", Uuid::now_v7()));
        Self::open(_project_id, &db_path)
    }

    pub fn open(_project_id: ProjectId, db_path: &std::path::Path) -> Result<Self, LeaseError> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| LeaseError::Db(e.to_string()))?;
        }

        let conn =
            rusqlite::Connection::open(db_path).map_err(|e| LeaseError::Db(e.to_string()))?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS workspace_leases (
            lease_id TEXT PRIMARY KEY, run_id TEXT NOT NULL, mode TEXT NOT NULL,
            isolation_kind TEXT NOT NULL DEFAULT 'shared', path TEXT NOT NULL,
            base_revision TEXT NOT NULL, state TEXT NOT NULL DEFAULT 'active',
            acquired_at TEXT NOT NULL, acquisition_sequence INTEGER NOT NULL DEFAULT 0,
            released_at TEXT
        )",
        )
        .map_err(|e| LeaseError::Db(e.to_string()))?;

        let _ = conn.close();

        Ok(LeaseService {
            db_path: db_path.to_path_buf(),
        })
    }

    pub fn acquire(
        &self,
        run_id: RunId,
        mode: LeaseMode,
        requested_isolation: Option<IsolationKind>,
    ) -> Result<CreatedLease, LeaseError> {
        let conn =
            rusqlite::Connection::open(&self.db_path).map_err(|e| LeaseError::Db(e.to_string()))?;

        conn.execute("BEGIN IMMEDIATE", params![])
            .map_err(|e| LeaseError::Db(e.to_string()))?;

        let active_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM workspace_leases WHERE state IN ('allocating', 'active')",
                params![],
                |row| row.get(0),
            )
            .map_err(|e| LeaseError::Db(e.to_string()))?;

        if mode == LeaseMode::Write && active_count > 0 {
            let _ = conn.execute("ROLLBACK", params![]);
            return Err(LeaseError::Conflict);
        }

        if mode == LeaseMode::ReadOnly {
            let write_count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM workspace_leases WHERE mode = 'write' AND state IN ('allocating', 'active')",
                params![],
                |row| row.get(0),
            ).map_err(|e| LeaseError::Db(e.to_string()))?;

            if write_count > 0 {
                let _ = conn.execute("ROLLBACK", params![]);
                return Err(LeaseError::Conflict);
            }
        }

        let lease_id = Uuid::now_v7().to_string();
        let ws_path = format!("/tmp/ws-{}", lease_id.clone());
        let now: OffsetDateTime = OffsetDateTime::now_utc();
        let now_str = now
            .format(&time::format_description::well_known::Rfc3339)
            .map_err(|e| LeaseError::Db(format!("time format: {}", e)))?;

        conn.execute(
            "INSERT INTO workspace_leases (lease_id, run_id, mode, path, base_revision, state, acquired_at, acquisition_sequence)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                &lease_id,
                &run_id.to_string(),
                match mode { LeaseMode::ReadOnly => "readOnly", LeaseMode::Write => "write" },
                &ws_path,
                "HEAD",
                "active",
                now_str,
                1u64,
            ],
        ).map_err(|e| LeaseError::Db(e.to_string()))?;

        conn.execute("COMMIT", params![])
            .map_err(|e| LeaseError::Db(e.to_string()))?;

        Ok(CreatedLease {
            lease_id,
            run_id,
            mode,
            path: ws_path,
            isolation_kind: requested_isolation.unwrap_or(IsolationKind::Shared),
            base_revision: "HEAD".to_string(),
            state: WorkspaceState::Active,
            acquisition_sequence: 1,
        })
    }

    pub fn get(&self, lease_id: String) -> Result<WorkspaceInfo, LeaseError> {
        let conn =
            rusqlite::Connection::open(&self.db_path).map_err(|e| LeaseError::Db(e.to_string()))?;

        let (run_id_str, mode_str, isol_kind, path, state, base_rev): (String, String, String, String, String, String) = conn.query_row(
            "SELECT run_id, mode, isolation_kind, path, state, base_revision FROM workspace_leases WHERE lease_id = ?1",
            params![&lease_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
        ).map_err(|e| LeaseError::Db(e.to_string()))?;

        let mode = match mode_str.as_str() {
            "readOnly" => LeaseMode::ReadOnly,
            "write" => LeaseMode::Write,
            _ => {
                return Err(LeaseError::NotFound {
                    lease_id: lease_id.clone(),
                });
            }
        };

        let state = match state.as_str() {
            "allocating" => WorkspaceState::Allocating,
            "active" => WorkspaceState::Active,
            "dirty" => WorkspaceState::Dirty,
            "released" => WorkspaceState::Released,
            "cleanupFailed" => WorkspaceState::CleanupFailed,
            _ => {
                return Err(LeaseError::NotFound {
                    lease_id: lease_id.clone(),
                });
            }
        };

        let _ = conn.close();

        Ok(WorkspaceInfo {
            lease_id,
            run_id: run_id_from_str(&run_id_str)?,
            mode,
            isolation_kind: match isol_kind.as_str() {
                "shared" => IsolationKind::Shared,
                "gitWorktree" => IsolationKind::GitWorktree,
                "copy" => IsolationKind::Copy,
                _ => IsolationKind::Shared,
            },
            path,
            state,
            base_revision: base_rev,
        })
    }

    pub fn release(&self, lease_id: String) -> Result<(), LeaseError> {
        let conn =
            rusqlite::Connection::open(&self.db_path).map_err(|e| LeaseError::Db(e.to_string()))?;

        let state: String = conn
            .query_row(
                "SELECT state FROM workspace_leases WHERE lease_id = ?1",
                params![&lease_id],
                |row| row.get(0),
            )
            .map_err(|e| LeaseError::Db(e.to_string()))?;

        if state == "released" {
            return Err(LeaseError::AlreadyReleased { lease_id });
        }

        let now: OffsetDateTime = OffsetDateTime::now_utc();
        let now_str = now
            .format(&time::format_description::well_known::Rfc3339)
            .map_err(|e| LeaseError::Db(format!("time format: {}", e)))?;

        conn.execute(
            "UPDATE workspace_leases SET state = 'released', released_at = ?1 WHERE lease_id = ?2",
            params![now_str, lease_id],
        )
        .map_err(|e| LeaseError::Db(e.to_string()))?;

        let _ = conn.close();

        Ok(())
    }

    pub fn active_for_repository(&self) -> Result<u64, LeaseError> {
        let conn =
            rusqlite::Connection::open(&self.db_path).map_err(|e| LeaseError::Db(e.to_string()))?;

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM workspace_leases WHERE state IN ('allocating', 'active')",
                [],
                |row| row.get(0),
            )
            .map_err(|e| LeaseError::Db(e.to_string()))?;

        let _ = conn.close();

        Ok(count as u64)
    }
}

fn run_id_from_str(s: &str) -> Result<RunId, LeaseError> {
    RunId::parse(s).map_err(|_| LeaseError::NotFound {
        lease_id: s.to_string(),
    })
}
