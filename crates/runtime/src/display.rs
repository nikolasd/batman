//! Display backend implementations.
//!
//! Provides concrete display backends for rendering Batman output:
//! - HerdrDisplay: Herdr terminal multiplexer backend
//! - TmuxDisplay: Tmux terminal multiplexer backend
//! - TerminalDisplay: Raw terminal backend (degraded capabilities)

use batman_protocol::{DisplayBackend, DisplayConfig, DisplayStatus};
use std::path::Path;
use std::process::Command;
use std::io;
use std::sync::Arc;

/// Result of a command execution — platform-independent, fixture-friendly.
#[derive(Debug, Clone)]
pub struct CommandResult {
    /// Whether the process exited successfully.
    pub success: bool,
    /// Captured stdout bytes.
    pub stdout: Vec<u8>,
    /// Captured stderr bytes.
    pub stderr: Vec<u8>,
}

/// Abstracts process execution for display backends.
///
/// Real executor uses `std::process::Command`; test executors return
/// preconfigured `CommandResult` values so tests never spawn real processes.
pub trait CommandExecutor: Send + Sync {
    /// Execute `program` with `args`, returning a platform-independent result.
    fn execute(&self, program: &str, args: &[&str]) -> io::Result<CommandResult>;
}

/// Real process executor wrapping `std::process::Command`.
pub struct RealCommandExecutor;

impl RealCommandExecutor {
    pub fn new() -> Self {
        RealCommandExecutor
    }
}

