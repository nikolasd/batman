//! Reconnect-capable worker-MCP scope tokens.
//!
//! A token is minted immediately before the supervised vendor process
//! launches and bound to `{ projectId, taskId, workerId, runId,
//! vendorProcessIdentity, expiresAt }`. Verification on every MCP socket
//! initialization checks the token is live (not expired, its run still
//! live) and that the connecting peer's process is a descendant of the
//! recorded vendor process -- a restarted MCP subprocess within that same
//! process tree may reinitialize with the same token; a peer outside the
//! ancestry, after vendor exit, or after expiry is rejected.
//!
//! Token bytes are the `HashMap` key only: never journaled, logged, or
//! echoed back in any diagnostic -- only the bound fields (never the token
//! string itself) are visible outside this module.

use std::collections::HashMap;
use std::sync::Mutex;

use batman_protocol::{ProjectId, RunId, TaskId, Timestamp, WorkerId};

use crate::ipc::{ScopedRun, VerifyError, WorkerCredentialVerifier};

/// The vendor process a scope token is bound to: its PID at mint time. Used
/// only to walk the connecting peer's ancestry; never persisted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VendorProcessIdentity {
    pub pid: i32,
}

/// The record a live scope token is bound to.
#[derive(Debug, Clone)]
struct ScopeTokenRecord {
    project_id: ProjectId,
    task_id: TaskId,
    worker_id: WorkerId,
    run_id: RunId,
    vendor_process: VendorProcessIdentity,
    expires_at: Timestamp,
}

/// Checks whether one process is a descendant of another by walking parent
/// PIDs. Injectable so tests can simulate ancestry without real processes;
/// the [`SystemPidAncestryChecker`] default walks the real process tree.
pub trait PidAncestryChecker: Send + Sync {
    /// Returns `Ok(true)` if `candidate` is `ancestor` or a descendant of
    /// it, `Ok(false)` if the walk reaches the process tree root without
    /// finding `ancestor`, or `Err` if this platform cannot report
    /// trustworthy process ancestry.
    fn is_descendant(&self, candidate: i32, ancestor: i32) -> Result<bool, AncestryError>;
}

/// Why a process-ancestry check could not be completed.
#[derive(Debug, thiserror::Error)]
pub enum AncestryError {
    /// This platform has no supported mechanism to walk process ancestry.
    #[error("process ancestry is not supported on this platform")]
    Unsupported,
}

/// The real ancestry checker: walks parent PIDs via `ps -o ppid=`, portable
/// across macOS and Linux without a platform-specific `/proc` dependency.
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub struct SystemPidAncestryChecker;

