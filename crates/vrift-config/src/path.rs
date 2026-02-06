//! Path normalization utilities for IPC and cross-process communication.
//!
//! All paths crossing process boundaries (CLI → Daemon, Shim → Daemon) should
//! be normalized through these functions to ensure consistent path resolution.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// Normalize a path for IPC communication.
///
/// Resolves symlinks and returns an absolute path.
/// This should be used for all paths sent between processes.
///
/// # Example
/// ```ignore
/// let abs_path = normalize_for_ipc(".").unwrap();
/// assert!(abs_path.is_absolute());
/// ```
pub fn normalize_for_ipc(path: impl AsRef<Path>) -> Result<PathBuf> {
    let path = path.as_ref();
    path.canonicalize()
        .with_context(|| format!("Failed to resolve path for IPC: {}", path.display()))
}

/// Normalize a path where the target file may not exist yet.
///
/// Canonicalizes the parent directory and appends the filename.
/// Useful for output paths like manifest files that will be created.
///
/// # Example
/// ```ignore
/// let out_path = normalize_nonexistent("/existing/dir/new_file.lmdb").unwrap();
/// ```
pub fn normalize_nonexistent(path: impl AsRef<Path>) -> Result<PathBuf> {
    let path = path.as_ref();
    let filename = path.file_name().context("Path has no filename")?;

    if let Some(parent) = path.parent() {
        if parent.as_os_str().is_empty() {
            // Relative file in current directory
            let cwd = std::env::current_dir().context("Failed to get current directory")?;
            Ok(cwd.join(filename))
        } else {
            let canonical_parent = parent.canonicalize().with_context(|| {
                format!("Failed to resolve parent directory: {}", parent.display())
            })?;
            Ok(canonical_parent.join(filename))
        }
    } else {
        // Path is just a filename
        let cwd = std::env::current_dir().context("Failed to get current directory")?;
        Ok(cwd.join(filename))
    }
}

/// Normalize path, falling back to the original if canonicalization fails.
///
/// This is useful when the path might not exist and that's acceptable.
pub fn normalize_or_original(path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_normalize_existing_path() {
        let temp = tempdir().unwrap();
        let file_path = temp.path().join("test.txt");
        fs::write(&file_path, "test").unwrap();

        let normalized = normalize_for_ipc(&file_path).unwrap();
        assert!(normalized.is_absolute());
        assert!(normalized.exists());
    }

    #[test]
    fn test_normalize_relative_path() {
        let normalized = normalize_for_ipc(".").unwrap();
        assert!(normalized.is_absolute());
    }

    #[test]
    fn test_normalize_nonexistent_creates_valid_path() {
        let temp = tempdir().unwrap();
        let new_file = temp.path().join("new_file.lmdb");

        let normalized = normalize_nonexistent(&new_file).unwrap();
        assert!(normalized.is_absolute());
        assert_eq!(normalized.file_name().unwrap(), "new_file.lmdb");
    }

    #[test]
    fn test_normalize_or_original_returns_original_on_failure() {
        let fake_path = Path::new("/nonexistent/path/file.txt");
        let result = normalize_or_original(fake_path);
        assert_eq!(result, fake_path);
    }
}