impl Default for RealCommandExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandExecutor for RealCommandExecutor {
    fn execute(&self, program: &str, args: &[&str]) -> io::Result<CommandResult> {
        let output = Command::new(program).args(args).output()?;
        Ok(CommandResult {
            success: output.status.success(),
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }
}

/// Trait for display backends.
pub trait DisplayBackendTrait: Send + Sync {
    /// Returns the backend name.
    fn backend_name(&self) -> &str;

    /// Checks if the backend is available and compatible.
    fn is_available(&self) -> bool;

    /// Activates the backend (spawns session, attaches, etc.).
    fn activate(&mut self) -> Result<(), String>;

    /// Returns the current status.
    fn status(&self) -> DisplayStatus;

    /// Returns the backend's version if known.
    fn version(&self) -> Option<String> {
        None
    }
}

/// Herdr display backend.
///
/// Compatibility gate: checks herdr is installed and parses version.
pub struct HerdrDisplay {
    config: DisplayConfig,
    min_version: String,
    session_active: bool,
    session_name: Option<String>,
    executor: Arc<dyn CommandExecutor>,
}
impl HerdrDisplay {
    pub fn new(config: DisplayConfig) -> Self {
        HerdrDisplay {
            config,
            min_version: "0.1.0".to_string(),
            session_active: false,
            session_name: None,
            executor: Arc::new(RealCommandExecutor::new()),
        }
    }

    /// Creates a HerdrDisplay with a custom command executor (for testing).
    pub fn with_executor(config: DisplayConfig, executor: Arc<dyn CommandExecutor>) -> Self {
        HerdrDisplay {
            config,
            min_version: "0.1.0".to_string(),
            session_active: false,
            session_name: None,
            executor,
        }
    }
    /// Checks if Herdr is available and compatible using the injected executor.
    fn check_herdr(&self, min_version: &str) -> bool {
        match self.executor.execute("herdr", &["--version"]) {
            Ok(result) if result.success => {
                let version_str = String::from_utf8_lossy(&result.stdout);
                let version = version_str
                    .split_whitespace()
                    .last()
                    .unwrap_or("")
                    .trim();
                Self::version_gte(version, min_version)
            }
            _ => false,
        }
    }

    /// Simple version comparison: returns true if `current >= minimum`.
    fn version_gte(current: &str, minimum: &str) -> bool {
        let parse_version = |v: &str| -> Vec<u32> {
            v.split('.')
                .filter_map(|s| s.parse::<u32>().ok())
                .collect()
        };

        let current_parts = parse_version(current);
        let min_parts = parse_version(minimum);

        for i in 0..3 {
            let c = current_parts.get(i).copied().unwrap_or(0);
            let m = min_parts.get(i).copied().unwrap_or(0);
            if c > m {
                return true;
            }
            if c < m {
                return false;
            }
        }
        true
    }
    /// Activates herdr by spawning a session using the injected executor.
    fn activate_herdr(&self, session_name: &str) -> Result<(), String> {
        match self.executor.execute("herdr", &["new", session_name]) {
            Ok(result) if result.success => Ok(()),
            Ok(result) => {
                let stderr = String::from_utf8_lossy(&result.stderr);
                Err(format!("herdr exited with error: {stderr}"))
            }
            Err(e) => Err(format!("failed to spawn herdr session: {e}")),
        }
    }
}

impl DisplayBackendTrait for HerdrDisplay {
    fn backend_name(&self) -> &str {
        "herdr"
    }

    fn is_available(&self) -> bool {
        self.check_herdr(&self.min_version)
    }

    fn activate(&mut self) -> Result<(), String> {
        if !self.is_available() {
            return Err("herdr not found or incompatible version".to_string());
        }
        match self.activate_herdr("batman-session") {
            Ok(()) => {
                self.mark_session_active("batman-session".to_string());
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    fn status(&self) -> DisplayStatus {
        DisplayStatus {
            backend: DisplayBackend::Herdr,
            available: self.is_available(),
            active: self.session_active,
            dimensions: None,
        }
    }
}

impl HerdrDisplay {
    pub fn mark_session_active(&mut self, session_name: String) {
        self.session_active = true;
        self.session_name = Some(session_name);
    }

    pub fn mark_session_inactive(&mut self) {
        self.session_active = false;
        self.session_name = None;
    }
}

/// Tmux display backend.
///
/// Compatibility gate: checks tmux is installed and parses version.
/// Minimum required version: 3.0
pub struct TmuxDisplay {
    config: DisplayConfig,
    min_version: String,
    session_active: bool,
    session_name: Option<String>,
    executor: Arc<dyn CommandExecutor>,
}

impl TmuxDisplay {
    pub fn new(config: DisplayConfig) -> Self {
        TmuxDisplay {
            config,
            min_version: "3.0".to_string(),
            session_active: false,
            session_name: None,
            executor: Arc::new(RealCommandExecutor::new()),
        }
    }

    /// Creates a TmuxDisplay with a custom command executor (for testing).
    pub fn with_executor(config: DisplayConfig, executor: Arc<dyn CommandExecutor>) -> Self {
        TmuxDisplay {
            config,
            min_version: "3.0".to_string(),
            session_active: false,
            session_name: None,
            executor,
        }
    }

    /// Checks if tmux is available and compatible using the injected executor.
    fn check_tmux(&self, min_version: &str) -> bool {
        match self.executor.execute("tmux", &["--version"]) {
            Ok(result) if result.success => {
                let version_str = String::from_utf8_lossy(&result.stdout);
                let version = version_str
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or("")
                    .trim()
                    .split(|c: char| !c.is_ascii_digit() && c != '.')
                    .next()
                    .unwrap_or("");
                Self::version_gte(version, min_version)
            }
            _ => false,
        }
    }

    /// Simple version comparison: returns true if `current >= minimum`.
    fn version_gte(current: &str, minimum: &str) -> bool {
        let parse_version = |v: &str| -> Vec<u32> {
            v.split('.')
                .filter_map(|s| s.parse::<u32>().ok())
                .collect()
        };

        let current_parts = parse_version(current);
        let min_parts = parse_version(minimum);

        for i in 0..3 {
            let c = current_parts.get(i).copied().unwrap_or(0);
            let m = min_parts.get(i).copied().unwrap_or(0);
            if c > m {
                return true;
            }
            if c < m {
                return false;
            }
        }
        true
    }

    /// Activates tmux by attaching to a session using the injected executor.
    fn activate_tmux(&self, session_name: &str) -> Result<(), String> {
        match self.executor.execute("tmux", &["new-session", "-d", "-s", session_name]) {
            Ok(result) if result.success => Ok(()),
            Ok(result) => {
                let stderr = String::from_utf8_lossy(&result.stderr);
                Err(format!("tmux exited with error: {stderr}"))
            }
            Err(e) => Err(format!("failed to spawn tmux session: {e}")),
        }
    }
}

impl DisplayBackendTrait for TmuxDisplay {
    fn backend_name(&self) -> &str {
        "tmux"
    }

    fn is_available(&self) -> bool {
        self.check_tmux(&self.min_version)
    }

    fn activate(&mut self) -> Result<(), String> {
        if !self.is_available() {
            return Err("tmux not found or incompatible version".to_string());
        }
        match self.activate_tmux("batman-session") {
            Ok(()) => {
                self.mark_session_active("batman-session".to_string());
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    fn status(&self) -> DisplayStatus {
        DisplayStatus {
            backend: DisplayBackend::Tmux,
            available: self.is_available(),
            active: self.session_active,
            dimensions: None,
        }
    }
}

impl TmuxDisplay {
    /// Marks a session as active.
    pub fn mark_session_active(&mut self, session_name: String) {
        self.session_active = true;
        self.session_name = Some(session_name);
    }

    /// Marks a session as inactive.
    pub fn mark_session_inactive(&mut self) {
        self.session_active = false;
        self.session_name = None;
    }
}

/// Raw terminal display backend (degraded capabilities).
///
/// Always available as a fallback. Does not require external tools.
pub struct TerminalDisplay {
    config: DisplayConfig,
}

impl TerminalDisplay {
    pub fn new(config: DisplayConfig) -> Self {
        TerminalDisplay { config }
    }

    /// Returns terminal dimensions if detectable.
    pub fn detect_dimensions() -> Option<(u16, u16)> {
        // Try to get terminal dimensions via environment or system calls
        // This is a simplified version - real implementation would use libc or termion
        None
    }
}

impl DisplayBackendTrait for TerminalDisplay {
    fn backend_name(&self) -> &str {
        "terminal"
    }

    fn is_available(&self) -> bool {
        // Terminal is always available as a fallback
        true
    }

    fn activate(&mut self) -> Result<(), String> {
        Ok(())
    }

    fn status(&self) -> DisplayStatus {
        let dimensions = Self::detect_dimensions();
        DisplayStatus {
            backend: DisplayBackend::Terminal,
            available: true,
            active: true,
            dimensions,
        }
    }
}

/// Display registry that manages available backends.
pub struct DisplayRegistry {
    backends: Vec<Box<dyn DisplayBackendTrait>>,
}

impl DisplayRegistry {
    pub fn new() -> Self {
        DisplayRegistry {
            backends: Vec::new(),
        }
    }

    /// Registers a display backend.
    pub fn register(&mut self, backend: Box<dyn DisplayBackendTrait>) {
        self.backends.push(backend);
    }

    /// Returns all registered backends.
    pub fn backends(&self) -> &[Box<dyn DisplayBackendTrait>] {
        &self.backends
    }

    /// Selects the best available backend.
    pub fn select_best(&self) -> Option<&dyn DisplayBackendTrait> {
        self.backends.iter().find(|b| b.is_available()).map(|b| b.as_ref())
    }

    /// Returns a mutable reference to a backend by index.
    pub fn backend_mut(&mut self, index: usize) -> Option<&mut (dyn DisplayBackendTrait + 'static)> {
        self.backends.get_mut(index).map(move |b| b.as_mut())
    }
}

impl Default for DisplayRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Display selector with ordered fallback.
pub struct DisplaySelector {
    preferred: Vec<DisplayBackend>,
}

impl DisplaySelector {
    pub fn new(preferred: Vec<DisplayBackend>) -> Self {
        DisplaySelector { preferred }
    }

    /// Selects the first available backend from the preferred list.
    pub fn select<'a>(&self, registry: &'a DisplayRegistry) -> Option<&'a dyn DisplayBackendTrait> {
        for backend in &self.preferred {
            if let Some(registered) = registry.backends().iter().find(|b| {
                b.backend_name() == backend.to_string()
            }) {
                if registered.is_available() {
                    return Some(registered.as_ref());
                }
            }
        }
        None
    }

    /// Returns the index of the first available backend from the preferred list.
    pub fn select_index(&self, registry: &DisplayRegistry) -> Option<usize> {
        for backend in &self.preferred {
            if let Some(index) = registry.backends.iter().position(|b| {
                b.backend_name() == backend.to_string() && b.is_available()
            }) {
                return Some(index);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fake backend for testing.
    struct FakeBackend {
        name: String,
        available: bool,
        activate_result: Result<(), String>,
    }

    impl DisplayBackendTrait for FakeBackend {
        fn backend_name(&self) -> &str {
            &self.name
        }

        fn is_available(&self) -> bool {
            self.available
        }

        fn activate(&mut self) -> Result<(), String> {
            self.activate_result.clone()
        }

        fn status(&self) -> DisplayStatus {
            DisplayStatus::new(DisplayBackend::Terminal, self.available, self.available)
        }
    }

    #[test]
    fn test_display_backend_traits() {
        let herdr = HerdrDisplay::new(DisplayConfig::default());
        assert_eq!(herdr.backend_name(), "herdr");

        let tmux = TmuxDisplay::new(DisplayConfig::default());
        assert_eq!(tmux.backend_name(), "tmux");

        let terminal = TerminalDisplay::new(DisplayConfig::default());
        assert_eq!(terminal.backend_name(), "terminal");
    }

    #[test]
    fn test_terminal_always_available() {
        let terminal = TerminalDisplay::new(DisplayConfig::default());
        assert!(terminal.is_available());
    }

    #[test]
    fn test_version_comparison() {
        assert!(HerdrDisplay::version_gte("0.1.0", "0.1.0"));
        assert!(HerdrDisplay::version_gte("0.2.0", "0.1.0"));
        assert!(!HerdrDisplay::version_gte("0.0.9", "0.1.0"));
        assert!(HerdrDisplay::version_gte("1.0.0", "0.1.0"));
    }

    #[test]
    fn test_display_registry() {
        let mut registry = DisplayRegistry::new();
        registry.register(Box::new(FakeBackend {
            name: "fake1".to_string(),
            available: true,
            activate_result: Ok(()),
        }));
        registry.register(Box::new(FakeBackend {
            name: "fake2".to_string(),
            available: false,
            activate_result: Err("not available".to_string()),
        }));

        assert_eq!(registry.backends().len(), 2);
        assert!(registry.select_best().is_some());
        assert_eq!(registry.select_best().unwrap().backend_name(), "fake1");
    }

    #[test]
    fn test_display_selector_ordered_fallback() {
        let mut registry = DisplayRegistry::new();
        // Register in reverse order: terminal, herdr, tmux
        registry.register(Box::new(FakeBackend {
            name: "terminal".to_string(),
            available: true,
            activate_result: Ok(()),
        }));
        registry.register(Box::new(FakeBackend {
            name: "herdr".to_string(),
            available: false, // herdr not available
            activate_result: Err("not available".to_string()),
        }));
        registry.register(Box::new(FakeBackend {
            name: "tmux".to_string(),
            available: true,
            activate_result: Ok(()),
        }));

        // Selector prefers tmux first, then herdr, then terminal
        let selector = DisplaySelector::new(vec![
            DisplayBackend::Tmux,
            DisplayBackend::Herdr,
            DisplayBackend::Terminal,
        ]);

        // Should select tmux (first in preferred list that's available)
        let selected = selector.select(&registry);
        assert!(selected.is_some());
        assert_eq!(selected.unwrap().backend_name(), "tmux");
    }

    #[test]
    fn test_display_selector_fallback_to_terminal() {
        let mut registry = DisplayRegistry::new();
        // Register only terminal (herdr/tmux not available)
        registry.register(Box::new(FakeBackend {
            name: "terminal".to_string(),
            available: true,
            activate_result: Ok(()),
        }));

        let selector = DisplaySelector::new(vec![
            DisplayBackend::Tmux,
            DisplayBackend::Herdr,
            DisplayBackend::Terminal,
        ]);

        // Should fall back to terminal and activate it
        let selected_index = selector.select_index(&registry);
        assert!(selected_index.is_some());
        let idx = selected_index.unwrap();
        let result = registry.backend_mut(idx).unwrap().activate();
        assert!(result.is_ok());
    }

    #[test]
    fn test_display_selector_no_backend_available() {
        let mut registry = DisplayRegistry::new();
        registry.register(Box::new(FakeBackend {
            name: "fake".to_string(),
            available: false,
            activate_result: Err("not available".to_string()),
        }));

        let selector = DisplaySelector::new(vec![
            DisplayBackend::Tmux,
            DisplayBackend::Herdr,
            DisplayBackend::Terminal,
        ]);

        // Should return None when no backend is available
        let selected = selector.select(&registry);
        assert!(selected.is_none());
    }

    #[test]
    fn test_activate_failure_handling() {
        let mut backend = FakeBackend {
            name: "failing".to_string(),
            available: true,
            activate_result: Err("activation failed".to_string()),
        };

        assert!(backend.activate().is_err());
    }
}
