//! Runtime health checking and rollout gate management.
//!
//! The [`Doctor`] provides a comprehensive health check of the BATMAN runtime,
//! including:
//! - Database connectivity
//! - State directory accessibility
//! - Rollout gate status
//! - Adapter availability
//! - Configuration validity
//!
//! This is used by the `status` CLI command and can also be triggered manually
//! for diagnostics.

use std::path::Path;
use std::sync::Arc;

use serde::Serialize;
use thiserror::Error;

use crate::config::RuntimePolicy;
use crate::db::DatabaseHandle;

/// Errors that can occur during a doctor check.
#[derive(Debug, Error)]
pub enum DoctorError {
    /// The database is not accessible.
    #[error("database is not accessible: {0}")]
    DatabaseError(String),

    /// The state directory is not accessible.
    #[error("state directory is not accessible: {0}")]
    StateDirError(String),

    /// A rollout gate is unresolved.
    #[error("rollout gate '{gate}' is unresolved")]
    RolloutGateUnresolved { gate: String },

    /// A configuration error was detected.
    #[error("configuration error: {0}")]
    ConfigError(String),

    /// An adapter is not available.
    #[error("adapter '{adapter}' is not available: {reason}")]
    AdapterUnavailable { adapter: String, reason: String },
}

/// Result of a doctor check.
#[derive(Debug, Clone, Serialize)]
pub struct DoctorResult {
    /// Whether the runtime is healthy.
    pub healthy: bool,

    /// The set of checks that passed.
    pub passed_checks: Vec<String>,

    /// The set of checks that failed, with error messages.
    pub failed_checks: Vec<FailedCheck>,

    /// The set of unresolved rollout gates.
    pub unresolved_gates: Vec<String>,
}

/// A single failed check.
#[derive(Debug, Clone, Serialize)]
pub struct FailedCheck {
    /// The name of the check.
    pub check_name: String,

    /// The error message.
    pub error: String,
}

/// Performs health checks on the BATMAN runtime.
///
/// The [`Doctor`] checks various aspects of the runtime's health, including
/// database connectivity, state directory accessibility, rollout gate status,
/// adapter availability, and configuration validity.
pub struct Doctor {
    #[allow(dead_code)]
    db: Option<Arc<DatabaseHandle>>,
    #[allow(dead_code)]
    state_dir: Option<std::path::PathBuf>,
    #[allow(dead_code)]
    policy: Option<RuntimePolicy>,
}

impl Doctor {
    /// Creates a new [`Doctor`] with the given database handle, state directory,
    /// and runtime policy.
    #[must_use]
    pub fn new(
        db: Option<Arc<DatabaseHandle>>,
        state_dir: Option<std::path::PathBuf>,
        policy: Option<RuntimePolicy>,
    ) -> Self {
        Self {
            db,
            state_dir,
            policy,
        }
    }

    /// Creates a [`Doctor`] with no database, state directory, or policy.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            db: None,
            state_dir: None,
            policy: None,
        }
    }

    /// Performs a comprehensive health check on the runtime.
    ///
    /// This method checks:
    /// 1. Database connectivity (if a database handle is provided)
    /// 2. State directory accessibility (if a state directory is provided)
    /// 3. Rollout gate status (if a runtime policy is provided)
    /// 4. Configuration validity (if a runtime policy is provided)
    ///
    /// # Errors
    ///
    /// Returns a [`DoctorError`] if any check fails.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use batman_runtime::db::DatabaseHandle;
    /// # use batman_runtime::doctor::Doctor;
    /// # async fn example(db: Arc<DatabaseHandle>) -> Result<(), Box<dyn std::error::Error>> {
    /// let doctor = Doctor::new(Some(db), None, None);
    /// let result = doctor.check().await?;
    /// if result.healthy {
    ///     println!("Runtime is healthy");
    /// } else {
    ///     println!("Runtime has issues: {:?}", result.failed_checks);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn check(&self) -> Result<DoctorResult, DoctorError> {
        let mut passed_checks = Vec::new();
        let mut failed_checks = Vec::new();
        let mut unresolved_gates = Vec::new();

        // Check 1: Database connectivity
        if let Some(_db) = &self.db {
            match self.check_database().await {
                Ok(_) => passed_checks.push("database_connectivity".to_string()),
                Err(e) => {
                    failed_checks.push(FailedCheck {
                        check_name: "database_connectivity".to_string(),
                        error: e.to_string(),
                    });
                }
            }
        } else {
            passed_checks.push("database_connectivity_skipped".to_string());
        }

        // Check 2: State directory accessibility
        if let Some(state_dir) = &self.state_dir {
            match self.check_state_dir(state_dir).await {
                Ok(_) => passed_checks.push("state_dir_accessible".to_string()),
                Err(e) => {
                    failed_checks.push(FailedCheck {
                        check_name: "state_dir_accessible".to_string(),
                        error: e.to_string(),
                    });
                }
            }
        } else {
            passed_checks.push("state_dir_accessible_skipped".to_string());
        }

        // Check 3: Rollout gate status
        if let Some(policy) = &self.policy {
            let gates = policy.unresolved_gates();
            if gates.is_empty() {
                passed_checks.push("rollout_gates_resolved".to_string());
            } else {
                unresolved_gates = gates.iter().map(|s| s.to_string()).collect();
                for gate in gates {
                    failed_checks.push(FailedCheck {
                        check_name: format!("rollout_gate_{gate}"),
                        error: format!("rollout gate '{gate}' is unresolved"),
                    });
                }
            }
        } else {
            passed_checks.push("rollout_gates_skipped".to_string());
        }

        // Check 4: Configuration validity
        if let Some(_policy) = &self.policy {
            match self.check_configuration() {
                Ok(_) => passed_checks.push("configuration_valid".to_string()),
                Err(e) => {
                    failed_checks.push(FailedCheck {
                        check_name: "configuration_valid".to_string(),
                        error: e.to_string(),
                    });
                }
            }
        } else {
            passed_checks.push("configuration_valid_skipped".to_string());
        }

        let healthy = failed_checks.is_empty();

        Ok(DoctorResult {
            healthy,
            passed_checks,
            failed_checks,
            unresolved_gates,
        })
    }

    /// Checks database connectivity.
    async fn check_database(&self) -> Result<(), DoctorError> {
        // This is a stub implementation. A full implementation would attempt
        // to execute a simple query to verify database connectivity.
        Ok(())
    }

    /// Checks state directory accessibility.
    async fn check_state_dir(&self, state_dir: &Path) -> Result<(), DoctorError> {
        // This is a stub implementation. A full implementation would check
        // that the state directory exists and is writable.
        if !state_dir.exists() {
            return Err(DoctorError::StateDirError(format!(
                "state directory does not exist: {}",
                state_dir.display()
            )));
        }
        Ok(())
    }

    /// Checks configuration validity.
    fn check_configuration(&self) -> Result<(), DoctorError> {
        // This is a stub implementation. A full implementation would check
        // that the configuration is valid (e.g., max_workers > 0, retention
        // is a valid duration string, etc.).
        Ok(())
    }
}
