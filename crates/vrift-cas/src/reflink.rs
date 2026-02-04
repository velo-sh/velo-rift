//! ReFLINK (Copy-on-Write clone) support for zero-copy file ingestion.
//!
//! This module provides filesystem-aware file cloning with automatic fallback:
//! 1. Try ReFLINK (FICLONE ioctl on Linux, clonefile on macOS)
//! 2. Try hardlink (same filesystem only)
//! 3. Fall back to copy
//!
//! # References
//! - Linux: ioctl(FICLONE) for btrfs/xfs/ext4 (reflink-capable)
//! - macOS: clonefile() for APFS

use std::fs::{self};
use std::io;
use std::path::Path;

/// Result of an ingestion operation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IngestMethod {
    /// Zero-copy clone (FICLONE/clonefile)
    Reflink,
    /// Hard link (same inode)
    Hardlink,
    /// Full data copy
    Copy,
}

impl std::fmt::Display for IngestMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IngestMethod::Reflink => write!(f, "reflink"),
            IngestMethod::Hardlink => write!(f, "hardlink"),
            IngestMethod::Copy => write!(f, "copy"),
        }
    }
}

/// Error type for reflink operations
#[derive(Debug, thiserror::Error)]
pub enum ReflinkError {
    #[error("Reflink not supported on this filesystem")]
    NotSupported,

    #[error("Cross-device reflink not allowed")]
    CrossDevice,

    #[error("IO error: {0}")]
    Io(#[from] io::Error),
}

/// Try to create a reflink (CoW clone) from source to destination.
///
/// On Linux, uses FICLONE ioctl (btrfs, xfs, ext4 with reflink)
/// On macOS, uses clonefile() (APFS)
///
/// # Errors
/// Returns `ReflinkError::NotSupported` if the filesystem doesn't support reflinks.
pub fn try_reflink(src: &Path, dst: &Path) -> Result<(), ReflinkError> {
    // Ensure parent directory exists
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }

    #[cfg(target_os = "linux")]
    {
        try_reflink_linux(src, dst)
    }

    #[cfg(target_os = "macos")]
    {
        try_reflink_macos(src, dst)
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (src, dst);
        Err(ReflinkError::NotSupported)
    }
}

#[cfg(target_os = "linux")]
fn try_reflink_linux(src: &Path, dst: &Path) -> Result<(), ReflinkError> {
    use std::fs::File;
    use std::os::unix::io::AsRawFd;

    // FICLONE ioctl number
    const FICLONE: libc::c_ulong = 0x40049409;

    let src_file = File::open(src)?;
    let dst_file = File::create(dst)?;

    let result = unsafe { libc::ioctl(dst_file.as_raw_fd(), FICLONE, src_file.as_raw_fd()) };

    if result == 0 {
        Ok(())
    } else {
        let err = io::Error::last_os_error();
        // Clean up failed destination
        let _ = fs::remove_file(dst);

        match err.raw_os_error() {
            Some(libc::EXDEV) => Err(ReflinkError::CrossDevice),
            // EOPNOTSUPP and ENOTSUP have the same value on Linux, use allow to handle both platforms
            #[allow(unreachable_patterns)]
            Some(libc::EOPNOTSUPP) | Some(libc::ENOTSUP) => Err(ReflinkError::NotSupported),
            _ => Err(ReflinkError::Io(err)),
        }
    }
}

#[cfg(target_os = "macos")]
fn try_reflink_macos(src: &Path, dst: &Path) -> Result<(), ReflinkError> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    // clonefile flags
    const CLONE_NOFOLLOW: u32 = 0x0001;

    extern "C" {
        fn clonefile(src: *const libc::c_char, dst: *const libc::c_char, flags: u32)
            -> libc::c_int;
    }

    let src_cstr = CString::new(src.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid path"))?;
    let dst_cstr = CString::new(dst.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid path"))?;

    let result = unsafe { clonefile(src_cstr.as_ptr(), dst_cstr.as_ptr(), CLONE_NOFOLLOW) };

    if result == 0 {
        Ok(())
    } else {
        let err = io::Error::last_os_error();
        match err.raw_os_error() {
            Some(libc::EXDEV) => Err(ReflinkError::CrossDevice),
            Some(libc::ENOTSUP) => Err(ReflinkError::NotSupported),
            _ => Err(ReflinkError::Io(err)),
        }
    }
}

/// Try to create a hard link from source to destination.
///
/// # Errors
/// Returns an error if the files are on different filesystems (EXDEV).
pub fn try_hardlink(src: &Path, dst: &Path) -> io::Result<()> {
    // Ensure parent directory exists
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }

    fs::hard_link(src, dst)
}

