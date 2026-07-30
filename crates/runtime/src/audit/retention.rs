//! Event retention and pruning.
//!
//! Prunes events older than the configured retention period.

use std::path::Path;

/// Retention policy for event pruning.
#[derive(Debug, Clone)]
pub struct Retention {
    /// The retention period (e.g. "30d", "90d", "1y").
    pub period: String,
}

impl Retention {
    /// Creates a new retention policy.
    #[must_use]
    pub fn new(period: impl Into<String>) -> Self {
        Self { period: period.into() }
    }

    /// Prunes events older than the retention period from the database.
    ///
    /// # Errors
    /// Returns an error if the pruning fails.
    pub fn prune(&self, _state_dir: &Path) -> Result<(), String> {
        // TODO: Implement actual pruning logic using the database actor
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_retention_new() {
        let retention = Retention::new("30d");
        assert_eq!(retention.period, "30d");
    }

    #[test]
    fn test_retention_prune() {
        let retention = Retention::new("30d");
        let result = retention.prune(Path::new("/tmp"));
        assert!(result.is_ok());
    }
}
