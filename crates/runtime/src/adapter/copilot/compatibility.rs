//! The Copilot CLI version compatibility table.
//!
//! An installed `copilot` CLI version is not trusted just because it
//! answers `--version`: this adapter only proceeds past `initialize` for a
//! CLI version it has been empirically verified against (exact match,
//! never a "nearby" patch version assumed compatible), and only for a
//! negotiated ACP protocol version its own `normalize.rs` understands the
//! field names of. `1.0.73` was the version installed on the build
//! machine at the start of this work; the CLI's own background
//! auto-updater (`copilot update`) then updated it to `1.0.75`
//! mid-development. Both are listed below because both were empirically
//! reprobed with a real `initialize` handshake and confirmed to negotiate
//! identical ACP v1 `agentCapabilities`/`agentInfo` field names (see
//! `tests/copilot_adapter.rs`'s real-binary tests) -- adding a newer CLI
//! release here always requires that same kind of empirical
//! verification, not a guess.

/// One verified `(CLI version, negotiated ACP protocol version)` pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CopilotCompatibilityEntry {
    pub cli_version: &'static str,
    pub acp_protocol_version: u64,
}

/// The exact CLI versions this adapter has been verified against, and the
/// ACP protocol version each negotiates.
pub const COPILOT_KNOWN_CLI_VERSIONS: &[CopilotCompatibilityEntry] = &[
    CopilotCompatibilityEntry {
        cli_version: "1.0.73",
        acp_protocol_version: 1,
    },
    CopilotCompatibilityEntry {
        cli_version: "1.0.75",
        acp_protocol_version: 1,
    },
];

/// The inclusive range of ACP protocol versions this adapter's
/// `normalize.rs` understands the v1 field names of. A negotiated version
/// outside this range is refused with `AdapterError::incompatible_version`.
pub const COPILOT_MIN_ACP_PROTOCOL_VERSION: u64 = 1;
pub const COPILOT_MAX_ACP_PROTOCOL_VERSION: u64 = 1;

/// Whether `cli_version` is a CLI version this adapter has verified
/// end-to-end (exact string match only).
#[must_use]
pub fn copilot_cli_version_known(cli_version: &str) -> bool {
    COPILOT_KNOWN_CLI_VERSIONS
        .iter()
        .any(|entry| entry.cli_version == cli_version)
}

/// Whether `protocol_version` is one this adapter's client/normalize code
/// understands the field names of.
#[must_use]
pub fn copilot_acp_protocol_version_supported(protocol_version: u64) -> bool {
    (COPILOT_MIN_ACP_PROTOCOL_VERSION..=COPILOT_MAX_ACP_PROTOCOL_VERSION)
        .contains(&protocol_version)
}
