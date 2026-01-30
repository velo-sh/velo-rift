//! # velo-shim
//!
//! LD_PRELOAD / DYLD_INSERT_LIBRARIES shim for Velo Rift virtual filesystem.
//!
//! This shared library intercepts filesystem syscalls (`open`, `stat`, `read`, etc.)
//! and redirects them through the Velo manifest and CAS.
//!
//! ## Usage (Linux)
//!
//! ```bash
//! VRIFT_MANIFEST=/path/to/manifest.bin \
//! VR_THE_SOURCE=/var/vrift/the_source \
//! LD_PRELOAD=/path/to/libvelo_shim.so \
//! python -c "import numpy"
//! ```
//!
//! ## Usage (macOS)
//!
//! ```bash
//! VRIFT_MANIFEST=/path/to/manifest.bin \
//! VR_THE_SOURCE=/var/vrift/the_source \
//! DYLD_INSERT_LIBRARIES=/path/to/libvelo_shim.dylib \
//! python -c "import numpy"
//! ```
//!
//! ## Environment Variables
//!
//! - `VRIFT_MANIFEST`: Path to the manifest file (required)
//! - `VR_THE_SOURCE`: Path to CAS root directory (default: `/var/vrift/the_source`)
//! - `VRIFT_VFS_PREFIX`: Virtual path prefix to intercept (default: `/vrift`)
//! - `VRIFT_DEBUG`: Enable debug logging if set

#![allow(clippy::missing_safety_doc)]
#![allow(unused_doc_comments)]

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::os::unix::io::RawFd;
#[allow(unused_imports)]
use std::path::PathBuf;

use std::ptr;
use std::sync::OnceLock;

use libc::{c_char, c_int, c_void, mode_t, size_t, ssize_t};
use memmap2::Mmap;
use velo_cas::CasStore;
use velo_manifest::Manifest;

// ============================================================================
// Platform-specific errno handling
// ============================================================================

#[cfg(target_os = "linux")]
unsafe fn set_errno(errno: c_int) {
    *libc::__errno_location() = errno;
}

#[cfg(target_os = "macos")]
unsafe fn set_errno(errno: c_int) {
    *libc::__error() = errno;
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
unsafe fn set_errno(_errno: c_int) {
    // Unsupported platform - no-op
}

// ============================================================================
// Global State
// ============================================================================

/// Global shim state, initialized on first syscall
static SHIM_STATE: OnceLock<ShimState> = OnceLock::new();

/// Thread-local file descriptor mapping
thread_local! {
    static FD_MAP: RefCell<HashMap<RawFd, VeloFd>> = RefCell::new(HashMap::new());
}

/// State for a Velo-managed file descriptor
struct VeloFd {
    /// Memory-mapped content (for reads)
    mmap: Mmap,
    /// Current read position
    position: usize,
    /// Virtual path (for debugging)
    #[allow(dead_code)]
    vpath: String,
    /// The actual underlying fd (for writes after break-before-write)
    real_fd: Option<RawFd>,
    /// Whether this fd was written to (needs re-ingest on close)
    modified: bool,
}

/// Global shim state
struct ShimState {
    /// The manifest for path lookups
    manifest: Manifest,
    /// CAS store for content retrieval
    cas: CasStore,
    /// Virtual path prefix (paths starting with this are intercepted)
    vfs_prefix: String,
}

use tracing::{debug, error};

impl ShimState {
    fn init() -> Option<Self> {
        let manifest_path = std::env::var("VRIFT_MANIFEST").ok()?;
        let cas_root =
            std::env::var("VR_THE_SOURCE").unwrap_or_else(|_| "/var/vrift/the_source".to_string());
        let vfs_prefix = std::env::var("VRIFT_VFS_PREFIX").unwrap_or_else(|_| "/vrift".to_string());

        // Initialize tracing if not already initialized
        // We use try_init because this might be called multiple times or conflict with app
        // Initialize tracing if not already initialized
        // We use try_init because this might be called multiple times or conflict with app
        // let _ = tracing_subscriber::fmt()
        //     .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        //     .with_writer(std::io::stderr)
        //     .try_init();

        debug!(manifest = %manifest_path, cas = %cas_root, prefix = %vfs_prefix, "Initializing Velo Shim");

        let manifest = Manifest::load(&manifest_path).ok()?;
        let cas = CasStore::new(&cas_root).ok()?;

        Some(Self {
            manifest,
            cas,
            vfs_prefix,
        })
    }

    fn get() -> Option<&'static Self> {
        SHIM_STATE.get_or_init(|| {
            Self::init().unwrap_or_else(|| {
                // Return a dummy state that doesn't intercept anything
                ShimState {
                    manifest: Manifest::new(),
                    cas: CasStore::new("/tmp/vrift-shim-dummy").unwrap(),
                    vfs_prefix: "/nonexistent-vrift-prefix".to_string(),
                }
            })
        });
        let state = SHIM_STATE.get()?;
        // Only return state if manifest is non-empty (properly initialized)
        if state.manifest.is_empty() && state.vfs_prefix == "/nonexistent-vrift-prefix" {
            None
        } else {
            Some(state)
        }
    }

    fn should_intercept(&self, path: &str) -> bool {
        path.starts_with(&self.vfs_prefix)
    }
}

