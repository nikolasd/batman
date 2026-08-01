//! Integration tests for crash recovery.
//!
//! These tests verify the RecoveryCoordinator's behavior with the CURRENT STUB
//! implementation. A full implementation would query the database for stuck runs
//! and transition them to terminal states.
//!
//! Tests run with --test-threads=1 since they manipulate real process state.

use std::sync::Arc;
use std::time::Duration;

use tempfile::TempDir;

use batman_runtime::db::DatabaseHandle;

#[tokio::test]
async fn recovery_returns_empty_when_no_stuck_runs() {
    // Note: This test verifies the CURRENT STUB implementation.
    // A full implementation would query the database for stuck runs and transition them.
    let state_dir = TempDir::new().unwrap();
    let db_path = state_dir.path().join("runtime.db");

    let db = DatabaseHandle::start(db_path).await.unwrap();
    let coordinator = batman_runtime::recovery::RecoveryCoordinator::with_defaults(Arc::new(db));
    let result = coordinator.recover().await.unwrap();

    // Stub implementation returns no recovered runs
    assert_eq!(result.recovered_count, 0);
    assert!(result.recovered_runs.is_empty());
}

#[tokio::test]
async fn recovery_config_default_values() {
    // Verify default config values
    let config = batman_runtime::recovery::RecoveryConfig::default();
    assert_eq!(config.stuck_threshold, Duration::from_secs(300));
    assert!(!config.recover_paused);
    assert!(!config.recover_waiting);
}

#[tokio::test]
async fn recovery_config_custom_values() {
    // Verify custom config values
    let config = batman_runtime::recovery::RecoveryConfig {
        stuck_threshold: Duration::from_secs(600),
        recover_paused: true,
        recover_waiting: true,
    };
    assert_eq!(config.stuck_threshold, Duration::from_secs(600));
    assert!(config.recover_paused);
    assert!(config.recover_waiting);
}
