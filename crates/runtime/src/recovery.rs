//! Crash recovery for the BATMAN runtime.
//!
//! After an unclean shutdown (crash, OOM kill, SIGKILL), runs may be left in
//! non-terminal states (`queued`, `starting`, `working`, `waitingUser`,
//! `waitingPeer`, `paused`). The [`RecoveryCoordinator`] finds these stuck
//! runs and transitions them to appropriate terminal states based on their
//! last activity timestamp and current state.
//!
//! This is the runtime's self-healing mechanism: it runs automatically after
//! each `serve` command and can also be triggered manually via the `status`
//! command with `--recover` flag.

use std::sync::Arc;
use std::time::Duration;

use batman_protocol::{RunId, RunState, WorkerId};
use thiserror::Error;

use crate::db::DatabaseHandle;

/// Errors that can occur during crash recovery.
#[derive(Debug, Error)]
pub enum RecoveryError {
    /// The database handle is invalid or closed.
    #[error("database handle is invalid: {0}")]
    InvalidDatabase(String),

    /// A run could not be transitioned to a terminal state.
    #[error("failed to transition run {run_id} from {from_state} to {to_state}: {reason}")]
    TransitionFailed {
        run_id: String,
        from_state: String,
        to_state: String,
        reason: String,
    },

    /// No runs were found that needed recovery.
    #[error("no runs found that needed recovery")]
    NoRunsToRecover,
}

/// Configuration for crash recovery.
#[derive(Debug, Clone)]
pub struct RecoveryConfig {
    /// The threshold for considering a run "stuck". Runs in non-terminal states
    /// that haven't had activity for longer than this duration will be recovered.
    pub stuck_threshold: Duration,

    /// Whether to recover runs in `paused` state. Paused runs are intentionally
    /// waiting for user input, so recovering them would cancel valid work.
    pub recover_paused: bool,

    /// Whether to recover runs in `waitingUser` or `waitingPeer` state. These
    /// runs are waiting for approval, so recovering them would cancel valid work.
    pub recover_waiting: bool,
}

impl Default for RecoveryConfig {
    fn default() -> Self {
        Self {
            stuck_threshold: Duration::from_secs(300), // 5 minutes
            recover_paused: false,
            recover_waiting: false,
        }
    }
}

/// Result of a recovery operation.
#[derive(Debug, Clone)]
pub struct RecoveryResult {
    /// The number of runs that were recovered.
    pub recovered_count: usize,

    /// The runs that were recovered, with their previous and new states.
    pub recovered_runs: Vec<RecoveredRun>,
}

/// A single run that was recovered.
#[derive(Debug, Clone)]
pub struct RecoveredRun {
    /// The run's unique identifier.
    pub run_id: RunId,

    /// The run's previous state before recovery.
    pub previous_state: RunState,

    /// The run's new state after recovery.
    pub new_state: RunState,

    /// Whether the recovery was successful.
    pub success: bool,

    /// An optional error message if recovery failed.
    pub error: Option<String>,
}

/// Coordinates crash recovery for the BATMAN runtime.
///
/// The [`RecoveryCoordinator`] finds runs that are stuck in non-terminal states
/// after an unclean shutdown and transitions them to appropriate terminal states.
#[expect(dead_code)]
pub struct RecoveryCoordinator {
    db: Arc<DatabaseHandle>,
    config: RecoveryConfig,
}

impl RecoveryCoordinator {
    /// Creates a new [`RecoveryCoordinator`] with the given database handle and
    /// configuration.
    #[must_use]
    pub fn new(db: Arc<DatabaseHandle>, config: RecoveryConfig) -> Self {
        Self { db, config }
    }

    /// Creates a [`RecoveryCoordinator`] with default configuration.
    #[must_use]
    pub fn with_defaults(db: Arc<DatabaseHandle>) -> Self {
        Self::new(db, RecoveryConfig::default())
    }

    /// Performs crash recovery on all runs in the database.
    ///
    /// This method:
    /// 1. Finds all runs in non-terminal states
    /// 2. Checks if they've been stuck for longer than [`RecoveryConfig::stuck_threshold`]
    /// 3. Transitions stuck runs to appropriate terminal states based on their
    ///    current state and the recovery configuration
    ///
    /// # Errors
    ///
    /// Returns a [`RecoveryError`] if:
    /// - The database handle is invalid
    /// - A run transition fails
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use batman_runtime::db::DatabaseHandle;
    /// # use batman_runtime::recovery::RecoveryCoordinator;
    /// # async fn example(db: Arc<DatabaseHandle>) -> Result<(), Box<dyn std::error::Error>> {
    /// let coordinator = RecoveryCoordinator::with_defaults(db);
    /// let result = coordinator.recover().await?;
    /// println!("Recovered {} runs", result.recovered_count);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn recover(&self) -> Result<RecoveryResult, RecoveryError> {
        // This is a stub implementation. A full implementation would query
        // the database for runs in non-terminal states with last_activity
        // older than the stuck_threshold.
        Ok(RecoveryResult {
            recovered_count: 0,
            recovered_runs: Vec::new(),
        })
    }

    /// Finds all runs that are stuck in non-terminal states.
    ///
    /// A run is considered stuck if:
    /// - It's in a non-terminal state (`queued`, `starting`, `working`,
    ///   `waitingUser`, `waitingPeer`, `paused`)
    /// - It hasn't had activity for longer than [`RecoveryConfig::stuck_threshold`]
    /// - If [`RecoveryConfig::recover_paused`] is `false`, runs in `paused` state
    ///   are excluded
    /// - If [`RecoveryConfig::recover_waiting`] is `false`, runs in
    ///   `waitingUser` or `waitingPeer` state are excluded
    #[allow(dead_code)]
    async fn find_stuck_runs(
        &self,
    ) -> Result<Vec<StuckRun>, RecoveryError> {
        // This is a stub implementation. A full implementation would query
        // the database for runs in non-terminal states with last_activity
        // older than the stuck_threshold.
        Ok(Vec::new())
    }

    /// Recovers a single stuck run by transitioning it to an appropriate
    /// terminal state.
    ///
    /// The target state is determined by the run's current state:
    /// - `queued` → `failed`
    /// - `starting` → `failed`
    /// - `working` → `failed`
    /// - `waitingUser` → `cancelled` (if [`RecoveryConfig::recover_waiting`] is `true`)
    /// - `waitingPeer` → `cancelled` (if [`RecoveryConfig::recover_waiting`] is `true`)
    /// - `paused` → `cancelled` (if [`RecoveryConfig::recover_paused`] is `true`)
    #[allow(dead_code)]
    async fn recover_run(&self, stuck_run: &StuckRun) -> Result<RunState, RecoveryError> {
        // This is a stub implementation. A full implementation would call
        // the domain repository to transition the run to the target state.
        Ok(stuck_run.current_state.clone())
    }
}

/// A run that is stuck in a non-terminal state.
#[expect(dead_code)]
struct StuckRun {
    /// The run's unique identifier.
    run_id: RunId,

    /// The run's current state.
    current_state: RunState,

    /// The run's worker identifier.
    worker_id: WorkerId,

    /// The timestamp of the last activity.
    last_activity: std::time::SystemTime,
}
