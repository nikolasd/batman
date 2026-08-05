//! Worker profile and worker records.
//!
//! `WorkerProfileRef` is an immutable snapshot of a harness profile
//! (fingerprint, adapter, model, permissions). `Worker` carries this
//! reference plus metadata.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::Timestamp;
use crate::ids::WorkerId;

/// An immutable snapshot of a harness profile.
///
/// Contains the fingerprint, adapter name, model, and a JSON permission
/// envelope. Later adapter configuration resolves and stores the complete
/// profile snapshot without changing these identity fields.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct WorkerProfileRef {
    /// The worker profile identifier (UUIDv7).
    pub id: WorkerId,
    /// A fingerprint of the harness binary + version (e.g. `sha256:…`).
    pub fingerprint: String,
    /// The adapter name (e.g. `claude`, `codex`, `copilot`, `ompNative`).
    pub adapter: String,
    /// The model identifier (e.g. `claude-sonnet-4-20250514`).
    pub model: String,
    /// A JSON object describing the permissions granted by this profile.
    #[serde(rename = "permissionEnvelope")]
    #[ts(type = "object")]
    pub permission_envelope: serde_json::Value,
}

/// A logical OMP identity wrapping one harness/profile and optional parent.
///
/// Carries an immutable [`WorkerProfileRef`] reference; replacing a harness
/// creates a new worker and run while retaining task identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct Worker {
    /// The worker identifier (UUIDv7).
    pub worker_id: WorkerId,
    /// The immutable profile reference.
    #[serde(rename = "profileRef")]
    pub profile_ref: WorkerProfileRef,
    /// The parent worker ID, if this worker was spawned as a child.
    #[serde(rename = "parentWorkerId", skip_serializing_if = "Option::is_none")]
    pub parent_worker_id: Option<WorkerId>,
    /// When the worker was created (UTC RFC 3339).
    #[serde(rename = "createdAt")]
    pub created_at: Timestamp,
}
