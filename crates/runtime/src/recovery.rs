//! Crash recovery for the BATMAN runtime.
//!
//! After an unclean shutdown (crash, OOM kill, SIGKILL), runs may be left in
//! non-terminal states (`queued`, `starting`, `working`, `waitingUser`,
//! `waitingPeer`, `paused`). The [`RecoveryCoordinator`] finds these stuck
//! runs -- ones whose most recent journaled event (or creation time, if
//! none) predates [`RecoveryConfig::stuck_threshold`] -- and transitions
//! each to an appropriate terminal state: `queued`/`starting`/`working` to
//! `failed` (no evidence the work ever completed), and `waitingUser`/
//! `waitingPeer`/`paused` to `cancelled` when the corresponding
//! [`RecoveryConfig`] flag opts in (these runs are intentionally waiting on
//! a human/peer, so recovering them by default would cancel valid work).
//!
//! [`crate::lifecycle::serve`] runs this sweep once, synchronously, after
//! opening the database but before the socket accepts any connection --
//! every run this sweep would touch is guaranteed stale from before this
//! process started, so there is no risk of racing a live mutation.

use std::sync::Arc;
use std::time::Duration;

use batman_protocol::{ProjectId, RunId, RunState, WorkerId};
use thiserror::Error;

use crate::db::{DatabaseHandle, DomainClosure};
use crate::domain::DomainRepository;


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

    /// The run's worker identifier.
    pub worker_id: WorkerId,

    /// The run's previous state before recovery.
    pub previous_state: RunState,

    /// The run's new state after recovery.
    pub new_state: RunState,

    /// The RFC 3339 timestamp of the run's last activity before recovery.
    pub last_activity: String,

    /// Whether the recovery was successful.
    pub success: bool,

    /// An optional error message if recovery failed.
    pub error: Option<String>,
}

/// Coordinates crash recovery for the BATMAN runtime.
///
/// The [`RecoveryCoordinator`] finds runs that are stuck in non-terminal states
/// after an unclean shutdown and transitions them to appropriate terminal states.
pub struct RecoveryCoordinator {
    db: Arc<DatabaseHandle>,
    project_id: ProjectId,
    config: RecoveryConfig,
}

impl RecoveryCoordinator {
    /// Creates a new [`RecoveryCoordinator`] with the given database handle and
    /// configuration.
    #[must_use]
    pub fn new(db: Arc<DatabaseHandle>, project_id: ProjectId, config: RecoveryConfig) -> Self {
        Self {
            db,
            project_id,
            config,
        }
    }

    /// Creates a [`RecoveryCoordinator`] with default configuration.
    #[must_use]
    pub fn with_defaults(db: Arc<DatabaseHandle>, project_id: ProjectId) -> Self {
        Self::new(db, project_id, RecoveryConfig::default())
    }

