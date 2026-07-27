//! Tmux display backend tests using injected command executors.

use batman_protocol::{DisplayBackend, DisplayConfig};
use batman_runtime::display::{CommandExecutor, CommandResult, DisplayBackendTrait, TmuxDisplay};
use std::io;
use std::sync::{Arc, Mutex};

/// Mock command executor for testing.
struct MockCommandExecutor {
    results: Vec<MockResult>,
    calls: Mutex<Vec<(String, Vec<String>)>>,
    call_count: std::sync::atomic::AtomicUsize,
}

enum MockResult {
    Success(Vec<u8>),
    Failure(Vec<u8>),
    SpawnError(String),
}

impl MockCommandExecutor {
    fn new() -> Self {
        MockCommandExecutor {
            results: Vec::new(),
            calls: Mutex::new(Vec::new()),
            call_count: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    fn push_success(&mut self, stdout: Vec<u8>) {
        self.results.push(MockResult::Success(stdout));
    }

    fn push_failure(&mut self, stderr: Vec<u8>) {
        self.results.push(MockResult::Failure(stderr));
    }

    fn push_spawn_error(&mut self, msg: &str) {
        self.results.push(MockResult::SpawnError(msg.to_string()));
    }

    fn recorded_calls(&self) -> Vec<(String, Vec<String>)> {
        self.calls.lock().unwrap().clone()
    }
    fn assert_command_invoked(&self, program: &str, expected_args: &[&str]) {
        let calls = self.recorded_calls();
        let found = calls.iter().any(|(p, args)| {
            p == program && args.len() == expected_args.len() &&
                args.iter().zip(expected_args.iter()).all(|(a, e)| *a == **e)
        });
        assert!(
            found,
            "Expected command '{}' with args {:?}, but recorded calls were: {:?}",
            program, expected_args, calls
        );
    }

    fn assert_version_check_invoked(&self) {
        self.assert_command_invoked("tmux", &["--version"]);
    }

    fn assert_session_creation_invoked(&self) {
        self.assert_command_invoked("tmux", &["new-session", "-d", "-s", "batman-session"]);
    }
}

impl CommandExecutor for MockCommandExecutor {
    fn execute(&self, program: &str, args: &[&str]) -> io::Result<CommandResult> {
        let idx = self.call_count.load(std::sync::atomic::Ordering::Relaxed);
        self.call_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        {
            let mut calls = self.calls.lock().unwrap();
            calls.push((
                program.to_string(),
                args.iter().map(|s| s.to_string()).collect(),
            ));
        }
        match self.results.get(idx) {
            Some(MockResult::Success(stdout)) => Ok(CommandResult {
                success: true,
                stdout: stdout.clone(),
                stderr: Vec::new(),
            }),
            Some(MockResult::Failure(stderr)) => Ok(CommandResult {
                success: false,
                stdout: Vec::new(),
                stderr: stderr.clone(),
            }),
            Some(MockResult::SpawnError(msg)) => {
                Err(io::Error::new(io::ErrorKind::NotFound, msg.clone()))
            }
            None => Err(io::Error::new(io::ErrorKind::Other, "no more results")),
        }
    }
}

#[test]
fn tmux_display_creates_with_config() {
    let config = DisplayConfig::default();
    let tmux = TmuxDisplay::new(config);
    assert_eq!(tmux.backend_name(), "tmux");
}

#[test]
fn tmux_display_with_mock_executor_available() {
    let mut mock = MockCommandExecutor::new();
    mock.push_success(b"tmux 3.3".to_vec());

    let config = DisplayConfig::default();
    let tmux = TmuxDisplay::with_executor(config, Arc::new(mock));

    assert!(tmux.is_available());
    assert_eq!(tmux.backend_name(), "tmux");
}

#[test]
fn tmux_display_with_mock_executor_unavailable_old_version() {
    let mut mock = MockCommandExecutor::new();
    mock.push_success(b"tmux 2.9".to_vec());

    let config = DisplayConfig::default();
    let tmux = TmuxDisplay::with_executor(config, Arc::new(mock));

    assert!(!tmux.is_available());
}

#[test]
fn tmux_display_with_mock_executor_unavailable_command_failure() {
    let mut mock = MockCommandExecutor::new();
    mock.push_failure(b"command not found".to_vec());

    let config = DisplayConfig::default();
    let tmux = TmuxDisplay::with_executor(config, Arc::new(mock));

    assert!(!tmux.is_available());
}

#[test]
fn tmux_display_with_mock_executor_unavailable_spawn_error() {
    let mut mock = MockCommandExecutor::new();
    mock.push_spawn_error("tmux not installed");

    let config = DisplayConfig::default();
    let tmux = TmuxDisplay::with_executor(config, Arc::new(mock));

    assert!(!tmux.is_available());
}

#[test]
fn tmux_display_activate_success() {
    let mut mock = MockCommandExecutor::new();
    // 1st: is_available check
    mock.push_success(b"tmux 3.3".to_vec());
    // 2nd: activate -> is_available check
    mock.push_success(b"tmux 3.3".to_vec());
    // 3rd: activate -> activate_tmux
    mock.push_success(Vec::new());

    let config = DisplayConfig::default();
    let mock = Arc::new(mock);
    let mut tmux = TmuxDisplay::with_executor(config, mock.clone());

    assert!(tmux.is_available());
    assert!(tmux.activate().is_ok());
    mock.assert_session_creation_invoked();
}

#[test]
fn tmux_display_activate_failure_nonzero_exit() {
    let mut mock = MockCommandExecutor::new();
    // 1st: is_available check
    mock.push_success(b"tmux 3.3".to_vec());
    // 2nd: activate -> is_available check
    mock.push_success(b"tmux 3.3".to_vec());
    // 3rd: activate -> activate_tmux - fails
    mock.push_failure(b"error: session creation failed".to_vec());

    let config = DisplayConfig::default();
    let mock = Arc::new(mock);
    let mut tmux = TmuxDisplay::with_executor(config, mock.clone());

    assert!(tmux.is_available());
    let result = tmux.activate();
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("tmux exited with error"));
}

#[test]
fn tmux_display_activate_failure_spawn_error() {
    let mut mock = MockCommandExecutor::new();
    // 1st: is_available check
    mock.push_success(b"tmux 3.3".to_vec());
    // 2nd: activate -> is_available check
    mock.push_success(b"tmux 3.3".to_vec());
    // 3rd: activate -> activate_tmux - spawn error
    mock.push_spawn_error("tmux not found");

    let config = DisplayConfig::default();
    let mock = Arc::new(mock);
    let mut tmux = TmuxDisplay::with_executor(config, mock.clone());

    assert!(tmux.is_available());
    let result = tmux.activate();
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("failed to spawn tmux session"));
}

