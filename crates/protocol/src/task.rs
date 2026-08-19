//! Stable task reference: the OMP-owned identity of a task as tracked by
//! the runtime.
//!
//! Stores the OMP client instance ID that owns the task plus the monotonic
//! OMP revision. The runtime never creates or edits the OMP task graph; it
//! only mirrors OMP-owned intent.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A stable reference to an OMP-owned task.
///
/// Stores `ownerClientInstanceId` plus the monotonic OMP revision. The
/// runtime never creates or edits the OMP task graph; it only mirrors
/// OMP-owned intent.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct TaskRef {
    /// The OMP client instance ID that owns this task.
    pub owner_client_instance_id: String,
    /// The monotonic OMP revision of the task.
    #[ts(type = "number")]
    pub revision: u64,
}