// ============================================================================
// Original libc function pointers
// ============================================================================

type OpenFn = unsafe extern "C" fn(*const c_char, c_int, mode_t) -> c_int;
type ReadFn = unsafe extern "C" fn(c_int, *mut c_void, size_t) -> ssize_t;
type WriteFn = unsafe extern "C" fn(c_int, *const c_void, size_t) -> ssize_t;
type CloseFn = unsafe extern "C" fn(c_int) -> c_int;
type StatFn = unsafe extern "C" fn(*const c_char, *mut libc::stat) -> c_int;
type FstatFn = unsafe extern "C" fn(c_int, *mut libc::stat) -> c_int;
type LseekFn = unsafe extern "C" fn(c_int, libc::off_t, c_int) -> libc::off_t;
type ReadlinkFn = unsafe extern "C" fn(*const c_char, *mut c_char, size_t) -> ssize_t;

static REAL_OPEN: OnceLock<OpenFn> = OnceLock::new();
static REAL_READ: OnceLock<ReadFn> = OnceLock::new();
static REAL_WRITE: OnceLock<WriteFn> = OnceLock::new();
static REAL_CLOSE: OnceLock<CloseFn> = OnceLock::new();
static REAL_STAT: OnceLock<StatFn> = OnceLock::new();
static REAL_FSTAT: OnceLock<FstatFn> = OnceLock::new();
static REAL_LSEEK: OnceLock<LseekFn> = OnceLock::new();
static REAL_READLINK: OnceLock<ReadlinkFn> = OnceLock::new();

macro_rules! get_real_fn {
    ($static:ident, $name:literal, $type:ty) => {{
        $static.get_or_init(|| {
            let name = CString::new($name).unwrap();
            unsafe {
                let ptr = libc::dlsym(libc::RTLD_NEXT, name.as_ptr());
                if ptr.is_null() {
                    panic!("Failed to find {}", $name);
                }
                std::mem::transmute::<*mut c_void, $type>(ptr)
            }
        })
    }};
}

// ============================================================================
// Helper functions
// ============================================================================

fn path_from_cstr(path: *const c_char) -> Option<String> {
    if path.is_null() {
        return None;
    }
    unsafe { CStr::from_ptr(path).to_str().ok().map(String::from) }
}

/// Get the next available fake FD (negative to avoid conflicts)
#[allow(dead_code)]
fn allocate_velo_fd() -> RawFd {
    static NEXT_FD: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1000);
    NEXT_FD.fetch_sub(1, std::sync::atomic::Ordering::SeqCst)
}

fn is_velo_fd(fd: RawFd) -> bool {
    fd < -100 // Our fake FDs are very negative
}

// ============================================================================
// Intercepted syscalls
// ============================================================================

