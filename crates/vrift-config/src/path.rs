//! Path normalization utilities for IPC and cross-process communication.
//!
//! All paths crossing process boundaries (CLI → Daemon, Inception Layer → Daemon) should
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

/// Check if a path is within a directory (security check for path traversal).
///
/// Both paths are canonicalized before comparison to handle symlinks and `..`.
/// Returns false if either path cannot be canonicalized.
///
/// # Security
/// Use this to validate user-provided paths don't escape expected directories.
pub fn is_within_directory(path: impl AsRef<Path>, dir: impl AsRef<Path>) -> bool {
    let path = path.as_ref();
    let dir = dir.as_ref();

    match (path.canonicalize(), dir.canonicalize()) {
        (Ok(canonical_path), Ok(canonical_dir)) => canonical_path.starts_with(&canonical_dir),
        _ => false,
    }
}

/// Compute relative path from base to target.
///
/// Returns the relative path from `base` to `target`, or the absolute target
/// if they don't share a common prefix.
pub fn compute_relative_path(base: impl AsRef<Path>, target: impl AsRef<Path>) -> PathBuf {
    let base = base.as_ref();
    let target = target.as_ref();

    // Try to strip the base prefix
    if let Ok(relative) = target.strip_prefix(base) {
        relative.to_path_buf()
    } else {
        // No common prefix, return absolute target
        target.to_path_buf()
    }
}

/// Strip a prefix from a path safely.
///
/// Returns the relative portion after the prefix, or None if the path
/// doesn't start with the prefix.
///
/// # Example
/// ```ignore
/// let rel = strip_prefix_safe("/vfs/project/src/main.rs", "/vfs/project");
/// assert_eq!(rel, Some(PathBuf::from("src/main.rs")));
/// ```
pub fn strip_prefix_safe(path: impl AsRef<Path>, prefix: impl AsRef<Path>) -> Option<PathBuf> {
    let path = path.as_ref();
    let prefix = prefix.as_ref();

    path.strip_prefix(prefix).ok().map(|p| p.to_path_buf())
}

/// Validate that path is within directory, returning Result for security checks.
///
/// Unlike `is_within_directory`, this returns a detailed error on failure.
///
/// # Example
/// ```ignore
/// let safe_path = ensure_within("/project/src/file.rs", "/project")?;
/// // Returns PathBuf on success, Error with context on failure
/// ```
pub fn ensure_within(path: impl AsRef<Path>, dir: impl AsRef<Path>) -> Result<PathBuf> {
    let path = path.as_ref();
    let dir = dir.as_ref();

    let canonical_path = path
        .canonicalize()
        .with_context(|| format!("Cannot resolve path: {}", path.display()))?;
    let canonical_dir = dir
        .canonicalize()
        .with_context(|| format!("Cannot resolve directory: {}", dir.display()))?;

    if canonical_path.starts_with(&canonical_dir) {
        Ok(canonical_path)
    } else {
        anyhow::bail!(
            "Path '{}' is outside directory '{}'",
            path.display(),
            dir.display()
        )
    }
}

/// Normalize a path relative to a project root.
///
/// If the path is absolute and within the root, returns the relative portion.
/// If the path is relative, canonicalizes relative to root.
///
/// # Example
/// ```ignore
/// let rel = normalize_relative_to("/project/src/main.rs", "/project")?;
/// assert_eq!(rel, PathBuf::from("src/main.rs"));
/// ```
pub fn normalize_relative_to(path: impl AsRef<Path>, root: impl AsRef<Path>) -> Result<PathBuf> {
    let path = path.as_ref();
    let root = root.as_ref();

    // Canonicalize root
    let canonical_root = root
        .canonicalize()
        .with_context(|| format!("Cannot resolve root: {}", root.display()))?;

    // Handle absolute paths
    if path.is_absolute() {
        if let Ok(canonical_path) = path.canonicalize() {
            if let Ok(relative) = canonical_path.strip_prefix(&canonical_root) {
                return Ok(relative.to_path_buf());
            }
        }
        // Path not within root, return as-is
        return Ok(path.to_path_buf());
    }

    // Handle relative paths - resolve relative to root
    let full_path = canonical_root.join(path);
    if let Ok(canonical) = full_path.canonicalize() {
        if let Ok(relative) = canonical.strip_prefix(&canonical_root) {
            return Ok(relative.to_path_buf());
        }
    }

    // Fallback: return original path
    Ok(path.to_path_buf())
}

/// Generate a stable project ID from a project root path using BLAKE3.
///
/// This ensures consistent project identification across versions and platforms.
pub fn compute_project_id(project_root: impl AsRef<Path>) -> String {
    let path = project_root.as_ref();
    let canon = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let mut hasher = blake3::Hasher::new();
    hasher.update(canon.to_string_lossy().as_bytes());
    hasher.finalize().to_hex().to_string()
}

