//! Display backend contracts.
//!
//! Defines the display backend types and configuration for rendering
//! Batman output in different environments (Herdr, Tmux, Terminal).

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use schemars::JsonSchema;

/// Supported display backends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum DisplayBackend {
    /// Herdr terminal multiplexer backend.
    Herdr,
    /// Tmux terminal multiplexer backend.
    Tmux,
    /// Raw terminal backend (degraded capabilities).
    Terminal,
}

impl std::fmt::Display for DisplayBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DisplayBackend::Herdr => write!(f, "herdr"),
            DisplayBackend::Tmux => write!(f, "tmux"),
            DisplayBackend::Terminal => write!(f, "terminal"),
        }
    }
}

/// Display configuration.
#[derive(Debug, Clone, Serialize, Deserialize, TS, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct DisplayConfig {
    /// The backend to use.
    pub backend: DisplayBackend,
    /// Optional width override (None = auto-detect).
    pub width: Option<u16>,
    /// Optional height override (None = auto-detect).
    pub height: Option<u16>,
}

impl Default for DisplayConfig {
    fn default() -> Self {
        DisplayConfig {
            backend: DisplayBackend::Terminal,
            width: None,
            height: None,
        }
    }
}

/// Display status information.
#[derive(Debug, Clone, Serialize, Deserialize, TS, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct DisplayStatus {
    /// The backend in use.
    pub backend: DisplayBackend,
    /// Whether the backend is available.
    pub available: bool,
    /// Whether the backend is currently active.
    pub active: bool,
    /// Terminal dimensions if known.
    pub dimensions: Option<(u16, u16)>,
}

impl DisplayStatus {
    pub fn new(backend: DisplayBackend, available: bool, active: bool) -> Self {
        DisplayStatus {
            backend,
            available,
            active,
            dimensions: None,
        }
    }
}
