//! Event export in JSONL format.
//!
//! Exports events from the database to a JSONL file.


/// Event export configuration.
#[derive(Debug, Clone)]
pub struct Export {
    /// The repository path.
    pub repo: String,
    /// The state directory.
    pub state_dir: String,
    /// The start time (optional).
    pub from: Option<String>,
    /// The end time (optional).
    pub to: Option<String>,
    /// The output file path.
    pub output: String,
}

impl Export {
    /// Creates a new export configuration.
    #[must_use]
    pub fn new(
        repo: impl Into<String>,
        state_dir: impl Into<String>,
        output: impl Into<String>,
    ) -> Self {
        Self {
            repo: repo.into(),
            state_dir: state_dir.into(),
            from: None,
            to: None,
            output: output.into(),
        }
    }

    /// Exports events to a JSONL file.
    ///
    /// # Errors
    /// Returns an error if the export fails.
    pub fn export(&self) -> Result<(), String> {
        // TODO: Implement actual export logic using the database actor
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_export_new() {
        let export = Export::new("repo", "state_dir", "output.jsonl");
        assert_eq!(export.repo, "repo");
        assert_eq!(export.state_dir, "state_dir");
        assert_eq!(export.output, "output.jsonl");
        assert!(export.from.is_none());
        assert!(export.to.is_none());
    }

    #[test]
    fn test_export_with_time_range() {
        let mut export = Export::new("repo", "state_dir", "output.jsonl");
        export.from = Some("2023-01-01".to_string());
        export.to = Some("2023-12-31".to_string());
        assert_eq!(export.from, Some("2023-01-01".to_string()));
        assert_eq!(export.to, Some("2023-12-31".to_string()));
    }

    #[test]
    fn test_export() {
        let export = Export::new("repo", "state_dir", "output.jsonl");
        let result = export.export();
        assert!(result.is_ok());
    }
}
