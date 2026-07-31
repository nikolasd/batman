//! Integration tests for `batcave doctor` CLI command.
//!
//! These tests verify the doctor command's behavior with various inputs:
//! - Valid repository with proper state directory
//! - Invalid/missing repository
//! - Missing state directory
//! - JSON output mode
//! - Error handling

use std::path::Path;
use std::process::{Command, Stdio};

use serde_json::Value;
use tempfile::TempDir;

const BATCAVE: &str = env!("CARGO_BIN_EXE_batcave");

struct Fixture {
    state: TempDir,
    repo: TempDir,
}

impl Fixture {
    fn new() -> Self {
        let state = tempfile::Builder::new()
            .prefix("bat-doc-s-")
            .tempdir_in("/tmp")
            .unwrap();
        let repo = tempfile::Builder::new()
            .prefix("bat-doc-r-")
            .tempdir_in("/tmp")
            .unwrap();
        std::fs::create_dir(repo.path().join(".git")).unwrap();
        Self { state, repo }
    }

    fn state_dir(&self) -> &Path {
        self.state.path()
    }

    fn repo_dir(&self) -> &Path {
        self.repo.path()
    }

    fn doctor(&self, json: bool) -> Command {
        let mut cmd = Command::new(BATCAVE);
        cmd.arg("doctor")
            .arg("--state-dir")
            .arg(self.state_dir())
            .arg("--repo")
            .arg(self.repo_dir());
        if json {
            cmd.arg("--json");
        }
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        cmd
    }
}

#[test]
fn doctor_with_missing_db_returns_failure() {
    let fixture = Fixture::new();
    let mut cmd = fixture.doctor(false);
    let output = cmd.output().expect("failed to execute doctor");

    // Should fail because no database exists yet
    assert!(!output.status.success());
}

#[test]
fn doctor_json_mode_with_missing_db() {
    let fixture = Fixture::new();
    let mut cmd = fixture.doctor(true);
    let output = cmd.output().expect("failed to execute doctor");

    // Should fail and output JSON
    assert!(!output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    // Should be valid JSON even on failure
    let parsed: Result<Value, _> = serde_json::from_str(&stdout);
    assert!(parsed.is_ok(), "JSON output should be parseable: {stdout}");

    let json = parsed.unwrap();
    assert_eq!(json.get("healthy").and_then(|v| v.as_bool()), Some(false));
    assert!(json.get("error").is_some() || json.get("failed_checks").is_some());
}

#[test]
fn doctor_with_nonexistent_state_dir() {
    let fixture = Fixture::new();
    let mut cmd = Command::new(BATCAVE);
    cmd.arg("doctor")
        .arg("--state-dir")
        .arg("/tmp/does/not/exist")
        .arg("--repo")
        .arg(fixture.repo_dir())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let output = cmd.output().expect("failed to execute doctor");

    // Should fail with helpful error
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("does not exist") || stderr.contains("state directory") || stderr.contains("No such file"),
        "Expected state directory or file error, got: {stderr}"
    );
}

#[test]
fn doctor_with_nonexistent_repo() {
    let fixture = Fixture::new();
    let mut cmd = Command::new(BATCAVE);
    cmd.arg("doctor")
        .arg("--state-dir")
        .arg(fixture.state_dir())
        .arg("--repo")
        .arg("/tmp/does/not/exist/repo")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let output = cmd.output().expect("failed to execute doctor");

    // Should fail
    assert!(!output.status.success());
}
