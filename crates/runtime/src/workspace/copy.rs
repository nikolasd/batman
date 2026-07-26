//! Copy isolation handling.
//!
//! Manages copying workspace trees for isolation.

use std::path::Path;
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
    /// Copies the source directory to the destination, excluding .git.
    pub fn copy(&self) -> Result<(), CopyError> {
        std::fs::create_dir_all(&self.destination)?;
        
        for entry in std::fs::read_dir(&self.source)? {
            let entry = entry?;
            let name = entry.file_name();
            let name_str = name.to_str().ok_or_else(|| {
                CopyError::Copy("Invalid file name".to_string())
            })?;
            
            // Skip .git directories
            if name_str == ".git" {
                continue;
            }
            
            let src_path = entry.path();
            let dest_path = self.destination.join(name);
            
            if src_path.is_dir() {
                let sub = CopyIsolation {
                    source: src_path,
                    destination: dest_path,
                };
                sub.copy()?;
            } else if src_path.is_file() {
                std::fs::copy(&src_path, &dest_path)?;
            }
        }
        
        Ok(())
    }
}
