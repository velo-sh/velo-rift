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

// Global list of real symbols used by shims
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
