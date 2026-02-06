//! Real Symbol Storage
//!
//! Pattern 2682: Provides access to real libc functions to avoid recursion deadlocks.
//!
//! On macOS: Uses dlsym(RTLD_NEXT) to get the real libc functions.
//! On Linux: Returns pointers to raw assembly syscall wrappers (Pattern 2692)
//!           to completely bypass libc and avoid any recursion.

use libc::{c_char, c_void};
use std::sync::atomic::{AtomicPtr, Ordering};

/// Storage for real libc functions to avoid recursion deadlocks
pub struct RealSymbol {
    ptr: AtomicPtr<c_void>,
    name: &'static str,
}

impl RealSymbol {
    pub const fn new(name: &'static str) -> Self {
        Self {
            ptr: AtomicPtr::new(std::ptr::null_mut()),
            name,
        }
    }

    /// Get the real function pointer.
    /// On macOS: Uses dlsym(RTLD_NEXT)
    /// On Linux: This is deprecated - use linux_raw module directly
    #[cfg(target_os = "macos")]
    pub unsafe fn get(&self) -> *mut c_void {
        let p = self.ptr.load(Ordering::Acquire);
        if !p.is_null() {
            return p;
        }
        let f = libc::dlsym(libc::RTLD_NEXT, self.name.as_ptr() as *const c_char);
        self.ptr.store(f, Ordering::Release);
        f
    }

    /// On Linux, we still support dlsym for compatibility,
    /// but prefer using linux_raw module for recursion-safe syscalls.
    #[cfg(target_os = "linux")]
    pub unsafe fn get(&self) -> *mut c_void {
        let p = self.ptr.load(Ordering::Acquire);
        if !p.is_null() {
            return p;
        }
        let f = libc::dlsym(libc::RTLD_NEXT, self.name.as_ptr() as *const c_char);
        self.ptr.store(f, Ordering::Release);
        f
    }
}

// Global list of real symbols used by shims (primarily macOS)
pub static REAL_OPEN: RealSymbol = RealSymbol::new("open\0");
pub static REAL_OPENAT: RealSymbol = RealSymbol::new("openat\0");
pub static REAL_CLOSE: RealSymbol = RealSymbol::new("close\0");
pub static REAL_WRITE: RealSymbol = RealSymbol::new("write\0");
pub static REAL_READ: RealSymbol = RealSymbol::new("read\0");
pub static REAL_STAT: RealSymbol = RealSymbol::new("stat\0");
pub static REAL_LSTAT: RealSymbol = RealSymbol::new("lstat\0");
pub static REAL_FSTAT: RealSymbol = RealSymbol::new("fstat\0");
pub static REAL_FSTATAT: RealSymbol = RealSymbol::new("fstatat\0");
pub static REAL_ACCESS: RealSymbol = RealSymbol::new("access\0");
pub static REAL_READLINK: RealSymbol = RealSymbol::new("readlink\0");
pub static REAL_REALPATH: RealSymbol = RealSymbol::new("realpath\0");
pub static REAL_DUP: RealSymbol = RealSymbol::new("dup\0");
pub static REAL_DUP2: RealSymbol = RealSymbol::new("dup2\0");
pub static REAL_FCHDIR: RealSymbol = RealSymbol::new("fchdir\0");
pub static REAL_LSEEK: RealSymbol = RealSymbol::new("lseek\0");
pub static REAL_FTRUNCATE: RealSymbol = RealSymbol::new("ftruncate\0");
pub static REAL_UNLINK: RealSymbol = RealSymbol::new("unlink\0");
pub static REAL_RMDIR: RealSymbol = RealSymbol::new("rmdir\0");
pub static REAL_RENAME: RealSymbol = RealSymbol::new("rename\0");
pub static REAL_MKDIR: RealSymbol = RealSymbol::new("mkdir\0");
pub static REAL_CHMOD: RealSymbol = RealSymbol::new("chmod\0");
pub static REAL_TRUNCATE: RealSymbol = RealSymbol::new("truncate\0");
pub static REAL_MMAP: RealSymbol = RealSymbol::new("mmap\0");
pub static REAL_MUNMAP: RealSymbol = RealSymbol::new("munmap\0");
pub static REAL_RENAMEAT: RealSymbol = RealSymbol::new("renameat\0");
pub static REAL_FCHMODAT: RealSymbol = RealSymbol::new("fchmodat\0");
pub static REAL_CHFLAGS: RealSymbol = RealSymbol::new("chflags\0");
pub static REAL_LINKAT: RealSymbol = RealSymbol::new("linkat\0");
pub static REAL_SETXATTR: RealSymbol = RealSymbol::new("setxattr\0");
pub static REAL_REMOVEXATTR: RealSymbol = RealSymbol::new("removexattr\0");
pub static REAL_UTIMES: RealSymbol = RealSymbol::new("utimes\0");
pub static REAL_UTIMENSAT: RealSymbol = RealSymbol::new("utimensat\0");
pub static REAL_OPENDIR: RealSymbol = RealSymbol::new("opendir\0");
pub static REAL_READDIR: RealSymbol = RealSymbol::new("readdir\0");
pub static REAL_CLOSEDIR: RealSymbol = RealSymbol::new("closedir\0");
pub static REAL_GETCWD: RealSymbol = RealSymbol::new("getcwd\0");
pub static REAL_CHDIR: RealSymbol = RealSymbol::new("chdir\0");
pub static REAL_LINK: RealSymbol = RealSymbol::new("link\0");
pub static REAL_UNLINKAT: RealSymbol = RealSymbol::new("unlinkat\0");
pub static REAL_MKDIRAT: RealSymbol = RealSymbol::new("mkdirat\0");
pub static REAL_SYMLINKAT: RealSymbol = RealSymbol::new("symlinkat\0");
pub static REAL_FCHMOD: RealSymbol = RealSymbol::new("fchmod\0");
pub static REAL_SETRLIMIT: RealSymbol = RealSymbol::new("setrlimit\0");