/// Intercept open() syscall
#[no_mangle]
pub unsafe extern "C" fn open(path: *const c_char, flags: c_int, mode: mode_t) -> c_int {
    let real_open = get_real_fn!(REAL_OPEN, "open", OpenFn);

    let Some(path_str) = path_from_cstr(path) else {
        return real_open(path, flags, mode);
    };

    let Some(state) = ShimState::get() else {
        return real_open(path, flags, mode);
    };

    if !state.should_intercept(&path_str) {
        return real_open(path, flags, mode);
    }

    let span = tracing::trace_span!("open", path = %path_str);
    let _enter = span.enter();

    debug!("Intercepting open call");

    // Look up in manifest
    let Some(entry) = state.manifest.get(&path_str) else {
        debug!("Path not found in manifest");
        set_errno(libc::ENOENT);
        return -1;
    };

    if entry.is_dir() {
        debug!("Path is a directory");
        set_errno(libc::EISDIR);
        return -1;
    }

    // Get content from CAS
    let content = match state.cas.get(&entry.content_hash) {
        Ok(data) => data,
        Err(e) => {
            error!(error = %e, "Failed to get content from CAS");
            set_errno(libc::EIO);
            return -1;
        }
    };

    // Create memory mapping
    #[cfg(target_os = "linux")]
    {
        // Linux: Use memfd_create for zero-copy (memory-only) file descriptor
        let name = CString::new(entry.content_hash.len().to_string()).unwrap();
        let memfd = unsafe { libc::memfd_create(name.as_ptr(), libc::MFD_CLOEXEC) };
        if memfd < 0 {
            error!("memfd_create failed");
            set_errno(libc::EIO);
            return -1;
        }

        // Write content to memfd
        // Note: write() is simple and fast for moderate sizes.
        // For huge files, splicing from a pipe or using a shared buffer would be even faster,
        // but this already avoids disk I/O.
        let written =
            unsafe { libc::write(memfd, content.as_ptr() as *const c_void, content.len()) };

        if written != content.len() as ssize_t {
            error!("Failed to write content to memfd");
            unsafe { libc::close(memfd) };
            set_errno(libc::EIO);
            return -1;
        }

        // Reset file offset to 0 so the user reads from start
        unsafe { libc::lseek(memfd, 0, libc::SEEK_SET) };

        // We return the REAL memfd. Because it's a real FD, we don't need to track it in FD_MAP.
        // The shim's read/close/lseek hooks will see it's not in FD_MAP and pass it to libc,
        // which works perfectly for memfd.
        debug!(fd = memfd, "Opened via memfd_create");
        memfd
    }

    #[cfg(not(target_os = "linux"))]
    {
        // Fallback (macOS): Use temp file + mmap
        let temp_path = format!("/tmp/vrift-shim-{}", std::process::id());
        let temp_file_path =
            PathBuf::from(&temp_path).join(CasStore::hash_to_hex(&entry.content_hash));

        if !temp_file_path.exists() {
            std::fs::create_dir_all(&temp_path).ok();
            std::fs::write(&temp_file_path, &content).ok();
        }

        let file = match std::fs::File::open(&temp_file_path) {
            Ok(f) => f,
            Err(_) => {
                set_errno(libc::EIO);
                return -1;
            }
        };

        let mmap = match unsafe { Mmap::map(&file) } {
            Ok(m) => m,
            Err(_) => {
                set_errno(libc::EIO);
                return -1;
            }
        };

        let velo_fd = allocate_velo_fd();

        FD_MAP.with(|map| {
            map.borrow_mut().insert(
                velo_fd,
                VeloFd {
                    mmap,
                    position: 0,
                    vpath: path_str.clone(),
                    real_fd: None,
                    modified: false,
                },
            );
        });

        debug!(fd = velo_fd, "Opened virtual file (mmap)");
        velo_fd
    }
}