#[test]
fn tmux_display_status_initial() {
    let mut mock = MockCommandExecutor::new();
    mock.push_success(b"tmux 3.3".to_vec());

    let config = DisplayConfig::default();
    let tmux = TmuxDisplay::with_executor(config, Arc::new(mock));

    let status = tmux.status();
    assert_eq!(status.backend, DisplayBackend::Tmux);
    assert!(status.available);
    assert!(!status.active);
}

#[test]
fn tmux_display_status_after_activation() {
    let mut mock = MockCommandExecutor::new();
    mock.push_success(b"tmux 3.3".to_vec());

    let config = DisplayConfig::default();
    let mut tmux = TmuxDisplay::with_executor(config, Arc::new(mock));

    tmux.mark_session_active("test-session".to_string());
    let status = tmux.status();
    assert!(status.active);
    assert_eq!(status.backend, DisplayBackend::Tmux);
}

#[test]
fn tmux_display_status_after_deactivation() {
    let mut mock = MockCommandExecutor::new();
    mock.push_success(b"tmux 3.3".to_vec());

    let config = DisplayConfig::default();
    let mut tmux = TmuxDisplay::with_executor(config, Arc::new(mock));

    tmux.mark_session_active("test-session".to_string());
    tmux.mark_session_inactive();
    let status = tmux.status();
    assert!(!status.active);
}

#[test]
fn tmux_display_version_parsing() {
    let mut mock = MockCommandExecutor::new();
    mock.push_success(b"tmux 3.3".to_vec());

    let config = DisplayConfig::default();
    let tmux = TmuxDisplay::with_executor(config, Arc::new(mock));

    assert!(tmux.is_available());
}

#[test]
fn tmux_display_version_too_old() {
    let mut mock = MockCommandExecutor::new();
    mock.push_success(b"tmux 2.9".to_vec());

    let config = DisplayConfig::default();
    let tmux = TmuxDisplay::with_executor(config, Arc::new(mock));

    assert!(!tmux.is_available());
}

#[test]
fn tmux_display_verifies_version_check_command() {
    let mut mock = MockCommandExecutor::new();
    mock.push_success(b"tmux 3.3".to_vec());

    let config = DisplayConfig::default();
    let mock = Arc::new(mock);
    let tmux = TmuxDisplay::with_executor(config, mock.clone());

    tmux.is_available();
    mock.assert_version_check_invoked();
}

#[test]
fn tmux_display_verifies_session_creation_command() {
    let mut mock = MockCommandExecutor::new();
    // 1st: is_available check
    mock.push_success(b"tmux 3.3".to_vec());
    // 2nd: activate -> is_available check
    mock.push_success(b"tmux 3.3".to_vec());
    // 3rd: activate -> activate_tmux
    mock.push_success(Vec::new());

    let config = DisplayConfig::default();
    let mock = Arc::new(mock);
    let mut tmux = TmuxDisplay::with_executor(config, mock.clone());

    assert!(tmux.is_available());
    assert!(tmux.activate().is_ok());
    mock.assert_session_creation_invoked();
}

#[test]
fn tmux_display_verifies_no_extra_commands_on_unavailable() {
    let mut mock = MockCommandExecutor::new();
    mock.push_spawn_error("not found");

    let config = DisplayConfig::default();
    let mock = Arc::new(mock);
    let tmux = TmuxDisplay::with_executor(config, mock.clone());

    assert!(!tmux.is_available());
    let calls = mock.recorded_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "tmux");
    assert_eq!(calls[0].1, vec!["--version"]);
}
