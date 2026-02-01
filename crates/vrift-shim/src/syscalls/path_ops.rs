use libc::{c_char, c_int};
use std::ffi::CString;
use std::path::Path;
use std::ptr;

// ============================================================================
// Core Logic
// ============================================================================

pub(crate) unsafe fn break_link(path_str: &str) -> Result<(), c_int> {
    let p = Path::new(path_str);
    let metadata = match std::fs::metadata(p) {
        Ok(m) => m,
        Err(_) => return Ok(()),
    };
    if std::os::unix::fs::MetadataExt::nlink(&metadata) < 2 {
        return Ok(());
    }

    #[cfg(target_os = "macos")]
    {
        let mut path_buf = [0u8; 1024];
        if path_str.len() >= 1024 {
            return Err(libc::ENAMETOOLONG);
        }
        ptr::copy_nonoverlapping(path_str.as_ptr(), path_buf.as_mut_ptr(), path_str.len());
        path_buf[path_str.len()] = 0;
        libc::chflags(path_buf.as_ptr() as *const c_char, 0);
    }

    #[cfg(target_os = "linux")]
    {
        if let Ok(res) = break_link_linux(path_str) {
            return Ok(res);
        }
    }

    // Fallback for macOS and Linux non-O_TMPFILE
    break_link_fallback(path_str)
}

#[cfg(target_os = "linux")]
unsafe fn break_link_linux(path_str: &str) -> Result<(), c_int> {
    use std::os::unix::ffi::OsStrExt;
    let path = Path::new(path_str);
    let parent = path.parent().unwrap_or_else(|| Path::new("."));

    let parent_path = CString::new(parent.as_os_str().as_bytes()).map_err(|_| libc::EINVAL)?;
    let dir_fd = libc::open(parent_path.as_ptr(), libc::O_DIRECTORY | libc::O_RDONLY);
    if dir_fd < 0 {
        return Err(libc::EACCES);
    }

    // O_TMPFILE = 0o20000000 | 0o0400000 on many Linux systems
    // But it's safer to use the constant if available. In libc it might be under __USE_GNU
    const O_TMPFILE: c_int = 0o20200000;

    // Use CString literal for "." safely
    let dot = CString::new(".").unwrap();
    let tmp_fd = libc::openat(dir_fd, dot.as_ptr(), O_TMPFILE | libc::O_RDWR, 0o600);
    if tmp_fd < 0 {
        libc::close(dir_fd);
        return Err(libc::ENOTSUP);
    }

    let src_fd = libc::open(
        CString::new(path_str).map_err(|_| libc::EINVAL)?.as_ptr(),
        libc::O_RDONLY,
    );
    if src_fd < 0 {
        libc::close(tmp_fd);
        libc::close(dir_fd);
        return Err(libc::EACCES);
    }

    // Try FICLONE (0x40049409)
    if libc::ioctl(tmp_fd, 0x40049409, src_fd) != 0 {
        // Fallback to copy_file_range
        let mut offset_in: libc::off_t = 0;
        let mut offset_out: libc::off_t = 0;
        // Use std::fs::metadata again or reuse?
        let len = std::fs::metadata(path_str)
            .map(|m| std::os::unix::fs::MetadataExt::len(&m))
            .unwrap_or(0);
        libc::copy_file_range(
            src_fd,
            &mut offset_in,
            tmp_fd,
            &mut offset_out,
            len as size_t,
            0,
        );
    }

    let proc_fd = format!("/proc/self/fd/{}", tmp_fd);
    let proc_fd_c = CString::new(proc_fd).map_err(|_| libc::EINVAL)?;
    let dest_c = CString::new(path_str).map_err(|_| libc::EINVAL)?;

    // AT_SYMLINK_FOLLOW = 0x400 in linkat
    if libc::linkat(
        libc::AT_FDCWD,
        proc_fd_c.as_ptr(),
        libc::AT_FDCWD,
        dest_c.as_ptr(),
        0x400,
    ) != 0
    {
        // If linkat fails (e.g. file exists), we might need to unlink first
        libc::unlink(dest_c.as_ptr());
        libc::linkat(
            libc::AT_FDCWD,
            proc_fd_c.as_ptr(),
            libc::AT_FDCWD,
            dest_c.as_ptr(),
            0x400,
        );
    }

    libc::close(src_fd);
    libc::close(tmp_fd);
    libc::close(dir_fd);
    Ok(())
}

unsafe fn break_link_fallback(path_str: &str) -> Result<(), c_int> {
    let mut tmp_path_buf = [0u8; 1024];
    let pb = path_str.as_bytes();
    if pb.len() > 1000 {
        return Err(libc::ENAMETOOLONG);
    }
    ptr::copy_nonoverlapping(pb.as_ptr(), tmp_path_buf.as_mut_ptr(), pb.len());
    let suffix = b".vrift_tmp";
    ptr::copy_nonoverlapping(
        suffix.as_ptr(),
        tmp_path_buf.as_mut_ptr().add(pb.len()),
        suffix.len(),
    );
    let tmp_len = pb.len() + suffix.len();
    tmp_path_buf[tmp_len] = 0;

    let tmp_ptr = tmp_path_buf.as_ptr() as *const c_char;
    let path_ptr = CString::new(path_str).map_err(|_| libc::EINVAL)?;
    // Ensure we can rename it even if it's currently read-only (e.g. from Step 1)
    let _ = libc::chmod(path_ptr.as_ptr(), 0o644);

    if libc::rename(path_ptr.as_ptr(), tmp_ptr) != 0 {
        return Err(libc::EACCES);
    }
    let tmp_path_str = std::str::from_utf8_unchecked(&tmp_path_buf[..tmp_len]);
    if std::fs::copy(tmp_path_str, path_str).is_err() {
        let _ = libc::rename(tmp_ptr, path_ptr.as_ptr());
        return Err(libc::EIO);
    }
    let _ = libc::unlink(tmp_ptr);
    #[cfg(target_os = "linux")]
    let _ = std::fs::set_permissions(
        path_str,
        std::os::unix::fs::PermissionsExt::from_mode(0o644),
    );
    Ok(())
}
