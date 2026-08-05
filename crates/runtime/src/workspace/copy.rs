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
    /// A copy exceeded its configured ceiling. Copy isolation duplicates a
    /// whole working tree onto the same disk the daemon runs from, so an
    /// unbounded copy of an accidentally huge repository is a
    /// disk-exhaustion vector against the host. Refusing partway is the
    /// safe outcome; the partial destination is removed by the caller.
    #[error("copy exceeded {limit} ceiling: {value} > {ceiling}")]
    CeilingExceeded {
        limit: &'static str,
        value: u64,
        ceiling: u64,
    },
}

/// Default ceilings for copy isolation. Large enough that a normal source
/// repository (including build output a developer left in the tree) copies
/// without complaint, small enough that a runaway copy fails in seconds
/// rather than filling the disk. Overridable via the `workspace` config key.
pub const DEFAULT_COPY_MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;
pub const DEFAULT_COPY_MAX_FILES: u64 = 200_000;

/// Running totals for one `copy()` call. Held by the public entry point and
/// borrowed by the recursive walker, because the walk descends into
/// subdirectories and a per-directory counter would reset on every descent.
struct CopyBudget {
    bytes: u64,
    files: u64,
    max_bytes: u64,
    max_files: u64,
}

impl CopyBudget {
    /// Charges one file against the budget *before* it is copied, so the
    /// ceiling bounds what lands on disk rather than reporting it after.
    fn charge(&mut self, bytes: u64) -> Result<(), CopyError> {
        self.files += 1;
        if self.files > self.max_files {
            return Err(CopyError::CeilingExceeded {
                limit: "file count",
                value: self.files,
                ceiling: self.max_files,
            });
        }
        self.bytes += bytes;
        if self.bytes > self.max_bytes {
            return Err(CopyError::CeilingExceeded {
                limit: "byte size",
                value: self.bytes,
                ceiling: self.max_bytes,
            });
        }
        Ok(())
    }
}

pub struct CopyIsolation {
    pub source: std::path::PathBuf,
    pub destination: std::path::PathBuf,
    pub max_bytes: u64,
    pub max_files: u64,
}

impl CopyIsolation {
    /// Copies the source directory to the destination, excluding .git.
    /// Does NOT follow symlinks - recreates them as symlinks.
    ///
    /// # Errors
    /// Returns [`CopyError::CeilingExceeded`] if the tree exceeds
    /// `max_bytes` or `max_files`; the partially written destination is
    /// removed before returning so no caller sees a truncated workspace.
    pub fn copy(&self) -> Result<(), CopyError> {
        let mut budget = CopyBudget {
            bytes: 0,
            files: 0,
            max_bytes: self.max_bytes,
            max_files: self.max_files,
        };
        let result = copy_tree(&self.source, &self.destination, &mut budget);
        if result.is_err() {
            // A half-copied workspace is worse than none: an adapter would
            // run against a silently incomplete checkout.
            let _ = std::fs::remove_dir_all(&self.destination);
        }
        result
    }
}

fn copy_tree(
    source: &std::path::Path,
    destination: &std::path::Path,
    budget: &mut CopyBudget,
) -> Result<(), CopyError> {
    std::fs::create_dir_all(destination)?;

    for entry in std::fs::read_dir(source)? {
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
        let dest_path = destination.join(name);

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
            copy_tree(&src_path, &dest_path, budget)?;
        } else if file_type.is_file() {
            // Charge before copying: the ceiling bounds disk usage, so it
            // must be checked against the size we are about to write.
            budget.charge(metadata.len())?;
            std::fs::copy(&src_path, &dest_path)?;
        }
        // Skip other special files (devices, sockets, etc.)
    }

    Ok(())
}