#[cfg(any(target_os = "macos", target_os = "linux"))]
impl PidAncestryChecker for SystemPidAncestryChecker {
    fn is_descendant(&self, candidate: i32, ancestor: i32) -> Result<bool, AncestryError> {
        let mut pid = candidate;
        // Bound the walk: a real process tree is never this deep, and this
        // guards against a parent-pid cycle reported by a hostile/broken ps.
        for _ in 0..4096 {
            if pid == ancestor {
                return Ok(true);
            }
            if pid <= 1 {
                return Ok(false);
            }
            match parent_pid(pid) {
                Some(parent) if parent != pid => pid = parent,
                _ => return Ok(false),
            }
        }
        Ok(false)
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn parent_pid(pid: i32) -> Option<i32> {
    let output = std::process::Command::new("ps")
        .args(["-o", "ppid=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()?.trim().parse().ok()
}

/// The foundation-style default on platforms without a supported ancestry
/// mechanism: reports worker coordination as unsupported rather than
/// accepting an unverifiable reconnect.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub struct SystemPidAncestryChecker;

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
impl PidAncestryChecker for SystemPidAncestryChecker {
    fn is_descendant(&self, _candidate: i32, _ancestor: i32) -> Result<bool, AncestryError> {
        Err(AncestryError::Unsupported)
    }
}

/// An in-memory store of live scope tokens, backing a
/// [`WorkerCredentialVerifier`]. One store per runtime process.
pub struct ScopeTokenStore {
    tokens: Mutex<HashMap<String, ScopeTokenRecord>>,
    ancestry: Box<dyn PidAncestryChecker>,
}

impl ScopeTokenStore {
    /// Creates an empty store using the real system ancestry checker.
    #[must_use]
    pub fn new() -> Self {
        Self::with_ancestry_checker(Box::new(SystemPidAncestryChecker))
    }

    /// Creates an empty store using an injected ancestry checker (tests).
    #[must_use]
    pub fn with_ancestry_checker(ancestry: Box<dyn PidAncestryChecker>) -> Self {
        Self {
            tokens: Mutex::new(HashMap::new()),
            ancestry,
        }
    }

    /// Mints a fresh token bound to the given scope, returning its bearer
    /// string. Call immediately before launching the supervised vendor
    /// process.
    pub fn mint(
        &self,
        project_id: ProjectId,
        task_id: TaskId,
        worker_id: WorkerId,
        run_id: RunId,
        vendor_process: VendorProcessIdentity,
        expires_at: Timestamp,
    ) -> String {
        let token = uuid::Uuid::now_v7().to_string();
        self.tokens.lock().expect("scope token mutex is never poisoned").insert(
            token.clone(),
            ScopeTokenRecord {
                project_id,
                task_id,
                worker_id,
                run_id,
                vendor_process,
                expires_at,
            },
        );
        token
    }

    /// Revokes the token bound to `run_id`, if any (e.g. when the run
    /// settles). Idempotent.
    pub fn revoke_for_run(&self, run_id: RunId) {
        let mut tokens = self.tokens.lock().expect("scope token mutex is never poisoned");
        tokens.retain(|_, record| record.run_id != run_id);
    }

    /// Verifies `token` against a live record, then checks `peer_pid` is
    /// the recorded vendor process or one of its descendants.
    ///
    /// # Errors
    /// Returns [`VerifyError::InvalidToken`] if the token is unknown or
    /// expired, and [`VerifyError::OutsideAncestry`] if `peer_pid` is not
    /// the vendor process or a descendant of it (including when this
    /// platform cannot report trustworthy ancestry at all).
    pub fn verify(&self, token: &str, peer_pid: Option<i32>) -> Result<ScopedRun, VerifyError> {
        let record = {
            let tokens = self.tokens.lock().expect("scope token mutex is never poisoned");
            tokens.get(token).cloned()
        };
        let record = record.ok_or(VerifyError::InvalidToken)?;

        let now = Timestamp::now();
        if now > record.expires_at {
            return Err(VerifyError::InvalidToken);
        }

        let Some(peer_pid) = peer_pid else {
            return Err(VerifyError::OutsideAncestry);
        };
        let is_descendant = self
            .ancestry
            .is_descendant(peer_pid, record.vendor_process.pid)
            .map_err(|_| VerifyError::OutsideAncestry)?;
        if !is_descendant {
            return Err(VerifyError::OutsideAncestry);
        }

        Ok(ScopedRun {
            run_id: record.run_id,
        })
    }

    /// Returns the full scope (project/task/worker/run) bound to a live
    /// token, without re-verifying ancestry. Used by the coordination
    /// broker after the connection has already been admitted.
    #[must_use]
    pub fn scope_for_run(&self, run_id: RunId) -> Option<(ProjectId, TaskId, WorkerId)> {
        let tokens = self.tokens.lock().expect("scope token mutex is never poisoned");
        tokens
            .values()
            .find(|record| record.run_id == run_id)
            .map(|record| (record.project_id, record.task_id, record.worker_id))
    }
}

impl Default for ScopeTokenStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Adapts [`ScopeTokenStore`] to [`WorkerCredentialVerifier`] for wiring
/// into [`crate::ipc::ServerConfig`].
pub struct ScopeTokenVerifier {
    store: std::sync::Arc<ScopeTokenStore>,
}

impl ScopeTokenVerifier {
    #[must_use]
    pub fn new(store: std::sync::Arc<ScopeTokenStore>) -> Self {
        Self { store }
    }
}

impl WorkerCredentialVerifier for ScopeTokenVerifier {
    fn verify(&self, scope_token: &str, peer_pid: Option<i32>) -> Result<ScopedRun, VerifyError> {
        self.store.verify(scope_token, peer_pid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeAncestry {
        descendant_of: Vec<(i32, i32)>,
    }

    impl PidAncestryChecker for FakeAncestry {
        fn is_descendant(&self, candidate: i32, ancestor: i32) -> Result<bool, AncestryError> {
            Ok(self.descendant_of.contains(&(candidate, ancestor)) || candidate == ancestor)
        }
    }

    fn store_with(pairs: Vec<(i32, i32)>) -> ScopeTokenStore {
        ScopeTokenStore::with_ancestry_checker(Box::new(FakeAncestry {
            descendant_of: pairs,
        }))
    }

    #[test]
    fn verifies_a_descendant_of_the_vendor_process() {
        let store = store_with(vec![(200, 100)]);
        let run_id = RunId::new();
        let token = store.mint(
            ProjectId::new(),
            TaskId::new(),
            WorkerId::new(),
            run_id,
            VendorProcessIdentity { pid: 100 },
            Timestamp::parse("2099-01-01T00:00:00Z").unwrap(),
        );

        let scoped = store.verify(&token, Some(200)).expect("descendant verifies");
        assert_eq!(scoped.run_id, run_id);
    }

    #[test]
    fn rejects_a_peer_outside_ancestry() {
        let store = store_with(vec![]);
        let token = store.mint(
            ProjectId::new(),
            TaskId::new(),
            WorkerId::new(),
            RunId::new(),
            VendorProcessIdentity { pid: 100 },
            Timestamp::parse("2099-01-01T00:00:00Z").unwrap(),
        );

        let err = store.verify(&token, Some(999)).unwrap_err();
        assert!(matches!(err, VerifyError::OutsideAncestry));
    }

    #[test]
    fn rejects_after_expiry() {
        let store = store_with(vec![]);
        let token = store.mint(
            ProjectId::new(),
            TaskId::new(),
            WorkerId::new(),
            RunId::new(),
            VendorProcessIdentity { pid: 100 },
            Timestamp::parse("2000-01-01T00:00:00Z").unwrap(),
        );

        let err = store.verify(&token, Some(100)).unwrap_err();
        assert!(matches!(err, VerifyError::InvalidToken));
    }

    #[test]
    fn rejects_an_unknown_token() {
        let store = store_with(vec![]);
        let err = store.verify("not-a-real-token", Some(100)).unwrap_err();
        assert!(matches!(err, VerifyError::InvalidToken));
    }

    #[test]
    fn a_restarted_descendant_may_reverify_the_same_token_while_the_run_is_live() {
        let store = store_with(vec![(201, 100), (202, 100)]);
        let run_id = RunId::new();
        let token = store.mint(
            ProjectId::new(),
            TaskId::new(),
            WorkerId::new(),
            run_id,
            VendorProcessIdentity { pid: 100 },
            Timestamp::parse("2099-01-01T00:00:00Z").unwrap(),
        );

        // First MCP subprocess initializes.
        assert!(store.verify(&token, Some(201)).is_ok());
        // It restarts under a new PID, still a descendant of the same
        // supervised vendor process, and reinitializes with the same token.
        let scoped = store.verify(&token, Some(202)).expect("restarted descendant reverifies");
        assert_eq!(scoped.run_id, run_id);
    }

    #[test]
    fn revoking_a_run_invalidates_its_token() {
        let store = store_with(vec![(100, 100)]);
        let run_id = RunId::new();
        let token = store.mint(
            ProjectId::new(),
            TaskId::new(),
            WorkerId::new(),
            run_id,
            VendorProcessIdentity { pid: 100 },
            Timestamp::parse("2099-01-01T00:00:00Z").unwrap(),
        );
        assert!(store.verify(&token, Some(100)).is_ok());

        store.revoke_for_run(run_id);

        let err = store.verify(&token, Some(100)).unwrap_err();
        assert!(matches!(err, VerifyError::InvalidToken));
    }

    #[test]
    fn rejects_when_the_platform_reports_no_peer_pid() {
        let store = store_with(vec![]);
        let token = store.mint(
            ProjectId::new(),
            TaskId::new(),
            WorkerId::new(),
            RunId::new(),
            VendorProcessIdentity { pid: 100 },
            Timestamp::parse("2099-01-01T00:00:00Z").unwrap(),
        );
        let err = store.verify(&token, None).unwrap_err();
        assert!(matches!(err, VerifyError::OutsideAncestry));
    }
}