/// Get the standardized LMDB manifest path for a given project ID.
///
/// Standard path: ~/.vrift/db/<project_id>.lmdb (using first 16 chars of ID)
pub fn get_manifest_db_path(project_id: &str) -> Option<PathBuf> {
    dirs::home_dir().map(|h| {
        h.join(".vrift")
            .join("db")
            .join(format!("{}.lmdb", &project_id[..16]))
    })
}

/// Get the standardized VDir mmap path for a given project ID.
///
/// Standard path: ~/.vrift/vdir/<project_id>.mmap (using first 16 chars of ID)
pub fn get_vdir_mmap_path(project_id: &str) -> Option<PathBuf> {
    dirs::home_dir().map(|h| {
        h.join(".vrift")
            .join("vdir")
            .join(format!("{}.mmap", &project_id[..16]))
    })
}

/// Get the standardized vDird socket path for a given project ID.
///
/// Standard path: ~/.vrift/sockets/<project_id>.sock (using first 16 chars of ID)
pub fn get_vdird_socket_path(project_id: &str) -> Option<PathBuf> {
    dirs::home_dir().map(|h| {
        h.join(".vrift")
            .join("sockets")
            .join(format!("{}.sock", &project_id[..16]))
    })
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

    #[test]
    fn test_is_within_directory() {
        let temp = tempdir().unwrap();
        let subdir = temp.path().join("subdir");
        fs::create_dir(&subdir).unwrap();
        let file = subdir.join("file.txt");
        fs::write(&file, "test").unwrap();

        // File is within temp dir
        assert!(is_within_directory(&file, temp.path()));
        // File is within subdir
        assert!(is_within_directory(&file, &subdir));
        // Subdir is within temp
        assert!(is_within_directory(&subdir, temp.path()));
        // Temp is NOT within subdir
        assert!(!is_within_directory(temp.path(), &subdir));
        // Nonexistent path returns false
        assert!(!is_within_directory("/nonexistent/path", temp.path()));
    }

    #[test]
    fn test_compute_relative_path() {
        let base = Path::new("/home/user/project");
        let target = Path::new("/home/user/project/src/main.rs");

        let relative = compute_relative_path(base, target);
        assert_eq!(relative, PathBuf::from("src/main.rs"));

        // No common prefix returns absolute target
        let other = Path::new("/tmp/file.txt");
        let result = compute_relative_path(base, other);
        assert_eq!(result, other);
    }

    #[test]
    fn test_strip_prefix_safe() {
        let path = Path::new("/vfs/project/src/main.rs");
        let prefix = Path::new("/vfs/project");

        let result = strip_prefix_safe(path, prefix);
        assert_eq!(result, Some(PathBuf::from("src/main.rs")));

        // No match returns None
        let other_prefix = Path::new("/other");
        assert_eq!(strip_prefix_safe(path, other_prefix), None);
    }

    #[test]
    fn test_ensure_within_valid() {
        let temp = tempdir().unwrap();
        let subdir = temp.path().join("subdir");
        fs::create_dir(&subdir).unwrap();
        let file = subdir.join("file.txt");
        fs::write(&file, "test").unwrap();

        // Valid: file is within temp
        let result = ensure_within(&file, temp.path());
        assert!(result.is_ok());
        assert!(result.unwrap().is_absolute());
    }

    #[test]
    fn test_ensure_within_invalid() {
        let temp1 = tempdir().unwrap();
        let temp2 = tempdir().unwrap();
        let file = temp1.path().join("file.txt");
        fs::write(&file, "test").unwrap();

        // Invalid: file in temp1 is not within temp2
        let result = ensure_within(&file, temp2.path());
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("outside"));
    }

    #[test]
    fn test_normalize_relative_to_absolute() {
        let temp = tempdir().unwrap();
        let file = temp.path().join("src/main.rs");
        fs::create_dir_all(temp.path().join("src")).unwrap();
        fs::write(&file, "test").unwrap();

        let result = normalize_relative_to(&file, temp.path()).unwrap();
        assert_eq!(result, PathBuf::from("src/main.rs"));
    }

    #[test]
    fn test_normalize_relative_to_relative() {
        let temp = tempdir().unwrap();
        fs::create_dir_all(temp.path().join("src")).unwrap();
        fs::write(temp.path().join("src/main.rs"), "test").unwrap();

        let result = normalize_relative_to("src/main.rs", temp.path()).unwrap();
        assert_eq!(result, PathBuf::from("src/main.rs"));
    }
}
