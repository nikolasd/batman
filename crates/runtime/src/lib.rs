pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub mod paths;
pub mod security;

pub use paths::{PathError, RuntimePaths};
pub use security::{SecurityError, StateRoot};
