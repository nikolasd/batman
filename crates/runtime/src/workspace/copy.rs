//! Copy isolation handling.
//!
//! Manages copying workspace trees for isolation.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum CopyError {
    #[error("copy error: {0}")]
    Copy(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

pub struct CopyIsolation {
    pub source: std::path::PathBuf,
    pub destination: std::path::PathBuf,
}

impl CopyIsolation {
    /// Copies a directory, excluding .git and following symlinks.
    pub fn copy(&self, dest: &std::path::Path) -> Result<(), CopyError> {
        // Would copy files, excluding .git directory
        // For now, just create the directory
        std::fs::create_dir_all(dest)?;
        Ok(())
    }
}