/// Intercept write() syscall with Break-Before-Write (RFC-0039)
///
/// For Tier-2 assets, this implements the BBW protocol:
/// 1. Detect write to ingested file
/// 2. Break hardlink (copy content to new file)
/// 3. Allow write to proceed
/// 4. Re-ingest on close()
#[no_mangle]
pub unsafe extern "C" fn write(fd: c_int, buf: *const c_void, count: size_t) -> ssize_t {
    let real_write = get_real_fn!(REAL_WRITE, "write", WriteFn);

    // For Velo FDs that have been "broken" for writing
    if is_velo_fd(fd) {
        return FD_MAP.with(|map| {
            let mut map = map.borrow_mut();
            let Some(vfd) = map.get_mut(&fd) else {
                set_errno(libc::EBADF);
                return -1;
            };

            // If we have a real_fd (from BBW), write to it
            if let Some(real_fd) = vfd.real_fd {
                vfd.modified = true;
                return real_write(real_fd, buf, count);
            }

            // Otherwise, break-before-write: copy content to temp file
            debug!(path = %vfd.vpath, "Break-Before-Write triggered");

            // Create temporary file for writing
            let temp_path = format!("/tmp/vrift-bbw-{}-{}", std::process::id(), fd.abs());
            let temp_cstr = CString::new(temp_path.clone()).unwrap();
            let temp_fd = libc::open(
                temp_cstr.as_ptr(),
                libc::O_CREAT | libc::O_RDWR | libc::O_TRUNC,
                0o644,
            );

            if temp_fd < 0 {
                error!("Failed to create temp file for BBW");
                return -1;
            }

            // Copy existing content to temp file
            let written = libc::write(
                temp_fd,
                vfd.mmap.as_ptr() as *const c_void,
                vfd.mmap.len(),
            );

            if written != vfd.mmap.len() as ssize_t {
                error!("Failed to copy content for BBW");
                libc::close(temp_fd);
                return -1;
            }

            // Seek to the current position
            libc::lseek(temp_fd, vfd.position as libc::off_t, libc::SEEK_SET);

            // Store the real fd and mark as modified
            vfd.real_fd = Some(temp_fd);
            vfd.modified = true;

            // Now write to the real fd
            real_write(temp_fd, buf, count)
        });
    }

    // Not a Velo FD, pass through
    real_write(fd, buf, count)
}

/// Intercept read() syscall
#[no_mangle]
pub unsafe extern "C" fn read(fd: c_int, buf: *mut c_void, count: size_t) -> ssize_t {
    let real_read = get_real_fn!(REAL_READ, "read", ReadFn);

    if !is_velo_fd(fd) {
        return real_read(fd, buf, count);
    }

    FD_MAP.with(|map| {
        let mut map = map.borrow_mut();
        let Some(vfd) = map.get_mut(&fd) else {
            set_errno(libc::EBADF);
            return -1;
        };

        let remaining = vfd.mmap.len().saturating_sub(vfd.position);
        let to_read = count.min(remaining);

        if to_read == 0 {
            return 0; // EOF
        }

        ptr::copy_nonoverlapping(vfd.mmap.as_ptr().add(vfd.position), buf as *mut u8, to_read);

        vfd.position += to_read;
        to_read as ssize_t
    })
}

/// Intercept close() syscall with re-ingest support (RFC-0039)
#[no_mangle]
pub unsafe extern "C" fn close(fd: c_int) -> c_int {
    let real_close = get_real_fn!(REAL_CLOSE, "close", CloseFn);

    if !is_velo_fd(fd) {
        return real_close(fd);
    }

    FD_MAP.with(|map| {
        if let Some(vfd) = map.borrow_mut().remove(&fd) {
            // If we broke the link and wrote, we need to re-ingest
            if vfd.modified {
                if let Some(real_fd) = vfd.real_fd {
                    debug!(path = %vfd.vpath, "Closing modified file - re-ingest needed");
                    // Close the temp fd
                    real_close(real_fd);
                    // TODO: Trigger re-ingest via manifest update
                    // For now, we just log that re-ingest is needed
                    // In full implementation, this would:
                    // 1. Read the temp file
                    // 2. Calculate new BLAKE3 hash
                    // 3. Store in CAS
                    // 4. Update manifest with new hash
                }
            }
        }
    });

    0
}

/// Intercept lseek() syscall
#[no_mangle]
pub unsafe extern "C" fn lseek(fd: c_int, offset: libc::off_t, whence: c_int) -> libc::off_t {
    let real_lseek = get_real_fn!(REAL_LSEEK, "lseek", LseekFn);

    if !is_velo_fd(fd) {
        return real_lseek(fd, offset, whence);
    }

    FD_MAP.with(|map| {
        let mut map = map.borrow_mut();
        let Some(vfd) = map.get_mut(&fd) else {
            set_errno(libc::EBADF);
            return -1;
        };

        let new_pos = match whence {
            libc::SEEK_SET => offset as usize,
            libc::SEEK_CUR => (vfd.position as i64 + offset) as usize,
            libc::SEEK_END => (vfd.mmap.len() as i64 + offset) as usize,
            _ => {
                set_errno(libc::EINVAL);
                return -1;
            }
        };

        if new_pos > vfd.mmap.len() {
            set_errno(libc::EINVAL);
            return -1;
        }

        vfd.position = new_pos;
        new_pos as libc::off_t
    })
}

