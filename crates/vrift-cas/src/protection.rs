//! Filesystem Protection Layer (RFC-0039 P1)
//!
//! Provides native system call support for advanced filesystem protection:
//! - Immutable flags (chattr +i / chflags uchg)
//! - Ownership and permissions management
//!
//! # Implementation
//!
//! - **macOS**: Uses `chflags(2)` with `UF_IMMUTABLE`
//! - **Linux**: Uses `ioctl(2)` with `FS_IOC_SETFLAGS` and `FS_IMMUTABLE_FL`

use std::io;
use std::path::Path;

/// RFC-0039 Security Invariant: TheSource (CAS) is a pure data warehouse.
/// Execution bits are strictly forbidden within the CAS storage layer.
/// This acts as a circuit breaker against direct execution of ingested payloads.
pub const CAS_READ_ONLY_PERM: u32 = (libc::S_IRUSR | libc::S_IRGRP | libc::S_IROTH) as u32; // 0444

/// The "Forbidden Mask" includes all Write and Execute bits for all users.
/// We use this to audit and strip permissions.
pub const CAS_FORBIDDEN_PERM_MASK: u32 = (libc::S_IWUSR
    | libc::S_IWGRP
    | libc::S_IWOTH // Write bits
    | libc::S_IXUSR
    | libc::S_IXGRP
    | libc::S_IXOTH) as u32; // Execute bits (The Iron Law)

/// Enforce the security invariant on a CAS blob.
/// Ensures the file is read-only and NOT executable.
pub fn enforce_cas_invariant(path: &Path) -> io::Result<()> {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    // Apply strict 0444 permissions
    fs::set_permissions(path, fs::Permissions::from_mode(CAS_READ_ONLY_PERM))?;
    Ok(())
}

/// Set or unset the immutable flag on a file.
///
/// On macOS, this sets the `UF_IMMUTABLE` flag (uchg).
/// On Linux, this sets the `FS_IMMUTABLE_FL` attribute via ioctl.
///
/// # Note
///
/// Setting the immutable flag on Linux typically requires `CAP_LINUX_IMMUTABLE`
/// or root privileges. On macOS, the owner can set `UF_IMMUTABLE`.
pub fn set_immutable(path: &Path, immutable: bool) -> io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;

        let c_path = CString::new(path.as_os_str().as_bytes())?;
        let flags: u32 = if immutable { libc::UF_IMMUTABLE } else { 0 };

        let ret = unsafe { libc::chflags(c_path.as_ptr(), flags) };
        if ret == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }

    #[cfg(target_os = "linux")]
    {
        use std::fs::File;
        use std::os::unix::io::AsRawFd;

        // On Linux, we use ioctl on the file descriptor.
        let file = File::open(path)?;
        let fd = file.as_raw_fd();

        // FS_IMMUTABLE_FL = 0x00000010
        // FS_IOC_GETFLAGS = 0x80086601
        // FS_IOC_SETFLAGS = 0x40086602
        const FS_IMMUTABLE_FL: libc::c_int = 0x00000010;
        const FS_IOC_GETFLAGS: libc::c_ulong = 0x80086601;
        const FS_IOC_SETFLAGS: libc::c_ulong = 0x40086602;

        let mut flags: libc::c_int = 0;
        let ret = unsafe { libc::ioctl(fd, FS_IOC_GETFLAGS, &mut flags) };
        if ret != 0 {
            return Err(io::Error::last_os_error());
        }

        if immutable {
            flags |= FS_IMMUTABLE_FL;
        } else {
            flags &= !FS_IMMUTABLE_FL;
        }

        let ret = unsafe { libc::ioctl(fd, FS_IOC_SETFLAGS, &flags) };
        if ret == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = path;
        let _ = immutable;
        Ok(()) // No-op on unsupported platforms
    }
}

/// Check if a file is immutable.
pub fn is_immutable(path: &Path) -> io::Result<bool> {
    #[cfg(target_os = "macos")]
    {
        use std::fs;
        // The compiler specifically suggested std::os::darwin::fs::MetadataExt
        #[cfg(target_os = "macos")]
        use std::os::darwin::fs::MetadataExt;

        let metadata = fs::metadata(path)?;
        let flags = metadata.st_flags();
        Ok((flags & libc::UF_IMMUTABLE) != 0)
    }

    #[cfg(target_os = "linux")]
    {
        use std::fs::File;
        use std::os::unix::io::AsRawFd;

        let file = File::open(path)?;
        let fd = file.as_raw_fd();
        const FS_IMMUTABLE_FL: libc::c_int = 0x00000010;
        const FS_IOC_GETFLAGS: libc::c_ulong = 0x80086601;

        let mut flags: libc::c_int = 0;
        let ret = unsafe { libc::ioctl(fd, FS_IOC_GETFLAGS, &mut flags) };
        if ret != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok((flags & FS_IMMUTABLE_FL) != 0)
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = path;
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use tempfile::tempdir;

    #[test]
    fn test_immutable_flag() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test_protection.txt");
        let mut file = File::create(&file_path).unwrap();
        file.write_all(b"test content").unwrap();
        drop(file);

        // Initially not immutable
        assert!(!is_immutable(&file_path).unwrap());

        // Set immutable
        // Note: This might fail if the user doesn't have permission (e.g. on Linux without CAP_LINUX_IMMUTABLE)
        // But on macOS, the owner can set UF_IMMUTABLE.
        match set_immutable(&file_path, true) {
            Ok(()) => {
                assert!(is_immutable(&file_path).unwrap());

                // Try to delete - should fail
                let res = std::fs::remove_file(&file_path);
                assert!(res.is_err(), "Should not be able to delete immutable file");

                // Unset immutable
                set_immutable(&file_path, false).unwrap();
                assert!(!is_immutable(&file_path).unwrap());

                // Should be deletable now
                std::fs::remove_file(&file_path).unwrap();
            }
            Err(e) => {
                // If it fails with EPERM, we skip the test but log it
                // EPERM is expected on Linux for non-root users
                #[cfg(target_os = "linux")]
                if e.kind() == io::ErrorKind::PermissionDenied {
                    println!("Skipping immutable test on Linux (requires root): {}", e);
                    return;
                }
                panic!("Failed to set immutable flag: {}", e);
            }
        }
    }
}