    /// Performs crash recovery on all runs in the database.
    ///
    /// This method:
    /// 1. Finds all runs in non-terminal states
    /// 2. Checks if they've been stuck for longer than [`RecoveryConfig::stuck_threshold`]
    /// 3. Transitions stuck runs to appropriate terminal states based on their
    ///    current state and the recovery configuration
    ///
    /// Each stuck run is recovered independently -- one run's transition
    /// failure never aborts the sweep for the others; it is recorded on
    /// that run's own [`RecoveredRun::success`]/[`RecoveredRun::error`].
    ///
    /// # Errors
    ///
    /// Returns a [`RecoveryError`] only if finding stuck runs itself fails
    /// (the database handle is invalid, or a stored row is corrupt).
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use std::sync::Arc;
    /// # use batman_protocol::ProjectId;
    /// # use batman_runtime::db::DatabaseHandle;
    /// # use batman_runtime::recovery::RecoveryCoordinator;
    /// # async fn example(db: Arc<DatabaseHandle>, project_id: ProjectId) -> Result<(), Box<dyn std::error::Error>> {
    /// let coordinator = RecoveryCoordinator::with_defaults(db, project_id);
    /// let result = coordinator.recover().await?;
    /// println!("Recovered {} runs", result.recovered_count);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn recover(&self) -> Result<RecoveryResult, RecoveryError> {
        let stuck_runs = self.find_stuck_runs().await?;
        let mut recovered_runs = Vec::with_capacity(stuck_runs.len());
        for stuck in &stuck_runs {
            match self.recover_run(stuck).await {
                Ok(new_state) => recovered_runs.push(RecoveredRun {
                    run_id: stuck.run_id,
                    worker_id: stuck.worker_id,
                    previous_state: stuck.current_state.clone(),
                    new_state,
                    last_activity: stuck.last_activity.clone(),
                    success: true,
                    error: None,
                }),
                Err(err) => recovered_runs.push(RecoveredRun {
                    run_id: stuck.run_id,
                    worker_id: stuck.worker_id,
                    previous_state: stuck.current_state.clone(),
                    new_state: stuck.current_state.clone(),
                    last_activity: stuck.last_activity.clone(),
                    success: false,
                    error: Some(err.to_string()),
                }),
            }
        }
        let recovered_count = recovered_runs.iter().filter(|r| r.success).count();
        Ok(RecoveryResult {
            recovered_count,
            recovered_runs,
        })
    }

    /// Finds all runs that are stuck in non-terminal states.
    ///
    /// A run is considered stuck if:
    /// - It's in a non-terminal state (`queued`, `starting`, `working`,
    ///   `waitingUser`, `waitingPeer`, `paused`)
    /// - Its last activity -- the timestamp of its most recent journaled
    ///   event, or its `created_at` if it has none -- predates
    ///   [`RecoveryConfig::stuck_threshold`]
    /// - If [`RecoveryConfig::recover_paused`] is `false`, runs in `paused` state
    ///   are excluded
    /// - If [`RecoveryConfig::recover_waiting`] is `false`, runs in
    ///   `waitingUser` or `waitingPeer` state are excluded
    async fn find_stuck_runs(&self) -> Result<Vec<StuckRun>, RecoveryError> {
        let cutoff = time::OffsetDateTime::now_utc()
            .checked_sub(time::Duration::seconds(
                i64::try_from(self.config.stuck_threshold.as_secs()).unwrap_or(i64::MAX),
            ))
            .ok_or_else(|| {
                RecoveryError::InvalidDatabase(
                    "stuck threshold exceeds representable time".to_string(),
                )
            })?
            .format(&time::format_description::well_known::Rfc3339)
            .map_err(|e| {
                RecoveryError::InvalidDatabase(format!("failed to format cutoff timestamp: {e}"))
            })?;

        let project_id = self.project_id;
        let cutoff_param = cutoff.clone();
        let closure: DomainClosure = Box::new(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT r.run_id, r.state, r.worker_id,
                        COALESCE((SELECT MAX(e.timestamp) FROM events e WHERE e.run_id = r.run_id), r.created_at)
                 FROM runs r
                 JOIN tasks t ON r.task_id = t.task_id
                 WHERE t.project_id = ?1
                   AND COALESCE((SELECT MAX(e2.timestamp) FROM events e2 WHERE e2.run_id = r.run_id), r.created_at) < ?2",
            )?;
            let rows: Vec<(String, String, String, String)> = stmt
                .query_map(
                    rusqlite::params![project_id.to_string(), cutoff_param],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                        ))
                    },
                )?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(serde_json::to_value(rows)?)
        });

        let value = self
            .db
            .run_domain_op(closure)
            .await
            .map_err(|e| RecoveryError::InvalidDatabase(e.to_string()))?;
        let rows: Vec<(String, String, String, String)> = serde_json::from_value(value)
            .map_err(|e| RecoveryError::InvalidDatabase(format!("malformed stuck-run rows: {e}")))?;

        let mut stuck = Vec::new();
        for (run_id_str, state_str, worker_id_str, last_activity) in rows {
            let run_id = RunId::parse(&run_id_str).map_err(|e| {
                RecoveryError::InvalidDatabase(format!("invalid run id {run_id_str}: {e}"))
            })?;
            let worker_id = WorkerId::parse(&worker_id_str).map_err(|e| {
                RecoveryError::InvalidDatabase(format!("invalid worker id {worker_id_str}: {e}"))
            })?;
            let current_state = RunState::try_from(state_str.as_str()).map_err(|e| {
                RecoveryError::InvalidDatabase(format!("invalid run state {state_str}: {e}"))
            })?;
            if current_state.is_terminal() {
                // Excluded here (rather than in SQL) so the single source of
                // truth for "which states are terminal" stays
                // `RunState::is_terminal()` -- never a second, driftable copy.
                continue;
            }

            let eligible = match current_state.to_string().as_str() {
                "paused" => self.config.recover_paused,
                "waitingUser" | "waitingPeer" => self.config.recover_waiting,
                _ => true,
            };
            if !eligible {
                continue;
            }

            stuck.push(StuckRun {
                run_id,
                current_state,
                worker_id,
                last_activity,
            });
        }
        Ok(stuck)
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
    ///
    /// `find_stuck_runs` already applies the `recover_paused`/`recover_waiting`
    /// gate, so every [`StuckRun`] reaching this method has a defined target.
    async fn recover_run(&self, stuck_run: &StuckRun) -> Result<RunState, RecoveryError> {
        let target = target_state_for(&stuck_run.current_state).ok_or_else(|| {
            RecoveryError::TransitionFailed {
                run_id: stuck_run.run_id.to_string(),
                from_state: stuck_run.current_state.to_string(),
                to_state: "<none>".to_string(),
                reason: "no recovery target is defined for this state".to_string(),
            }
        })?;

        let run_id = stuck_run.run_id;
        let project_id = self.project_id;
        let target_for_closure = target.clone();
        let closure: DomainClosure = Box::new(move |conn| {
            let mut repo = DomainRepository::new(conn, project_id);
            repo.transition_run(run_id, &target_for_closure)
                .map(|c| serde_json::json!({ "sequence": c.sequence }))
        });

        self.db.run_domain_op(closure).await.map_err(|err| {
            RecoveryError::TransitionFailed {
                run_id: run_id.to_string(),
                from_state: stuck_run.current_state.to_string(),
                to_state: target.to_string(),
                reason: err.to_string(),
            }
        })?;

        Ok(target)
    }
}

/// The terminal state a stuck run in `current` should recover to, or `None`
/// if `current` (already terminal, or an unrecognized state) has no
/// recovery target.
fn target_state_for(current: &RunState) -> Option<RunState> {
    match current.to_string().as_str() {
        "queued" | "starting" | "working" => RunState::try_from("failed").ok(),
        "waitingUser" | "waitingPeer" | "paused" => RunState::try_from("cancelled").ok(),
        _ => None,
    }
}


/// A run that is stuck in a non-terminal state.
struct StuckRun {
    /// The run's unique identifier.
    run_id: RunId,

    /// The run's current state.
    current_state: RunState,

    /// The run's worker identifier.
    worker_id: WorkerId,

    /// The RFC 3339 timestamp of the run's last activity (its most recent
    /// journaled event, or its creation time if it has none).
    last_activity: String,
}