/// Intercept stat() syscall
#[no_mangle]
pub unsafe extern "C" fn stat(path: *const c_char, statbuf: *mut libc::stat) -> c_int {
    let real_stat = get_real_fn!(REAL_STAT, "stat", StatFn);

    let Some(path_str) = path_from_cstr(path) else {
        return real_stat(path, statbuf);
    };

    let Some(state) = ShimState::get() else {
        return real_stat(path, statbuf);
    };

    if !state.should_intercept(&path_str) {
        return real_stat(path, statbuf);
    }

    let span = tracing::trace_span!("stat", path = %path_str);
    let _enter = span.enter();

    debug!("Intercepting stat call");

    let Some(entry) = state.manifest.get(&path_str) else {
        set_errno(libc::ENOENT);
        return -1;
    };

    // Fill stat buffer
    let stat = &mut *statbuf;
    ptr::write_bytes(stat, 0, 1); // Zero-initialize

    stat.st_size = entry.size as libc::off_t;
    stat.st_mtime = entry.mtime as libc::time_t;

    // Handle mode - platform-specific type
    #[cfg(target_os = "macos")]
    {
        stat.st_mode = (entry.mode as i32) as u16;
    }
    #[cfg(target_os = "linux")]
    {
        stat.st_mode = entry.mode;
    }

    if entry.is_dir() {
        #[cfg(target_os = "macos")]
        {
            stat.st_mode |= (libc::S_IFDIR as u32) as u16;
        }
        #[cfg(target_os = "linux")]
        {
            stat.st_mode |= libc::S_IFDIR;
        }
        stat.st_nlink = 2;
    } else {
        #[cfg(target_os = "macos")]
        {
            stat.st_mode |= (libc::S_IFREG as u32) as u16;
        }
        #[cfg(target_os = "linux")]
        {
            stat.st_mode |= libc::S_IFREG;
        }
        stat.st_nlink = 1;
    }

    0
}

/// Intercept lstat() syscall
#[no_mangle]
pub unsafe extern "C" fn lstat(path: *const c_char, statbuf: *mut libc::stat) -> c_int {
    let real_lstat = get_real_fn!(REAL_STAT, "lstat", StatFn); // lstat signature same as stat

    let Some(path_str) = path_from_cstr(path) else {
        return real_lstat(path, statbuf);
    };

    let Some(state) = ShimState::get() else {
        return real_lstat(path, statbuf);
    };

    if !state.should_intercept(&path_str) {
        return real_lstat(path, statbuf);
    }

    let span = tracing::trace_span!("lstat", path = %path_str);
    let _enter = span.enter();

    debug!("Intercepting lstat call");

    let Some(entry) = state.manifest.get(&path_str) else {
        set_errno(libc::ENOENT);
        return -1;
    };

    // Fill stat buffer
    let stat = &mut *statbuf;
    ptr::write_bytes(stat, 0, 1); // Zero-initialize

    stat.st_size = entry.size as libc::off_t;
    stat.st_mtime = entry.mtime as libc::time_t;

    if entry.is_dir() {
        #[cfg(target_os = "macos")]
        {
            stat.st_mode = (libc::S_IFDIR as u32 | 0o755) as u16;
        }
        #[cfg(target_os = "linux")]
        {
            stat.st_mode = libc::S_IFDIR | 0o755;
        }
        stat.st_nlink = 2;
    } else if entry.is_symlink() {
        #[cfg(target_os = "macos")]
        {
            stat.st_mode = (libc::S_IFLNK as u32 | 0o777) as u16;
        }
        #[cfg(target_os = "linux")]
        {
            stat.st_mode = libc::S_IFLNK | 0o777;
        }
        stat.st_nlink = 1;
    } else {
        #[cfg(target_os = "macos")]
        {
            stat.st_mode = (libc::S_IFREG as u32 | entry.mode) as u16;
        }
        #[cfg(target_os = "linux")]
        {
            stat.st_mode = libc::S_IFREG | entry.mode;
        }
        stat.st_nlink = 1;
    }

    0
}