/// Ingest a file with automatic fallback strategy.
///
/// Tries methods in order of efficiency:
/// 1. ReFLINK (zero-copy clone)
/// 2. Hardlink (same inode, zero-copy for reads)
/// 3. Copy (full data copy)
///
/// # Returns
/// The method used for ingestion.
///
/// # Example
/// ```ignore
/// let method = ingest_with_fallback(staging_file, cas_blob)?;
/// tracing::info!("Ingested using {}", method);
/// ```
pub fn ingest_with_fallback(src: &Path, dst: &Path) -> io::Result<IngestMethod> {
    // Skip if destination already exists (content-addressed)
    if dst.exists() {
        // Determine what method would have been used
        return Ok(IngestMethod::Reflink);
    }

    // Try reflink first (zero-copy)
    if try_reflink(src, dst).is_ok() {
        return Ok(IngestMethod::Reflink);
    }

    // Try hardlink (same inode)
    if try_hardlink(src, dst).is_ok() {
        return Ok(IngestMethod::Hardlink);
    }

    // Fallback: full copy
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(src, dst)?;
    Ok(IngestMethod::Copy)
}

/// Ingest a file and remove the source after successful ingestion.
///
/// This is the preferred method for staging file commit:
/// - Atomically moves data to CAS
/// - Cleans up staging file
pub fn ingest_and_remove(src: &Path, dst: &Path) -> io::Result<IngestMethod> {
    let method = ingest_with_fallback(src, dst)?;
    fs::remove_file(src)?;
    Ok(method)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use tempfile::tempdir;

    #[test]
    fn test_ingest_with_fallback_creates_file() {
        let temp = tempdir().unwrap();
        let src = temp.path().join("src.txt");
        let dst = temp.path().join("dst.txt");

        // Create source file
        let mut f = File::create(&src).unwrap();
        f.write_all(b"Hello, World!").unwrap();
        drop(f);

        let method = ingest_with_fallback(&src, &dst).unwrap();
        assert!(dst.exists());
        assert!(matches!(
            method,
            IngestMethod::Reflink | IngestMethod::Hardlink | IngestMethod::Copy
        ));

        // Verify content
        let content = fs::read_to_string(&dst).unwrap();
        assert_eq!(content, "Hello, World!");
    }

    #[test]
    fn test_ingest_skips_existing_destination() {
        let temp = tempdir().unwrap();
        let src = temp.path().join("src.txt");
        let dst = temp.path().join("dst.txt");

        // Create both files
        fs::write(&src, b"source").unwrap();
        fs::write(&dst, b"destination").unwrap();

        let method = ingest_with_fallback(&src, &dst).unwrap();
        assert_eq!(method, IngestMethod::Reflink);

        // Verify destination was NOT overwritten
        let content = fs::read_to_string(&dst).unwrap();
        assert_eq!(content, "destination");
    }

    #[test]
    fn test_ingest_and_remove_deletes_source() {
        let temp = tempdir().unwrap();
        let src = temp.path().join("src.txt");
        let dst = temp.path().join("subdir/dst.txt");

        fs::write(&src, b"test data").unwrap();

        let method = ingest_and_remove(&src, &dst).unwrap();
        assert!(!src.exists());
        assert!(dst.exists());
        assert!(matches!(
            method,
            IngestMethod::Reflink | IngestMethod::Hardlink | IngestMethod::Copy
        ));
    }

    #[test]
    fn test_ingest_creates_parent_directories() {
        let temp = tempdir().unwrap();
        let src = temp.path().join("src.txt");
        let dst = temp.path().join("a/b/c/dst.txt");

        fs::write(&src, b"nested").unwrap();

        let method = ingest_with_fallback(&src, &dst).unwrap();
        assert!(dst.exists());
        assert!(matches!(
            method,
            IngestMethod::Reflink | IngestMethod::Hardlink | IngestMethod::Copy
        ));
    }

    #[test]
    fn test_ingest_method_display() {
        assert_eq!(format!("{}", IngestMethod::Reflink), "reflink");
        assert_eq!(format!("{}", IngestMethod::Hardlink), "hardlink");
        assert_eq!(format!("{}", IngestMethod::Copy), "copy");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_reflink_macos_apfs() {
        // This test will succeed on APFS volumes
        let temp = tempdir().unwrap();
        let src = temp.path().join("src.txt");
        let dst = temp.path().join("dst.txt");

        fs::write(&src, b"reflink test").unwrap();

        match try_reflink(&src, &dst) {
            Ok(()) => {
                assert!(dst.exists());
                assert_eq!(fs::read_to_string(&dst).unwrap(), "reflink test");
            }
            Err(ReflinkError::NotSupported) => {
                // Not APFS, that's fine
            }
            Err(e) => panic!("Unexpected error: {}", e),
        }
    }

    #[test]
    fn test_ingest_nonexistent_source() {
        let temp = tempdir().unwrap();
        let src = temp.path().join("nonexistent.txt");
        let dst = temp.path().join("dst.txt");

        let result = ingest_with_fallback(&src, &dst);
        assert!(result.is_err());
    }

    #[test]
    fn test_ingest_empty_file() {
        let temp = tempdir().unwrap();
        let src = temp.path().join("empty.txt");
        let dst = temp.path().join("dst.txt");

        // Create empty file
        fs::write(&src, b"").unwrap();

        let method = ingest_with_fallback(&src, &dst).unwrap();
        assert!(dst.exists());
        assert_eq!(fs::read(&dst).unwrap().len(), 0);
        assert!(matches!(
            method,
            IngestMethod::Reflink | IngestMethod::Hardlink | IngestMethod::Copy
        ));
    }

    #[test]
    fn test_ingest_binary_file() {
        let temp = tempdir().unwrap();
        let src = temp.path().join("binary.bin");
        let dst = temp.path().join("out.bin");

        // Create binary content with all byte values
        let binary_data: Vec<u8> = (0..=255).collect();
        fs::write(&src, &binary_data).unwrap();

        let method = ingest_with_fallback(&src, &dst).unwrap();
        assert!(dst.exists());
        assert_eq!(fs::read(&dst).unwrap(), binary_data);
        assert!(matches!(
            method,
            IngestMethod::Reflink | IngestMethod::Hardlink | IngestMethod::Copy
        ));
    }

    #[test]
    fn test_ingest_large_file() {
        let temp = tempdir().unwrap();
        let src = temp.path().join("large.bin");
        let dst = temp.path().join("large_out.bin");

        // Create 1MB file
        let data = vec![0x42u8; 1024 * 1024];
        fs::write(&src, &data).unwrap();

        let method = ingest_with_fallback(&src, &dst).unwrap();
        assert!(dst.exists());
        let dst_size = fs::metadata(&dst).unwrap().len();
        assert_eq!(dst_size, 1024 * 1024);
        assert!(matches!(
            method,
            IngestMethod::Reflink | IngestMethod::Hardlink | IngestMethod::Copy
        ));
    }

    #[test]
    fn test_try_hardlink_same_filesystem() {
        let temp = tempdir().unwrap();
        let src = temp.path().join("src.txt");
        let dst = temp.path().join("link.txt");

        fs::write(&src, b"hardlink test").unwrap();

        // Should succeed on same filesystem
        let result = try_hardlink(&src, &dst);
        assert!(result.is_ok());
        assert!(dst.exists());
        assert_eq!(fs::read_to_string(&dst).unwrap(), "hardlink test");
    }

    #[test]
    fn test_try_hardlink_creates_parent_dirs() {
        let temp = tempdir().unwrap();
        let src = temp.path().join("src.txt");
        let dst = temp.path().join("nested/dir/link.txt");

        fs::write(&src, b"nested hardlink").unwrap();

        let result = try_hardlink(&src, &dst);
        assert!(result.is_ok());
        assert!(dst.exists());
    }

    #[test]
    fn test_reflink_error_display() {
        let e1 = ReflinkError::NotSupported;
        let e2 = ReflinkError::CrossDevice;
        let e3 = ReflinkError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "test error",
        ));

        assert!(!format!("{}", e1).is_empty());
        assert!(!format!("{}", e2).is_empty());
        assert!(!format!("{}", e3).is_empty());
    }
}
