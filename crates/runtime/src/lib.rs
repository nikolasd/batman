pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub mod db;
pub mod ipc;
pub mod lifecycle;
pub mod paths;
pub mod security;

pub use db::{DatabaseHandle, DbError};
pub use ipc::{IpcError, Server, ServerConfig};
pub use lifecycle::should_idle_shutdown;
pub use paths::{PathError, RuntimePaths};
pub use security::{SecurityError, StateRoot};
