//! Copy isolation handling.
//!
//! Manages copying workspace trees for isolation.
//! Copies without following symlinks - symlinks are recreated as symlinks.

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
    /// Does NOT follow symlinks - recreates them as symlinks.
    pub fn copy(&self) -> Result<(), CopyError> {
        std::fs::create_dir_all(&self.destination)?;

        for entry in std::fs::read_dir(&self.source)? {
            let entry = entry?;
            let name = entry.file_name();
            let name_str = name
                .to_str()
                .ok_or_else(|| CopyError::Copy("Invalid file name".to_string()))?;

            // Skip .git directories
            if name_str == ".git" {
                continue;
            }

            let src_path = entry.path();
            let dest_path = self.destination.join(name);

            // Use symlink_metadata to check the type WITHOUT following symlinks
            let metadata = std::fs::symlink_metadata(&src_path)?;
            let file_type = metadata.file_type();

            // Check symlinks FIRST (before is_dir/is_file which follow symlinks)
            if file_type.is_symlink() {
                // Recreate symlinks as symlinks (don't follow them)
                let target = std::fs::read_link(&src_path)?;
                #[cfg(unix)]
                {
                    std::os::unix::fs::symlink(&target, &dest_path)?;
                }
                #[cfg(windows)]
                {
                    // Check if the target is a directory (by reading the link)
                    if target.is_dir() {
                        std::os::windows::fs::symlink_dir(&target, &dest_path)?;
                    } else {
                        std::os::windows::fs::symlink_file(&target, &dest_path)?;
                    }
                }
            } else if file_type.is_dir() {
                // Recursively copy subdirectories (not symlinks to directories)
                let sub = CopyIsolation {
                    source: src_path,
                    destination: dest_path,
                };
                sub.copy()?;
            } else if file_type.is_file() {
                // Copy regular files
                std::fs::copy(&src_path, &dest_path)?;
            }
            // Skip other special files (devices, sockets, etc.)
        }

        Ok(())
    }
}
