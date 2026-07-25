//! Workspace lease arbitration and materialization service.

mod lease;
mod materialize;
mod git;
mod copy;
mod inspect;
mod apply;

pub use lease::{CreatedLease, LeaseError, LeaseService};
pub use materialize::{MaterializerError, WorkspaceMaterializer};
pub use inspect::{InspectError, WorkspaceInspector};
pub use apply::{ApplyError, WorkspaceApplier};
