//! The durable SQLite journal: a single-owner actor thread holding the
//! `rusqlite::Connection`, driven over a bounded async command channel so
//! every write returns to its caller only after its transaction commits.
//!
//! The only event type the journal accepts is
//! [`crate::security::redaction::PersistableEvent`], obtainable only via
//! [`crate::security::redaction::Redactor::sanitize`] -- there is no raw
//! event or `serde_json::Value` append API.

mod actor;
pub(crate) mod migrations;
mod models;

pub use actor::{DatabaseHandle, DbError, DomainClosure};
pub use models::{Diagnostics, OperationIntent, ReplayedEvent};