/// Intercept fstat() syscall
#[no_mangle]
pub unsafe extern "C" fn fstat(fd: c_int, statbuf: *mut libc::stat) -> c_int {
    let real_fstat = get_real_fn!(REAL_FSTAT, "fstat", FstatFn);

    if !is_velo_fd(fd) {
        return real_fstat(fd, statbuf);
    }

    FD_MAP.with(|map| {
        let map = map.borrow();
        let Some(vfd) = map.get(&fd) else {
            set_errno(libc::EBADF);
            return -1;
        };

        let stat = &mut *statbuf;
        ptr::write_bytes(stat, 0, 1);

        stat.st_size = vfd.mmap.len() as libc::off_t;
        stat.st_nlink = 1;

        #[cfg(target_os = "macos")]
        let mode = (libc::S_IFREG as u32 | 0o644) as u16;
        #[cfg(target_os = "linux")]
        let mode = libc::S_IFREG | 0o644;

        stat.st_mode = mode;

        0
    })
}

/// Intercept readlink() syscall
#[no_mangle]
pub unsafe extern "C" fn readlink(
    path: *const c_char,
    buf: *mut c_char,
    bufsize: size_t,
) -> ssize_t {
    let real_readlink = get_real_fn!(REAL_READLINK, "readlink", ReadlinkFn);

    let Some(path_str) = path_from_cstr(path) else {
        return real_readlink(path, buf, bufsize);
    };

    let Some(state) = ShimState::get() else {
        return real_readlink(path, buf, bufsize);
    };

    if !state.should_intercept(&path_str) {
        return real_readlink(path, buf, bufsize);
    }

    let span = tracing::trace_span!("readlink", path = %path_str);
    let _enter = span.enter();

    debug!("Intercepting readlink call");

    let Some(entry) = state.manifest.get(&path_str) else {
        set_errno(libc::ENOENT);
        return -1;
    };

    if !entry.is_symlink() {
        set_errno(libc::EINVAL); // Not a symlink
        return -1;
    }

    // Get content from CAS (target path)
    let content = match state.cas.get(&entry.content_hash) {
        Ok(data) => data,
        Err(e) => {
            error!(error = %e, "Failed to get symlink target from CAS");
            set_errno(libc::EIO);
            return -1;
        }
    };

    let len = content.len().min(bufsize);
    ptr::copy_nonoverlapping(content.as_ptr() as *const c_char, buf, len);

    // readlink does NOT null-terminate
    len as ssize_t
}

// ============================================================================
// Linux-specific syscall wrappers (__xstat family)
// ============================================================================

#[cfg(target_os = "linux")]
mod linux_compat {
    use super::*;

    /// Intercept __xstat() (glibc internal)
    #[no_mangle]
    pub unsafe extern "C" fn __xstat(
        _ver: c_int,
        path: *const c_char,
        statbuf: *mut libc::stat,
    ) -> c_int {
        stat(path, statbuf)
    }

    /// Intercept __lxstat() (glibc internal)
    #[no_mangle]
    pub unsafe extern "C" fn __lxstat(
        _ver: c_int,
        path: *const c_char,
        statbuf: *mut libc::stat,
    ) -> c_int {
        lstat(path, statbuf)
    }

    /// Intercept __fxstat() (glibc internal)
    #[no_mangle]
    pub unsafe extern "C" fn __fxstat(_ver: c_int, fd: c_int, statbuf: *mut libc::stat) -> c_int {
        fstat(fd, statbuf)
    }

    /// Intercept open64() (same as open on 64-bit)
    #[no_mangle]
    pub unsafe extern "C" fn open64(path: *const c_char, flags: c_int, mode: mode_t) -> c_int {
        open(path, flags, mode)
    }
}

// ============================================================================
// Module initialization (constructor)
// ============================================================================

/// Called when the library is loaded
#[used]
#[cfg(not(test))]
#[cfg_attr(target_os = "linux", link_section = ".init_array")]
#[cfg_attr(target_os = "macos", link_section = "__DATA,__mod_init_func")]
static INIT: extern "C" fn() = {
    extern "C" fn init() {
        // Pre-initialize state to avoid lazy init during syscalls
        let _ = ShimState::get();
    }
    init
};
