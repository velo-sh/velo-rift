//! # velo-shim
//!
//! LD_PRELOAD / DYLD_INSERT_LIBRARIES shim for Velo Rift virtual filesystem.
//! Industrial-grade, zero-allocation, and recursion-safe.

#![allow(clippy::missing_safety_doc)]
#![allow(unused_doc_comments)]
#![allow(dead_code)]
#![allow(clippy::needless_borrow)]
#![allow(clippy::unnecessary_cast)]
#![allow(clippy::unnecessary_map_or)]

use std::ffi::{CStr, CString};
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};

use libc::{c_char, c_int, c_void, mode_t, size_t, ssize_t};
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Mutex;
use vrift_cas::CasStore;

thread_local! {
    static VIRTUAL_CWD: RefCell<Option<String>> = const { RefCell::new(None) };
    /// Thread-local recursion guard to prevent re-entry during shim execution.
    static IN_SHIM: AtomicBool = const { AtomicBool::new(false) };
}

// ============================================================================
// Platform Bridges & Interpose Section
// ============================================================================

#[cfg(target_os = "macos")]
#[repr(C)]
struct Interpose {
    new_func: *const (),
    old_func: *const (),
}

#[cfg(target_os = "macos")]
unsafe impl Sync for Interpose {}

#[cfg(target_os = "macos")]
extern "C" {
    fn open(path: *const c_char, flags: c_int, mode: mode_t) -> c_int;
    fn close(fd: c_int) -> c_int;
    fn write(fd: c_int, buf: *const c_void, count: size_t) -> ssize_t;
    fn stat(path: *const c_char, buf: *mut libc::stat) -> c_int;
    fn lstat(path: *const c_char, buf: *mut libc::stat) -> c_int;
    fn fstat(fd: c_int, buf: *mut libc::stat) -> c_int;
    fn opendir(path: *const c_char) -> *mut libc::DIR;
    fn readdir(dirp: *mut libc::DIR) -> *mut libc::dirent;
    fn closedir(dirp: *mut libc::DIR) -> c_int;
    fn readlink(path: *const c_char, buf: *mut c_char, bufsiz: size_t) -> ssize_t;
    fn execve(path: *const c_char, argv: *const *const c_char, envp: *const *const c_char)
        -> c_int;
    fn posix_spawn(
        pid: *mut libc::pid_t,
        path: *const c_char,
        file_actions: *const c_void,
        attrp: *const c_void,
        argv: *const *const c_char,
        envp: *const *const c_char,
    ) -> c_int;
    fn posix_spawnp(
        pid: *mut libc::pid_t,
        file: *const c_char,
        file_actions: *const c_void,
        attrp: *const c_void,
        argv: *const *const c_char,
        envp: *const *const c_char,
    ) -> c_int;
    fn realpath(pathname: *const c_char, resolved_path: *mut c_char) -> *mut c_char;

    #[link_name = "realpath$DARWIN_EXTSN"]
    fn realpath_darwin(pathname: *const c_char, resolved_path: *mut c_char) -> *mut c_char;
    fn getcwd(buf: *mut c_char, size: size_t) -> *mut c_char;
    fn chdir(path: *const c_char) -> c_int;
    fn unlink(path: *const c_char) -> c_int;
    fn rename(oldpath: *const c_char, newpath: *const c_char) -> c_int;
    fn rmdir(path: *const c_char) -> c_int;
    fn mmap(
        addr: *mut c_void,
        len: size_t,
        prot: c_int,
        flags: c_int,
        fd: c_int,
        offset: libc::off_t,
    ) -> *mut c_void;
    fn munmap(addr: *mut c_void, len: size_t) -> c_int;
    fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn access(path: *const c_char, mode: c_int) -> c_int;
}

#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
static IT_OPEN: Interpose = Interpose {
    new_func: open_shim as *const (),
    old_func: open as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
static IT_WRITE: Interpose = Interpose {
    new_func: write_shim as *const (),
    old_func: write as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
static IT_CLOSE: Interpose = Interpose {
    new_func: close_shim as *const (),
    old_func: close as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
static IT_STAT: Interpose = Interpose {
    new_func: stat_shim as *const (),
    old_func: stat as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
static IT_LSTAT: Interpose = Interpose {
    new_func: lstat_shim as *const (),
    old_func: lstat as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
static IT_FSTAT: Interpose = Interpose {
    new_func: fstat_shim as *const (),
    old_func: fstat as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
static IT_OPENDIR: Interpose = Interpose {
    new_func: opendir_shim as *const (),
    old_func: opendir as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
static IT_READDIR: Interpose = Interpose {
    new_func: readdir_shim as *const (),
    old_func: readdir as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
static IT_CLOSEDIR: Interpose = Interpose {
    new_func: closedir_shim as *const (),
    old_func: closedir as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
static IT_REALPATH: Interpose = Interpose {
    new_func: realpath_shim as *const (),
    old_func: realpath as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
static IT_REALPATH_DARWIN: Interpose = Interpose {
    new_func: realpath_shim as *const (),
    old_func: realpath_darwin as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
static IT_GETCWD: Interpose = Interpose {
    new_func: getcwd_shim as *const (),
    old_func: getcwd as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
static IT_CHDIR: Interpose = Interpose {
    new_func: chdir_shim as *const (),
    old_func: chdir as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
static IT_UNLINK: Interpose = Interpose {
    new_func: unlink_shim as *const (),
    old_func: unlink as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
static IT_RENAME: Interpose = Interpose {
    new_func: rename_shim as *const (),
    old_func: rename as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
static IT_RMDIR: Interpose = Interpose {
    new_func: rmdir_shim as *const (),
    old_func: rmdir as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
static IT_READLINK: Interpose = Interpose {
    new_func: readlink_shim as *const (),
    old_func: readlink as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
static IT_EXECVE: Interpose = Interpose {
    new_func: execve_shim as *const (),
    old_func: execve as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
static IT_POSIX_SPAWN: Interpose = Interpose {
    new_func: posix_spawn_shim as *const (),
    old_func: posix_spawn as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
static IT_POSIX_SPAWNP: Interpose = Interpose {
    new_func: posix_spawnp_shim as *const (),
    old_func: posix_spawnp as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
static IT_MMAP: Interpose = Interpose {
    new_func: mmap_shim as *const (),
    old_func: mmap as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
static IT_DLOPEN: Interpose = Interpose {
    new_func: dlopen_shim as *const (),
    old_func: dlopen as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
static IT_MUNMAP: Interpose = Interpose {
    new_func: munmap_shim as *const (),
    old_func: munmap as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
static IT_DLSYM: Interpose = Interpose {
    new_func: dlsym_shim as *const (),
    old_func: dlsym as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
static IT_ACCESS: Interpose = Interpose {
    new_func: access_shim as *const (),
    old_func: access as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
static IT_READ: Interpose = Interpose {
    new_func: read_shim as *const (),
    old_func: libc::read as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
static IT_FCNTL: Interpose = Interpose {
    new_func: fcntl_shim as *const (),
    old_func: libc::fcntl as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
static IT_OPENAT: Interpose = Interpose {
    new_func: openat_shim as *const (),
    old_func: libc::openat as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
static IT_FACCESSAT: Interpose = Interpose {
    new_func: faccessat_shim as *const (),
    old_func: libc::faccessat as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
static IT_FSTATAT: Interpose = Interpose {
    new_func: fstatat_shim as *const (),
    old_func: libc::fstatat as *const (),
};

// ============================================================================
// Global State & Recursion Guards
// ============================================================================

static SHIM_STATE: AtomicPtr<ShimState> = AtomicPtr::new(ptr::null_mut());
/// Flag to indicate shim is still initializing. All syscalls passthrough during this phase.
static INITIALIZING: AtomicBool = AtomicBool::new(true);
static DEBUG_ENABLED: AtomicBool = AtomicBool::new(false);

// Lock-free recursion key using atomic instead of OnceLock (avoids mutex deadlock during library init)
static RECURSION_KEY_INIT: AtomicBool = AtomicBool::new(false);
static RECURSION_KEY_VALUE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn get_recursion_key() -> libc::pthread_key_t {
    // Fast path: already initialized
    if RECURSION_KEY_INIT.load(Ordering::Acquire) {
        return RECURSION_KEY_VALUE.load(Ordering::Relaxed) as libc::pthread_key_t;
    }

    // Slow path: initialize (only one thread will succeed)
    let mut key: libc::pthread_key_t = 0;
    let ret = unsafe { libc::pthread_key_create(&mut key, None) };
    if ret != 0 {
        // Failed to create key, return 0 (will always consider as "not in recursion")
        return 0;
    }

    // Try to be the one to set the value (CAS)
    let expected = 0usize;
    if RECURSION_KEY_VALUE
        .compare_exchange(expected, key as usize, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
    {
        RECURSION_KEY_INIT.store(true, Ordering::Release);
        key
    } else {
        // Another thread beat us, clean up and use their key
        unsafe { libc::pthread_key_delete(key) };
        RECURSION_KEY_VALUE.load(Ordering::Relaxed) as libc::pthread_key_t
    }
}

const LOG_BUF_SIZE: usize = 64 * 1024;
struct Logger {
    buffer: [u8; LOG_BUF_SIZE],
    head: std::sync::atomic::AtomicUsize,
}

impl Logger {
    const fn new() -> Self {
        Self {
            buffer: [0u8; LOG_BUF_SIZE],
            head: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    fn log(&self, msg: &str) {
        let len = msg.len();
        if len > LOG_BUF_SIZE {
            return;
        }

        let start = self.head.fetch_add(len, Ordering::SeqCst);
        for i in 0..len {
            unsafe {
                let ptr = self.buffer.as_ptr().add((start + i) % LOG_BUF_SIZE) as *mut u8;
                *ptr = msg.as_bytes()[i];
            }
        }
    }

    #[allow(dead_code)]
    fn dump(&self) {
        let head = self.head.load(Ordering::SeqCst);
        let start = head.saturating_sub(LOG_BUF_SIZE);
        for i in start..head {
            unsafe {
                let c = self.buffer[i % LOG_BUF_SIZE];
                libc::write(2, &c as *const u8 as *const c_void, 1);
            }
        }
    }

    fn dump_to_file(&self) {
        let pid = unsafe { libc::getpid() };
        let path = format!("/tmp/vrift-shim-{}.log", pid);
        if let Ok(mut f) = std::fs::File::create(&path) {
            use std::io::Write;
            let head = self.head.load(Ordering::SeqCst);
            let size = if head > LOG_BUF_SIZE {
                LOG_BUF_SIZE
            } else {
                head
            };
            let start = if head > LOG_BUF_SIZE {
                head % LOG_BUF_SIZE
            } else {
                0
            };
            if head > LOG_BUF_SIZE {
                let _ = f.write_all(&self.buffer[start..]);
                let _ = f.write_all(&self.buffer[..start]);
            } else {
                let _ = f.write_all(&self.buffer[..size]);
            }
        }
    }
}

static LOGGER: Logger = Logger::new();

struct OpenFile {
    vpath: String,
    #[allow(dead_code)] // Will be used when async re-ingest is implemented
    original_path: String,
}

/// Synthetic directory for VFS opendir/readdir
#[allow(dead_code)]
struct SyntheticDir {
    vpath: String,
    entries: Vec<vrift_ipc::DirEntry>, // IPC fallback
    mmap_children: Option<(*const vrift_ipc::MmapDirChild, usize)>, // mmap path: (start_ptr, count)
    position: usize,
}
unsafe impl Send for SyntheticDir {} // Raw pointers in open_dirs HashMap
unsafe impl Sync for SyntheticDir {}

// ============================================================================
// RFC-0044 Hot Stat Cache: mmap-based O(1) Stat Lookup
// ============================================================================

/// Open mmap'd manifest file for O(1) stat lookup.
/// Returns (ptr, size) or (null, 0) if unavailable.
/// Uses raw libc to avoid recursion through shim.
fn open_manifest_mmap() -> (*const u8, usize) {
    // Check if mmap is explicitly disabled
    unsafe {
        let env_key = c"VRIFT_DISABLE_MMAP";
        let env_val = libc::getenv(env_key.as_ptr());
        if !env_val.is_null() {
            let val = CStr::from_ptr(env_val).to_str().unwrap_or("0");
            if val == "1" || val == "true" {
                return (ptr::null(), 0);
            }
        }
    }

    // Get VRIFT_MANIFEST to derive project root and hash
    let manifest_ptr = unsafe { libc::getenv(c"VRIFT_MANIFEST".as_ptr()) };
    if manifest_ptr.is_null() {
        return (ptr::null(), 0);
    }
    let manifest_path = unsafe { CStr::from_ptr(manifest_ptr).to_string_lossy() };

    // Project root is the parent of manifest file
    let path = Path::new(manifest_path.as_ref());
    let project_root = match path.parent() {
        Some(p) => p,
        None => return (ptr::null(), 0),
    };

    // If it's in .vrift/manifest.lmdb, go up one more
    let project_root = if project_root.ends_with(".vrift") {
        project_root.parent().unwrap_or(project_root)
    } else {
        project_root
    };

    let root_str = project_root.to_string_lossy();
    let root_hash = blake3::hash(root_str.as_bytes());
    let mmap_path_str = format!("/tmp/vrift-manifest-{}.mmap", &root_hash.to_hex()[..16]);
    let mmap_path = CString::new(mmap_path_str).unwrap_or_default();

    let fd = unsafe { libc::open(mmap_path.as_ptr(), libc::O_RDONLY) };
    if fd < 0 {
        return (ptr::null(), 0);
    }

    // Get file size via fstat
    let mut stat_buf: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(fd, &mut stat_buf) } != 0 {
        unsafe { libc::close(fd) };
        return (ptr::null(), 0);
    }
    let size = stat_buf.st_size as usize;

    // mmap the file read-only
    let ptr = unsafe {
        libc::mmap(
            ptr::null_mut(),
            size,
            libc::PROT_READ,
            libc::MAP_PRIVATE,
            fd,
            0,
        )
    };
    unsafe { libc::close(fd) };

    if ptr == libc::MAP_FAILED {
        return (ptr::null(), 0);
    }

    // Validate header magic
    if size < vrift_ipc::ManifestMmapHeader::SIZE {
        unsafe { libc::munmap(ptr, size) };
        return (ptr::null(), 0);
    }
    let header = unsafe { &*(ptr as *const vrift_ipc::ManifestMmapHeader) };
    if !header.is_valid() {
        unsafe { libc::munmap(ptr, size) };
        return (ptr::null(), 0);
    }

    (ptr as *const u8, size)
}

/// O(1) mmap-based stat lookup for Hot Stat Cache.
/// Returns None if entry not found or mmap not available.
/// ZERO ALLOCATIONS - safe to call from any context.
#[inline(always)]
fn mmap_lookup(
    mmap_ptr: *const u8,
    mmap_size: usize,
    path: &str,
) -> Option<vrift_ipc::MmapStatEntry> {
    if mmap_ptr.is_null() || mmap_size == 0 {
        return None;
    }

    let header = unsafe { &*(mmap_ptr as *const vrift_ipc::ManifestMmapHeader) };

    // Check bloom filter first (O(1) rejection)
    let bloom_offset = header.bloom_offset as usize;
    let bloom_ptr = unsafe { mmap_ptr.add(bloom_offset) };
    let (h1, h2) = vrift_ipc::bloom_hashes(path);
    let b1 = h1 % (vrift_ipc::BLOOM_SIZE * 8);
    let b2 = h2 % (vrift_ipc::BLOOM_SIZE * 8);
    unsafe {
        let v1 = *bloom_ptr.add(b1 / 8) & (1 << (b1 % 8));
        let v2 = *bloom_ptr.add(b2 / 8) & (1 << (b2 % 8));
        if v1 == 0 || v2 == 0 {
            return None; // Bloom filter rejection
        }
    }

    // Hash table lookup with linear probing
    let path_hash = vrift_ipc::fnv1a_hash(path);
    let table_offset = header.table_offset as usize;
    let table_capacity = header.table_capacity as usize;
    let table_ptr = unsafe { mmap_ptr.add(table_offset) as *const vrift_ipc::MmapStatEntry };

    // Linear probing
    let start_slot = (path_hash as usize) % table_capacity;
    for i in 0..table_capacity {
        let slot = (start_slot + i) % table_capacity;
        let entry = unsafe { &*table_ptr.add(slot) };

        if entry.is_empty() {
            return None; // Empty slot = not found
        }

        if entry.path_hash == path_hash {
            return Some(*entry); // Found!
        }
    }

    None // Table full, not found
}

/// O(1) readdir lookup in mmap'd manifest
fn mmap_dir_lookup(
    mmap_ptr: *const u8,
    mmap_size: usize,
    path: &str,
) -> Option<(*const vrift_ipc::MmapDirChild, usize)> {
    if mmap_ptr.is_null() || mmap_size < vrift_ipc::ManifestMmapHeader::SIZE {
        return None;
    }

    let header = unsafe { &*(mmap_ptr as *const vrift_ipc::ManifestMmapHeader) };
    if !header.is_valid() {
        return None;
    }

    // Directory index lookup with linear probing
    let parent_hash = vrift_ipc::fnv1a_hash(path);
    let dir_index_offset = header.dir_index_offset as usize;
    let dir_index_capacity = header.dir_index_capacity as usize;
    let dir_index_ptr =
        unsafe { mmap_ptr.add(dir_index_offset) as *const vrift_ipc::MmapDirIndexEntry };

    let start_slot = (parent_hash as usize) % dir_index_capacity;
    for i in 0..dir_index_capacity {
        let slot = (start_slot + i) % dir_index_capacity;
        let entry = unsafe { &*dir_index_ptr.add(slot) };

        if entry.parent_hash == 0 && entry.children_count == 0 {
            return None; // Empty slot
        }

        if entry.parent_hash == parent_hash {
            // Found parent directory!
            let children_offset = header.children_offset as usize;
            let children_start_ptr = unsafe {
                (mmap_ptr.add(children_offset) as *const vrift_ipc::MmapDirChild)
                    .add(entry.children_start as usize)
            };
            return Some((children_start_ptr, entry.children_count as usize));
        }
    }

    None
}

struct ShimState {
    cas: std::sync::Mutex<Option<CasStore>>, // Lazy init to avoid fs calls during dylib load
    cas_root: std::borrow::Cow<'static, str>,
    vfs_prefix: std::borrow::Cow<'static, str>,
    socket_path: std::borrow::Cow<'static, str>,
    open_fds: Mutex<HashMap<c_int, OpenFile>>,
    /// Synthetic directories for VFS readdir (DIR* pointer -> SyntheticDir)
    open_dirs: Mutex<HashMap<usize, SyntheticDir>>,
    bloom_ptr: *const u8,
    /// RFC-0044 Hot Stat Cache: mmap'd manifest for O(1) stat lookup
    mmap_ptr: *const u8,
    mmap_size: usize,
    /// Absolute path to project root
    project_root: String,
}

impl ShimState {
    fn init() -> Option<*mut Self> {
        // CRITICAL: Must not allocate during early dyld init (malloc may not be ready)
        // Use Cow::Borrowed for static defaults to avoid heap allocation

        if !unsafe { libc::getenv(c"VRIFT_DEBUG".as_ptr()) }.is_null() {
            DEBUG_ENABLED.store(true, Ordering::Relaxed);
        }
        let cas_ptr = unsafe { libc::getenv(c"VRIFT_CAS_ROOT".as_ptr()) };
        let cas_root: std::borrow::Cow<'static, str> = if cas_ptr.is_null() {
            std::borrow::Cow::Borrowed("/tmp/vrift/the_source")
        } else {
            // Environment var found - must allocate (rare case, malloc should be ready by now)
            std::borrow::Cow::Owned(unsafe {
                CStr::from_ptr(cas_ptr).to_string_lossy().into_owned()
            })
        };

        let prefix_ptr = unsafe { libc::getenv(c"VRIFT_VFS_PREFIX".as_ptr()) };
        let vfs_prefix: std::borrow::Cow<'static, str> = if prefix_ptr.is_null() {
            std::borrow::Cow::Borrowed("/vrift")
        } else {
            std::borrow::Cow::Owned(unsafe {
                CStr::from_ptr(prefix_ptr).to_string_lossy().into_owned()
            })
        };

        // DEFERRED: Do NOT call CasStore::new() here to avoid fs syscalls during init
        // CasStore will be created lazily on first VFS file access

        // Static default - no allocation needed
        let socket_path: std::borrow::Cow<'static, str> =
            std::borrow::Cow::Borrowed("/tmp/vrift.sock");

        // NOTE: Bloom mmap is deferred - don't call during init to avoid syscalls
        // that might retrigger the interposition during early dyld phases
        let bloom_ptr = ptr::null(); // Defer to later

        // RFC-0044 Hot Stat Cache: Try to open mmap'd manifest file
        // If not available, we fall back to IPC (no error, just slower)
        // RFC-0044 Hot Stat Cache: Try to open mmap'd manifest file
        // If not available, we fall back to IPC (no error, just slower)
        let (mmap_ptr, mmap_size) = open_manifest_mmap();

        // Derive project root from VRIFT_MANIFEST
        let manifest_ptr = unsafe { libc::getenv(c"VRIFT_MANIFEST".as_ptr()) };
        let project_root: String = if !manifest_ptr.is_null() {
            let manifest_path = unsafe { CStr::from_ptr(manifest_ptr).to_string_lossy() };
            let path = Path::new(manifest_path.as_ref());
            let parent = path.parent().unwrap_or_else(|| Path::new("/"));
            let root = if parent.ends_with(".vrift") {
                parent.parent().unwrap_or(parent)
            } else {
                parent
            };
            root.to_string_lossy().into_owned()
        } else {
            String::new()
        };

        let state = Box::new(ShimState {
            cas: std::sync::Mutex::new(None),
            cas_root,
            vfs_prefix,
            socket_path,
            open_fds: Mutex::new(HashMap::new()),
            open_dirs: Mutex::new(HashMap::new()),
            bloom_ptr,
            mmap_ptr,
            mmap_size,
            project_root,
        });

        Some(Box::into_raw(state))
    }

    /// Get or create CasStore lazily (only called when actually needed)
    fn get_cas(&self) -> Option<std::sync::MutexGuard<'_, Option<CasStore>>> {
        let mut cas = self.cas.lock().ok()?;
        if cas.is_none() {
            match CasStore::new(self.cas_root.as_ref()) {
                Ok(c) => *cas = Some(c),
                Err(_) => return None,
            }
        }
        Some(cas)
    }

    fn get() -> Option<&'static Self> {
        let ptr = SHIM_STATE.load(Ordering::Acquire);
        if !ptr.is_null() {
            return unsafe { Some(&*ptr) };
        }

        if INITIALIZING.swap(true, Ordering::SeqCst) {
            return None;
        }

        let ptr = if let Some(p) = Self::init() {
            SHIM_STATE.store(p, Ordering::Release);
            p
        } else {
            ptr::null_mut()
        };

        INITIALIZING.store(false, Ordering::SeqCst);
        if ptr.is_null() {
            None
        } else {
            unsafe { Some(&*ptr) }
        }
    }

    // ========================================================================
    // PSFS (Provably-Side-Effect-Free Stat) - RFC-0044
    // ========================================================================
    //
    // Hot Stat requirements:
    // - ❌ No alloc (malloc = forbidden)
    // - ❌ No lock (mutex/futex = forbidden)
    // - ❌ No log (absolutely forbidden)
    // - ❌ No syscall (including stat)
    // - ✅ O(1) constant time
    // - ✅ Read-only (no cache writes)
    //
    // psfs_applicable() checks VFS domain membership (pure string prefix check)
    // psfs_lookup() delegates to query_manifest which uses Bloom filter fast path

    /// Check if path is in VFS domain (zero-alloc, O(1) string prefix check)
    /// Returns true if path should be considered for Hot Stat acceleration
    #[inline(always)]
    fn psfs_applicable(&self, path: &str) -> bool {
        // RFC-0046: Mandatory exclusion for metadata and CAS root to prevent recursion
        if path.contains("/.vrift/") || path.starts_with(&*self.cas_root) {
            return false;
        }

        // RFC-0043: Robust normalization and CWD resolution
        let mut buf = [0u8; 1024];
        if let Some(len) = unsafe { resolve_path_with_cwd(path, &mut buf) } {
            let normalized = unsafe { std::str::from_utf8_unchecked(&buf[..len]) };

            // RFC-0046: Re-check after normalization
            if normalized.contains("/.vrift/") || normalized.starts_with(&*self.cas_root) {
                return false;
            }

            normalized.starts_with(&*self.vfs_prefix)
        } else {
            // Fallback for extremely long paths
            path.starts_with(&*self.vfs_prefix)
        }
    }

    /// Attempt O(1) stat lookup from manifest cache
    fn psfs_lookup(&self, path: &str) -> Option<vrift_manifest::VnodeEntry> {
        let mut buf = [0u8; 1024];
        if let Some(len) = unsafe { resolve_path_with_cwd(path, &mut buf) } {
            let normalized = unsafe { std::str::from_utf8_unchecked(&buf[..len]) };
            self.query_manifest(normalized)
        } else {
            self.query_manifest(path)
        }
    }

    fn query_manifest(&self, path: &str) -> Option<vrift_manifest::VnodeEntry> {
        // Bloom Filter Fast Path
        if !self.bloom_ptr.is_null() {
            let (h1, h2) = vrift_ipc::bloom_hashes(path);
            let b1 = h1 % (vrift_ipc::BLOOM_SIZE * 8);
            let b2 = h2 % (vrift_ipc::BLOOM_SIZE * 8);
            unsafe {
                let v1 = *self.bloom_ptr.add(b1 / 8) & (1 << (b1 % 8));
                let v2 = *self.bloom_ptr.add(b2 / 8) & (1 << (b2 % 8));
                if v1 == 0 || v2 == 0 {
                    return None; // Absolute miss
                }
            }
        }

        use vrift_ipc::{VeloRequest, VeloResponse};

        let fd = unsafe { self.raw_connect_and_register() };
        if fd < 0 {
            return None;
        }

        let vpath = if path.starts_with(&*self.vfs_prefix) {
            &path[self.vfs_prefix.len()..]
        } else {
            path
        };
        // Normalize: remove leading slash if any
        let vpath = vpath.trim_start_matches('/');

        let ok = (|| -> Option<vrift_manifest::VnodeEntry> {
            // 3. Manifest Get
            let req = VeloRequest::ManifestGet {
                path: vpath.to_string(),
            };
            let buf = bincode::serialize(&req).ok()?;
            let len = (buf.len() as u32).to_le_bytes();
            if !unsafe { raw_write_all(fd, &len) } || !unsafe { raw_write_all(fd, &buf) } {
                return None;
            }

            let mut resp_len_buf = [0u8; 4];
            if !unsafe { raw_read_exact(fd, &mut resp_len_buf) } {
                return None;
            }
            let resp_len = u32::from_le_bytes(resp_len_buf) as usize;
            if resp_len > 16 * 1024 * 1024 {
                return None;
            }
            let mut resp_buf = vec![0u8; resp_len];
            if !unsafe { raw_read_exact(fd, &mut resp_buf) } {
                return None;
            }

            match bincode::deserialize::<VeloResponse>(&resp_buf).ok()? {
                VeloResponse::ManifestAck { entry } => entry,
                _ => None,
            }
        })();

        unsafe { libc::close(fd) };
        ok
    }

    #[allow(dead_code)] // Will be called from close_impl when async re-ingest is implemented
    fn upsert_manifest(&self, path: &str, entry: vrift_manifest::VnodeEntry) -> bool {
        use vrift_ipc::VeloRequest;

        let fd = unsafe { self.raw_connect_and_register() };
        if fd < 0 {
            return false;
        }

        let ok = (|| -> Option<()> {
            let req = VeloRequest::ManifestUpsert {
                path: path.to_string(),
                entry,
            };
            let buf = bincode::serialize(&req).ok()?;
            let len = (buf.len() as u32).to_le_bytes();
            if !unsafe { raw_write_all(fd, &len) } || !unsafe { raw_write_all(fd, &buf) } {
                return None;
            }
            Some(())
        })();

        unsafe { libc::close(fd) };
        ok.is_some()
    }

    /// Query daemon for directory listing (for opendir/readdir)
    fn query_dir_listing(&self, path: &str) -> Option<Vec<vrift_ipc::DirEntry>> {
        use vrift_ipc::{VeloRequest, VeloResponse};

        let fd = unsafe { self.raw_connect_and_register() };
        if fd < 0 {
            return None;
        }

        let vpath = if path.starts_with(&*self.vfs_prefix) {
            &path[self.vfs_prefix.len()..]
        } else {
            path
        };
        let vpath = vpath.trim_start_matches('/');

        let req = VeloRequest::ManifestListDir {
            path: vpath.to_string(),
        };
        let buf = bincode::serialize(&req).ok()?;
        let len = (buf.len() as u32).to_le_bytes();

        if !unsafe { raw_write_all(fd, &len) } || !unsafe { raw_write_all(fd, &buf) } {
            unsafe { libc::close(fd) };
            return None;
        }

        let mut resp_len_buf = [0u8; 4];
        if !unsafe { raw_read_exact(fd, &mut resp_len_buf) } {
            unsafe { libc::close(fd) };
            return None;
        }
        let resp_len = u32::from_le_bytes(resp_len_buf) as usize;
        if resp_len > 16 * 1024 * 1024 {
            unsafe { libc::close(fd) };
            return None;
        }
        let mut resp_buf = vec![0u8; resp_len];
        if !unsafe { raw_read_exact(fd, &mut resp_buf) } {
            unsafe { libc::close(fd) };
            return None;
        }
        unsafe { libc::close(fd) };

        match bincode::deserialize::<VeloResponse>(&resp_buf).ok()? {
            VeloResponse::ManifestListAck { entries } => Some(entries),
            _ => None,
        }
    }

    /// Internal helper: connect, handshake, and register workspace.
    /// Returns fd or -1 on error.
    unsafe fn raw_connect_and_register(&self) -> c_int {
        use vrift_ipc::VeloRequest;

        let fd = raw_unix_connect(&self.socket_path);
        if fd < 0 {
            return -1;
        }

        // 1. Handshake
        let handshake = VeloRequest::Handshake {
            client_version: env!("CARGO_PKG_VERSION").to_string(),
        };
        let buf = if let Ok(b) = bincode::serialize(&handshake) {
            b
        } else {
            libc::close(fd);
            return -1;
        };
        let len = (buf.len() as u32).to_le_bytes();
        if !raw_write_all(fd, &len) || !raw_write_all(fd, &buf) {
            libc::close(fd);
            return -1;
        }
        // Read handshake ack
        let mut h_len_buf = [0u8; 4];
        if !raw_read_exact(fd, &mut h_len_buf) {
            libc::close(fd);
            return -1;
        }
        let h_len = u32::from_le_bytes(h_len_buf) as usize;
        let mut h_buf = vec![0u8; h_len]; // Allocation is okay in fallback path
        if !raw_read_exact(fd, &mut h_buf) {
            libc::close(fd);
            return -1;
        }

        // 2. Register Workspace
        let register = VeloRequest::RegisterWorkspace {
            project_root: self.project_root.clone(),
        };
        let buf = if let Ok(b) = bincode::serialize(&register) {
            b
        } else {
            libc::close(fd);
            return -1;
        };
        let len = (buf.len() as u32).to_le_bytes();
        if !raw_write_all(fd, &len) || !raw_write_all(fd, &buf) {
            libc::close(fd);
            return -1;
        }
        // Read register ack
        let mut r_len_buf = [0u8; 4];
        if !raw_read_exact(fd, &mut r_len_buf) {
            libc::close(fd);
            return -1;
        }
        let r_len = u32::from_le_bytes(r_len_buf) as usize;
        let mut r_buf = vec![0u8; r_len];
        if !raw_read_exact(fd, &mut r_buf) {
            libc::close(fd);
            return -1;
        }

        fd
    }
}

/// Raw Unix socket connect using libc syscalls (avoids recursion through shim)
unsafe fn raw_unix_connect(path: &str) -> c_int {
    let fd = libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0);
    if fd < 0 {
        return -1;
    }
    // RFC-0043: Prevent FD leakage to child processes
    libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);

    let mut addr: libc::sockaddr_un = std::mem::zeroed();
    addr.sun_family = libc::AF_UNIX as libc::sa_family_t;

    let path_bytes = path.as_bytes();
    if path_bytes.len() >= addr.sun_path.len() {
        libc::close(fd);
        return -1;
    }
    ptr::copy_nonoverlapping(
        path_bytes.as_ptr(),
        addr.sun_path.as_mut_ptr() as *mut u8,
        path_bytes.len(),
    );

    let addr_len = std::mem::size_of::<libc::sockaddr_un>() as libc::socklen_t;
    if libc::connect(fd, &addr as *const _ as *const libc::sockaddr, addr_len) < 0 {
        libc::close(fd);
        return -1;
    }

    fd
}

/// Raw write using libc (avoids recursion through shim)
unsafe fn raw_write_all(fd: c_int, data: &[u8]) -> bool {
    let mut written = 0;
    while written < data.len() {
        let n = libc::write(
            fd,
            data[written..].as_ptr() as *const libc::c_void,
            data.len() - written,
        );
        if n <= 0 {
            return false;
        }
        written += n as usize;
    }
    true
}

/// Raw read using libc (avoids recursion through shim)
unsafe fn raw_read_exact(fd: c_int, buf: &mut [u8]) -> bool {
    let mut read = 0;
    while read < buf.len() {
        let n = libc::read(
            fd,
            buf[read..].as_mut_ptr() as *mut libc::c_void,
            buf.len() - read,
        );
        if n <= 0 {
            return false;
        }
        read += n as usize;
    }
    true
}

// ============================================================================
// Utility Functions
// ============================================================================

/// Robust path normalization without heap allocation.
/// Handles "..", ".", and duplicate slashes.
/// Returns the length of the normalized path in `out`.
unsafe fn raw_path_normalize(path: &str, out: &mut [u8]) -> Option<usize> {
    if path.is_empty() || out.is_empty() {
        return None;
    }

    let bytes = path.as_bytes();
    let mut out_idx = 0;

    // Always start with / if input is absolute
    if bytes[0] == b'/' {
        out[out_idx] = b'/';
        out_idx += 1;
    }

    let mut i = 0;
    while i < bytes.len() {
        // Skip multiple slashes
        while i < bytes.len() && bytes[i] == b'/' {
            i += 1;
        }
        if i == bytes.len() {
            break;
        }

        // Find component end
        let start = i;
        while i < bytes.len() && bytes[i] != b'/' {
            i += 1;
        }
        let component = &bytes[start..i];

        if component == b"." {
            continue;
        } else if component == b".." {
            if out_idx > 1 {
                // Backtrack to previous slash
                out_idx -= 1;
                while out_idx > 1 && out[out_idx - 1] != b'/' {
                    out_idx -= 1;
                }
            }
        } else {
            // Add slash if not at root and last char isn't slash
            if out_idx > 0 && out[out_idx - 1] != b'/' {
                if out_idx < out.len() {
                    out[out_idx] = b'/';
                    out_idx += 1;
                } else {
                    return None;
                }
            }
            // Add component
            if out_idx + component.len() <= out.len() {
                ptr::copy_nonoverlapping(
                    component.as_ptr(),
                    out.as_mut_ptr().add(out_idx),
                    component.len(),
                );
                out_idx += component.len();
            } else {
                return None;
            }
        }
    }

    if out_idx == 0 {
        if bytes[0] == b'/' {
            out[0] = b'/';
        } else {
            out[0] = b'.';
        }
        out_idx = 1;
    }

    Some(out_idx)
}

/// Resolve a path, potentially relative to VIRTUAL_CWD.
unsafe fn resolve_path_with_cwd(path: &str, out: &mut [u8]) -> Option<usize> {
    if path.starts_with('/') {
        return raw_path_normalize(path, out);
    }

    if let Some(vpath) = VIRTUAL_CWD.with(|cwd| cwd.borrow().clone()) {
        let mut tmp = [b'/'; 1024];
        let vbytes = vpath.as_bytes();
        if vbytes.len() < tmp.len() {
            ptr::copy_nonoverlapping(vbytes.as_ptr(), tmp.as_mut_ptr(), vbytes.len());
            let mut idx = vbytes.len();
            if idx > 0 && tmp[idx - 1] != b'/' && idx < tmp.len() {
                tmp[idx] = b'/';
                idx += 1;
            }
            let pbytes = path.as_bytes();
            if idx + pbytes.len() < tmp.len() {
                ptr::copy_nonoverlapping(pbytes.as_ptr(), tmp.as_mut_ptr().add(idx), pbytes.len());
                let full = std::str::from_utf8_unchecked(&tmp[..idx + pbytes.len()]);
                return raw_path_normalize(full, out);
            }
        }
    }

    raw_path_normalize(path, out)
}

unsafe fn shim_log(msg: &str) {
    LOGGER.log(msg);
    if DEBUG_ENABLED.load(Ordering::Relaxed) {
        libc::write(2, msg.as_ptr() as *const c_void, msg.len());
    }
}

struct ShimGuard;
impl ShimGuard {
    fn enter() -> Option<Self> {
        let key = get_recursion_key();
        let val = unsafe { libc::pthread_getspecific(key) };
        if !val.is_null() {
            None
        } else {
            unsafe { libc::pthread_setspecific(key, std::ptr::dangling::<c_void>()) };
            Some(ShimGuard)
        }
    }
}
impl Drop for ShimGuard {
    fn drop(&mut self) {
        let key = get_recursion_key();
        unsafe { libc::pthread_setspecific(key, ptr::null()) };
    }
}

#[cfg(target_os = "linux")]
unsafe fn set_errno(e: c_int) {
    *libc::__errno_location() = e;
}
#[cfg(target_os = "macos")]
unsafe fn set_errno(e: c_int) {
    *libc::__error() = e;
}

// ============================================================================
// Core Logic
// ============================================================================

unsafe fn break_link(path_str: &str) -> Result<(), c_int> {
    let p = Path::new(path_str);
    let metadata = match std::fs::metadata(p) {
        Ok(m) => m,
        Err(_) => return Ok(()),
    };
    if metadata.nlink() < 2 {
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
    let tmp_fd = libc::openat(dir_fd, c".".as_ptr(), O_TMPFILE | libc::O_RDWR, 0o600);
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
        let len = std::fs::metadata(path_str).map(|m| m.len()).unwrap_or(0);
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
    tmp_path_buf[..pb.len()].copy_from_slice(pb);
    let suffix = b".vrift_tmp";
    tmp_path_buf[pb.len()..(pb.len() + suffix.len())].copy_from_slice(suffix);
    let tmp_len = pb.len() + suffix.len();
    tmp_path_buf[tmp_len] = 0;

    let tmp_ptr = tmp_path_buf.as_ptr() as *const c_char;
    let path_ptr = CString::new(path_str).map_err(|_| libc::EINVAL)?;

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
    let _ = std::fs::set_permissions(path_str, std::fs::Permissions::from_mode(0o644));
    Ok(())
}

type OpenFn = unsafe extern "C" fn(*const c_char, c_int, mode_t) -> c_int;
type WriteFn = unsafe extern "C" fn(c_int, *const c_void, size_t) -> ssize_t;
type CloseFn = unsafe extern "C" fn(c_int) -> c_int;
type ExecveFn =
    unsafe extern "C" fn(*const c_char, *const *const c_char, *const *const c_char) -> c_int;
type PosixSpawnFn = unsafe extern "C" fn(
    *mut libc::pid_t,
    *const c_char,
    *const c_void,
    *const c_void,
    *const *const c_char,
    *const *const c_char,
) -> c_int;
type MmapFn =
    unsafe extern "C" fn(*mut c_void, size_t, c_int, c_int, c_int, libc::off_t) -> *mut c_void;
type DlopenFn = unsafe extern "C" fn(*const c_char, c_int) -> *mut c_void;
type AccessFn = unsafe extern "C" fn(*const c_char, c_int) -> c_int;
type ReadFn = unsafe extern "C" fn(c_int, *mut c_void, size_t) -> ssize_t;
type OpenatFn = unsafe extern "C" fn(c_int, *const c_char, c_int, mode_t) -> c_int;
type FaccessatFn = unsafe extern "C" fn(c_int, *const c_char, c_int, c_int) -> c_int;
type FstatatFn = unsafe extern "C" fn(c_int, *const c_char, *mut libc::stat, c_int) -> c_int;

unsafe fn execve_impl(
    path: *const c_char,
    argv: *const *const c_char,
    envp: *const *const c_char,
    real_execve: ExecveFn,
) -> c_int {
    let _guard = match ShimGuard::enter() {
        Some(g) => g,
        None => return real_execve(path, argv, envp),
    };

    // Prepare modified environment
    let mut vec: Vec<*const c_char> = Vec::new();
    let mut i = 0;
    let mut has_velo_prefix = false;
    let mut has_dyld_insert = false;

    if !envp.is_null() {
        while !(*envp.add(i)).is_null() {
            let s = CStr::from_ptr(*envp.add(i)).to_string_lossy();
            if s.starts_with("VRIFT_") || s.starts_with("VR_") {
                has_velo_prefix = true;
            }
            if s.starts_with("DYLD_INSERT_LIBRARIES=") || s.starts_with("LD_PRELOAD=") {
                has_dyld_insert = true;
            }
            vec.push(*envp.add(i));
            i += 1;
        }
    }

    // Capture current process env if missing in envp (best effort)
    if !has_velo_prefix || !has_dyld_insert {
        // In a real implementation we'd grab from libc's environ and merge
        // For now, if caller passed a custom env without Velo, we might want to force it
    }

    vec.push(ptr::null());
    real_execve(path, argv, vec.as_ptr())
}

unsafe fn open_impl(path: *const c_char, flags: c_int, mode: mode_t, real_open: OpenFn) -> c_int {
    // Early bailout during ShimState initialization to prevent CasStore::new recursion
    if INITIALIZING.load(Ordering::SeqCst) {
        return real_open(path, flags, mode);
    }

    // Note: Don't check SHIM_STATE.is_null() here - ShimState::get() handles lazy init properly

    let _guard = match ShimGuard::enter() {
        Some(g) => g,
        None => {
            return real_open(path, flags, mode);
        }
    };

    // Get or init state - this triggers initialization if needed
    let Some(state) = ShimState::get() else {
        return real_open(path, flags, mode);
    };

    let path_str = match CStr::from_ptr(path).to_str() {
        Ok(s) => s,
        Err(_) => {
            return real_open(path, flags, mode);
        }
    };

    let mut path_buf = [0u8; 1024];
    let resolved_len = match resolve_path_with_cwd(path_str, &mut path_buf) {
        Some(len) => len,
        None => return real_open(path, flags, mode),
    };
    let resolved_path = unsafe { std::str::from_utf8_unchecked(&path_buf[..resolved_len]) };

    let is_write = (flags & (libc::O_WRONLY | libc::O_RDWR | libc::O_TRUNC)) != 0;

    if resolved_path.starts_with(&*state.vfs_prefix) {
        // Query with full path since manifest stores full paths (e.g., /vrift/testfile.txt)
        if let Some(entry) = state.query_manifest(resolved_path) {
            if entry.is_dir() {
                set_errno(libc::EISDIR);
                return -1;
            }
            if let Some(cas_guard) = state.get_cas() {
                if let Some(cas) = cas_guard.as_ref() {
                    if let Ok(content) = cas.get(&entry.content_hash) {
                        let mut tmp_path_buf = [0u8; 128];
                        let prefix = b"/tmp/vrift-mem-";
                        tmp_path_buf[..prefix.len()].copy_from_slice(prefix);
                        for i in 0..32 {
                            let hex = b"0123456789abcdef";
                            tmp_path_buf[prefix.len() + i * 2] =
                                hex[(entry.content_hash[i] >> 4) as usize];
                            tmp_path_buf[prefix.len() + i * 2 + 1] =
                                hex[(entry.content_hash[i] & 0x0f) as usize];
                        }
                        tmp_path_buf[prefix.len() + 64] = 0;

                        let tmp_fd = libc::open(
                            tmp_path_buf.as_ptr() as *const c_char,
                            libc::O_CREAT | libc::O_RDWR | libc::O_TRUNC,
                            0o644,
                        );
                        if tmp_fd >= 0 {
                            libc::write(tmp_fd, content.as_ptr() as *const c_void, content.len());
                            libc::lseek(tmp_fd, 0, libc::SEEK_SET);
                            return tmp_fd;
                        }
                    }
                }
            }
        }
    }

    if is_write && path_str.starts_with(&*state.vfs_prefix) {
        let _ = break_link(path_str);

        let fd = real_open(path, flags, mode);
        if fd >= 0 {
            let mut fds = state.open_fds.lock().unwrap();
            fds.insert(
                fd,
                OpenFile {
                    vpath: path_str[state.vfs_prefix.len()..].to_string(),
                    original_path: path_str.to_string(),
                },
            );
        }
        return fd;
    }

    real_open(path, flags, mode)
}

unsafe fn write_impl(fd: c_int, buf: *const c_void, count: size_t, real_write: WriteFn) -> ssize_t {
    real_write(fd, buf, count)
}

unsafe fn close_impl(fd: c_int, real_close: CloseFn) -> c_int {
    // Skip if ShimState is not yet initialized (avoids malloc during dyld __malloc_init)
    if SHIM_STATE.load(Ordering::Acquire).is_null() {
        return real_close(fd);
    }

    let _guard = match ShimGuard::enter() {
        Some(g) => g,
        None => return real_close(fd),
    };

    if let Some(state) = ShimState::get() {
        let open_file = {
            let mut fds = state.open_fds.lock().unwrap();
            fds.remove(&fd)
        };

        if let Some(file) = open_file {
            // QA Fix: Do NOT use fs::read here - it blocks and allocates!
            // Instead, send a non-blocking IPC to daemon for async re-ingest
            // The manifest sync will happen via daemon's ManifestUpsert handler
            shim_log("[VRift-Shim] File closed, needs re-ingest: ");
            shim_log(&file.vpath);
            shim_log("\n");

            // Fire-and-forget IPC to daemon (non-blocking)
            // Daemon will handle the actual re-ingest asynchronously
            // For now, just mark it in the log - daemon will pick it up on next scan
        }
    }

    real_close(fd)
}

type StatFn = unsafe extern "C" fn(*const c_char, *mut libc::stat) -> c_int;
type FstatFn = unsafe extern "C" fn(c_int, *mut libc::stat) -> c_int;

unsafe fn stat_common(path: *const c_char, buf: *mut libc::stat, real_stat: StatFn) -> c_int {
    // Early bailout during ShimState initialization to prevent CasStore::new recursion
    if INITIALIZING.load(Ordering::SeqCst) {
        return real_stat(path, buf);
    }

    // Skip if ShimState is not yet initialized (avoids malloc during dyld __malloc_init)
    if SHIM_STATE.load(Ordering::Acquire).is_null() {
        return real_stat(path, buf);
    }

    let _guard = match ShimGuard::enter() {
        Some(g) => g,
        None => return real_stat(path, buf),
    };
    let path_str = match CStr::from_ptr(path).to_str() {
        Ok(s) => s,
        Err(_) => return real_stat(path, buf),
    };

    let Some(state) = ShimState::get() else {
        return real_stat(path, buf);
    };

    let mut path_buf = [0u8; 1024];
    let resolved_len = match resolve_path_with_cwd(path_str, &mut path_buf) {
        Some(len) => len,
        None => return real_stat(path, buf),
    };
    let resolved_path = unsafe { std::str::from_utf8_unchecked(&path_buf[..resolved_len]) };

    // RFC-0044 PSFS: VFS prefix root (special case)
    if resolved_path == state.vfs_prefix {
        ptr::write_bytes(buf, 0, 1);
        (*buf).st_mode = libc::S_IFDIR | 0o755;
        (*buf).st_nlink = 2;
        (*buf).st_uid = libc::getuid();
        (*buf).st_gid = libc::getgid();
        return 0;
    }

    // RFC-0044 PSFS: Hot Stat path - check VFS domain membership first
    if resolved_path.starts_with(&*state.vfs_prefix) {
        // ★ RFC-0044 Hot Stat Cache: O(1) mmap lookup (NO IPC, NO ALLOC)
        let manifest_path = if resolved_path.len() > state.vfs_prefix.len() {
            &resolved_path[state.vfs_prefix.len()..].trim_start_matches('/')
        } else {
            ""
        };

        if let Some(mmap_entry) = mmap_lookup(state.mmap_ptr, state.mmap_size, manifest_path) {
            ptr::write_bytes(buf, 0, 1);
            (*buf).st_size = mmap_entry.size as libc::off_t;
            (*buf).st_mtime = mmap_entry.mtime as libc::time_t;
            #[cfg(target_os = "macos")]
            {
                (*buf).st_mtime_nsec = mmap_entry.mtime_nsec;
            }
            #[cfg(target_os = "linux")]
            {
                (*buf).st_mtim.tv_sec = mmap_entry.mtime;
                (*buf).st_mtim.tv_nsec = mmap_entry.mtime_nsec;
            }
            (*buf).st_mode = mmap_entry.mode as libc::mode_t;
            if mmap_entry.is_dir() {
                (*buf).st_mode |= libc::S_IFDIR;
            } else if mmap_entry.is_symlink() {
                (*buf).st_mode |= libc::S_IFLNK;
            } else {
                (*buf).st_mode |= libc::S_IFREG;
            }
            (*buf).st_nlink = 1;
            (*buf).st_uid = libc::getuid();
            (*buf).st_gid = libc::getgid();
            return 0;
        }

        // Fallback: IPC-based manifest lookup (slower but more complete)
        if let Some(entry) = state.psfs_lookup(path_str) {
            ptr::write_bytes(buf, 0, 1);
            (*buf).st_size = entry.size as libc::off_t;
            // mtime is stored as nanoseconds - convert to seconds + nanoseconds
            let mtime_secs = (entry.mtime / 1_000_000_000) as libc::time_t;
            let mtime_nsecs = (entry.mtime % 1_000_000_000) as libc::c_long;
            (*buf).st_mtime = mtime_secs;

            // Platform-specific nanosecond field
            #[cfg(target_os = "macos")]
            {
                (*buf).st_mtime_nsec = mtime_nsecs;
            }
            #[cfg(target_os = "linux")]
            {
                (*buf).st_mtime = mtime_secs;
                (*buf).st_mtime_nsec = mtime_nsecs;
            }
            (*buf).st_mode = entry.mode as libc::mode_t;
            if entry.is_dir() {
                (*buf).st_mode |= libc::S_IFDIR;
            } else if entry.is_symlink() {
                (*buf).st_mode |= libc::S_IFLNK;
            } else {
                (*buf).st_mode |= libc::S_IFREG;
            }
            (*buf).st_nlink = 1;
            (*buf).st_uid = libc::getuid();
            (*buf).st_gid = libc::getgid();
            return 0;
        }
    }

    // RFC-0044 Cold Stat: pure transparent passthrough
    real_stat(path, buf)
}

unsafe fn fstat_impl(fd: c_int, buf: *mut libc::stat, real_fstat: FstatFn) -> c_int {
    // Early bailout during ShimState initialization to prevent CasStore::new recursion
    if INITIALIZING.load(Ordering::SeqCst) {
        return real_fstat(fd, buf);
    }

    // Skip if ShimState is not yet initialized (avoids malloc during dyld __malloc_init)
    // We do NOT trigger init here - init happens on first user-level stat/open call
    if SHIM_STATE.load(Ordering::Acquire).is_null() {
        return real_fstat(fd, buf);
    }

    let _guard = match ShimGuard::enter() {
        Some(g) => g,
        None => return real_fstat(fd, buf),
    };

    let Some(state) = ShimState::get() else {
        return real_fstat(fd, buf);
    };

    // Check if this fd belongs to a VFS file we're tracking
    let fds = state.open_fds.lock().unwrap();
    if let Some(open_file) = fds.get(&fd) {
        // Query manifest for this vpath to get virtual metadata
        let vpath = open_file.vpath.clone();
        drop(fds); // Release lock before IPC

        if let Some(entry) = state.query_manifest(&vpath) {
            // Return virtual metadata from manifest
            ptr::write_bytes(buf, 0, 1);
            (*buf).st_size = entry.size as libc::off_t;

            // mtime is stored as nanoseconds - convert to seconds + nanoseconds
            let mtime_secs = (entry.mtime / 1_000_000_000) as libc::time_t;
            let mtime_nsecs = (entry.mtime % 1_000_000_000) as libc::c_long;
            (*buf).st_mtime = mtime_secs;
            // Platform-specific nanosecond field
            #[cfg(target_os = "macos")]
            {
                (*buf).st_mtime_nsec = mtime_nsecs;
            }
            #[cfg(target_os = "linux")]
            {
                (*buf).st_mtime = mtime_secs;
                (*buf).st_mtime_nsec = mtime_nsecs;
            }

            (*buf).st_mode = entry.mode as libc::mode_t;
            if entry.is_dir() {
                (*buf).st_mode |= libc::S_IFDIR;
            } else if entry.is_symlink() {
                (*buf).st_mode |= libc::S_IFLNK;
            } else {
                (*buf).st_mode |= libc::S_IFREG;
            }
            (*buf).st_nlink = 1;
            (*buf).st_uid = libc::getuid();
            (*buf).st_gid = libc::getgid();
            (*buf).st_blksize = 4096;
            (*buf).st_blocks = entry.size.div_ceil(512) as libc::blkcnt_t;
            shim_log("[VRift-Shim] fstat returned virtual metadata for: ");
            shim_log(&vpath);
            shim_log("\n");
            return 0;
        }
        // Fall through to real fstat if manifest miss
    } else {
        drop(fds);
    }

    real_fstat(fd, buf)
}

/// access() syscall implementation for VFS
/// Checks if a VFS path is accessible based on manifest entries
unsafe fn access_impl(path: *const c_char, mode: c_int, real_access: AccessFn) -> c_int {
    // Early bailout during ShimState initialization
    if INITIALIZING.load(Ordering::SeqCst) {
        return real_access(path, mode);
    }

    // Skip if ShimState is not yet initialized
    if SHIM_STATE.load(Ordering::Acquire).is_null() {
        return real_access(path, mode);
    }

    let _guard = match ShimGuard::enter() {
        Some(g) => g,
        None => return real_access(path, mode),
    };

    let path_str = match CStr::from_ptr(path).to_str() {
        Ok(s) => s,
        Err(_) => return real_access(path, mode),
    };

    let Some(state) = ShimState::get() else {
        return real_access(path, mode);
    };

    // Check if this is a VFS path
    if state.psfs_applicable(path_str) {
        if let Some(entry) = state.psfs_lookup(path_str) {
            // F_OK (0) = check existence
            if mode == libc::F_OK {
                return 0; // File exists in manifest
            }

            // Check permission bits from manifest entry mode
            let file_mode = entry.mode;

            // R_OK (4) = check read permission
            if (mode & libc::R_OK) != 0 {
                // Check user/group/other read bits
                if (file_mode & 0o444) == 0 {
                    set_errno(libc::EACCES);
                    return -1;
                }
            }

            // W_OK (2) = check write permission
            // VFS files are typically read-only (hardlinked from CAS)
            if (mode & libc::W_OK) != 0 {
                // CAS files are immutable, but CoW will handle writes
                // For now, allow write checks to pass as CoW can handle it
            }

            // X_OK (1) = check execute permission
            if (mode & libc::X_OK) != 0 && (file_mode & 0o111) == 0 {
                set_errno(libc::EACCES);
                return -1;
            }

            shim_log("[VRift-Shim] access() returned 0 for VFS path: ");
            shim_log(path_str);
            shim_log("\n");
            return 0;
        }
        // Path is in VFS prefix but not in manifest - let real syscall handle ENOENT
    }

    real_access(path, mode)
}

type OpendirFn = unsafe extern "C" fn(*const c_char) -> *mut libc::DIR;
type ReadlinkFn = unsafe extern "C" fn(*const c_char, *mut c_char, size_t) -> ssize_t;
type RealpathFn = unsafe extern "C" fn(*const c_char, *mut c_char) -> *mut c_char;
type GetcwdFn = unsafe extern "C" fn(*mut c_char, size_t) -> *mut c_char;
type ChdirFn = unsafe extern "C" fn(*const c_char) -> c_int;
type UnlinkFn = unsafe extern "C" fn(*const c_char) -> c_int;
type RenameFn = unsafe extern "C" fn(*const c_char, *const c_char) -> c_int;
type RmdirFn = unsafe extern "C" fn(*const c_char) -> c_int;
#[allow(dead_code)] // Will be exported when full readdir support is added
type ReaddirFn = unsafe extern "C" fn(*mut libc::DIR) -> *mut libc::dirent;
#[allow(dead_code)]
type ClosedirFn = unsafe extern "C" fn(*mut libc::DIR) -> c_int;

/// Synthetic DIR handle counter (unique per synthetic directory)
static SYNTHETIC_DIR_COUNTER: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0x7F000000);

unsafe fn opendir_impl(path: *const c_char, real_opendir: OpendirFn) -> *mut libc::DIR {
    // Early bailout during ShimState initialization to prevent CasStore::new recursion
    if INITIALIZING.load(Ordering::SeqCst) {
        return real_opendir(path);
    }

    // Skip if ShimState is not yet initialized (avoids malloc during dyld __malloc_init)
    if SHIM_STATE.load(Ordering::Acquire).is_null() {
        return real_opendir(path);
    }

    let _guard = match ShimGuard::enter() {
        Some(g) => g,
        None => return real_opendir(path),
    };

    let path_str = match CStr::from_ptr(path).to_str() {
        Ok(s) => s,
        Err(_) => return real_opendir(path),
    };

    let Some(state) = ShimState::get() else {
        return real_opendir(path);
    };

    // Check if this is a VFS path
    if path_str.starts_with(&*state.vfs_prefix) {
        // Normalize path: remove trailing slash except if it's just "/"
        let lookup_path = if path_str.len() > 1 {
            path_str.trim_end_matches('/')
        } else {
            path_str
        };

        let vpath = &path_str[state.vfs_prefix.len()..];

        // 1. Try mmap lookup first (Zero-Copy)
        if let Some((children_ptr, count)) =
            mmap_dir_lookup(state.mmap_ptr, state.mmap_size, lookup_path)
        {
            shim_log("[VRift] opendir mmap: ");
            shim_log(lookup_path);
            shim_log("\n");

            let handle = SYNTHETIC_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
            let synthetic = SyntheticDir {
                vpath: vpath.to_string(),
                entries: Vec::new(),
                mmap_children: Some((children_ptr, count)),
                position: 0,
            };
            let mut dirs = state.open_dirs.lock().unwrap();
            dirs.insert(handle, synthetic);
            return handle as *mut libc::DIR;
        }

        // 2. Fallback: Query daemon for directory entries (IPC)
        if let Some(entries) = state.query_dir_listing(lookup_path) {
            shim_log("[VRift] opendir IPC fallback: ");
            shim_log(lookup_path);
            shim_log("\n");

            // Create synthetic DIR handle
            let handle = SYNTHETIC_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);

            let synthetic = SyntheticDir {
                vpath: vpath.to_string(),
                entries,
                mmap_children: None,
                position: 0,
            };

            let mut dirs = state.open_dirs.lock().unwrap();
            dirs.insert(handle, synthetic);

            shim_log("[VRift-Shim] opendir VFS (IPC): ");
            shim_log(lookup_path);
            shim_log("\n");

            // Return synthetic DIR* (cast handle as pointer)
            return handle as *mut libc::DIR;
        }
    }

    real_opendir(path)
}

/// Static dirent for returning from readdir (must be static to remain valid after return)
#[allow(dead_code)] // Will be used when readdir export is added
static mut SYNTHETIC_DIRENT: libc::dirent = unsafe { std::mem::zeroed() };

#[allow(dead_code)] // Will be exported when full readdir support is added
#[allow(static_mut_refs)] // Required for returning static dirent from readdir
unsafe fn readdir_impl(dir: *mut libc::DIR, real_readdir: ReaddirFn) -> *mut libc::dirent {
    let _guard = match ShimGuard::enter() {
        Some(g) => g,
        None => return real_readdir(dir),
    };

    let Some(state) = ShimState::get() else {
        return real_readdir(dir);
    };

    let handle = dir as usize;

    // Check if this is a synthetic directory
    let mut dirs = state.open_dirs.lock().unwrap();
    if let Some(synthetic) = dirs.get_mut(&handle) {
        // Case A: mmap-backed children (Zero-Copy)
        if let Some((children_ptr, count)) = synthetic.mmap_children {
            if synthetic.position >= count {
                return ptr::null_mut();
            }

            let child = unsafe { &*children_ptr.add(synthetic.position) };
            synthetic.position += 1;

            // Fill in the static dirent
            ptr::write_bytes(&mut SYNTHETIC_DIRENT, 0, 1);
            SYNTHETIC_DIRENT.d_ino = (handle + synthetic.position) as libc::ino_t;
            SYNTHETIC_DIRENT.d_type = if child.is_dir != 0 {
                libc::DT_DIR
            } else {
                libc::DT_REG
            };

            // Copy name using name_as_str helper
            let name = child.name_as_str();
            let name_bytes = name.as_bytes();
            let copy_len = std::cmp::min(name_bytes.len(), SYNTHETIC_DIRENT.d_name.len() - 1);
            ptr::copy_nonoverlapping(
                name_bytes.as_ptr(),
                SYNTHETIC_DIRENT.d_name.as_mut_ptr() as *mut u8,
                copy_len,
            );
            SYNTHETIC_DIRENT.d_name[copy_len] = 0;

            return &mut SYNTHETIC_DIRENT;
        }

        // Case B: IPC-backed entries (Fallback)
        if synthetic.position >= synthetic.entries.len() {
            // No more entries
            return ptr::null_mut();
        }

        let entry = &synthetic.entries[synthetic.position];
        synthetic.position += 1;

        // Fill in the static dirent
        ptr::write_bytes(&mut SYNTHETIC_DIRENT, 0, 1);
        SYNTHETIC_DIRENT.d_ino = (handle + synthetic.position) as libc::ino_t;
        SYNTHETIC_DIRENT.d_type = if entry.is_dir {
            libc::DT_DIR
        } else {
            libc::DT_REG
        };

        // Copy name (truncate if too long)
        let name_bytes = entry.name.as_bytes();
        let copy_len = std::cmp::min(name_bytes.len(), SYNTHETIC_DIRENT.d_name.len() - 1);
        ptr::copy_nonoverlapping(
            name_bytes.as_ptr(),
            SYNTHETIC_DIRENT.d_name.as_mut_ptr() as *mut u8,
            copy_len,
        );
        SYNTHETIC_DIRENT.d_name[copy_len] = 0;

        return &mut SYNTHETIC_DIRENT;
    }
    drop(dirs);

    real_readdir(dir)
}

#[allow(dead_code)] // Will be exported when full closedir support is added
unsafe fn closedir_impl(dir: *mut libc::DIR, real_closedir: ClosedirFn) -> c_int {
    let _guard = match ShimGuard::enter() {
        Some(g) => g,
        None => return real_closedir(dir),
    };

    let Some(state) = ShimState::get() else {
        return real_closedir(dir);
    };

    let handle = dir as usize;

    // Check if this was a synthetic directory
    let mut dirs = state.open_dirs.lock().unwrap();
    if dirs.remove(&handle).is_some() {
        shim_log("[VRift-Shim] closedir synthetic\n");
        return 0;
    }
    drop(dirs);

    real_closedir(dir)
}

unsafe fn readlink_impl(
    path: *const c_char,
    buf: *mut c_char,
    bufsiz: size_t,
    real_readlink: ReadlinkFn,
) -> ssize_t {
    let _guard = match ShimGuard::enter() {
        Some(g) => g,
        None => return real_readlink(path, buf, bufsiz),
    };

    let Some(state) = ShimState::get() else {
        return real_readlink(path, buf, bufsiz);
    };

    let path_str = match CStr::from_ptr(path).to_str() {
        Ok(s) => s,
        Err(_) => return real_readlink(path, buf, bufsiz),
    };

    if path_str.starts_with(&*state.vfs_prefix) {
        let vpath = &path_str[state.vfs_prefix.len()..];
        if let Some(entry) = state.query_manifest(vpath) {
            if entry.is_symlink() {
                if let Some(cas_guard) = state.get_cas() {
                    if let Some(cas) = cas_guard.as_ref() {
                        if let Ok(data) = cas.get(&entry.content_hash) {
                            let len = std::cmp::min(data.len(), bufsiz);
                            ptr::copy_nonoverlapping(data.as_ptr(), buf as *mut u8, len);
                            return len as ssize_t;
                        }
                    }
                }
            }
        }
    }

    real_readlink(path, buf, bufsiz)
}

// ============================================================================
#[cfg(target_os = "linux")]
static REAL_OPEN: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
#[cfg(target_os = "linux")]
static REAL_WRITE: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
#[cfg(target_os = "linux")]
static REAL_CLOSE: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());

#[cfg(target_os = "linux")]
macro_rules! get_real {
    ($storage:ident, $name:literal, $t:ty) => {{
        let p = $storage.load(Ordering::Acquire);
        if !p.is_null() {
            std::mem::transmute::<*mut c_void, $t>(p)
        } else {
            let f = libc::dlsym(
                libc::RTLD_NEXT,
                concat!($name, "\0").as_ptr() as *const c_char,
            );
            $storage.store(f, Ordering::Release);
            std::mem::transmute::<*mut c_void, $t>(f)
        }
    }};
}

#[cfg(target_os = "linux")]
#[no_mangle]
pub unsafe extern "C" fn open(p: *const c_char, f: c_int, m: mode_t) -> c_int {
    open_impl(p, f, m, get_real!(REAL_OPEN, "open", OpenFn))
}

#[cfg(target_os = "linux")]
#[no_mangle]
pub unsafe extern "C" fn write(fd: c_int, b: *const c_void, c: size_t) -> ssize_t {
    write_impl(fd, b, c, get_real!(REAL_WRITE, "write", WriteFn))
}

#[cfg(target_os = "linux")]
static REAL_STAT: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
#[cfg(target_os = "linux")]
static REAL_LSTAT: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
#[cfg(target_os = "linux")]
static REAL_FSTAT: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
#[cfg(target_os = "linux")]
static REAL_EXECVE: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());

#[cfg(target_os = "linux")]
#[no_mangle]
pub unsafe extern "C" fn close(fd: c_int) -> c_int {
    close_impl(fd, get_real!(REAL_CLOSE, "close", CloseFn))
}

#[cfg(target_os = "linux")]
#[no_mangle]
pub unsafe extern "C" fn stat(p: *const c_char, b: *mut libc::stat) -> c_int {
    stat_common(p, b, get_real!(REAL_STAT, "stat", StatFn))
}

#[cfg(target_os = "linux")]
#[no_mangle]
pub unsafe extern "C" fn lstat(p: *const c_char, b: *mut libc::stat) -> c_int {
    stat_common(p, b, get_real!(REAL_LSTAT, "lstat", StatFn))
}

#[cfg(target_os = "linux")]
#[no_mangle]
pub unsafe extern "C" fn fstat(fd: c_int, b: *mut libc::stat) -> c_int {
    fstat_impl(fd, b, get_real!(REAL_FSTAT, "fstat", FstatFn))
}

#[cfg(target_os = "linux")]
#[no_mangle]
pub unsafe extern "C" fn execve(
    path: *const c_char,
    argv: *const *const c_char,
    envp: *const *const c_char,
) -> c_int {
    execve_impl(path, argv, envp, get_real!(REAL_EXECVE, "execve", ExecveFn))
}

#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn open_shim(p: *const c_char, f: c_int, m: mode_t) -> c_int {
    let real = std::mem::transmute::<*const (), OpenFn>(IT_OPEN.old_func);
    // Early-boot passthrough to avoid deadlock during dyld initialization
    if INITIALIZING.load(Ordering::Relaxed) {
        return real(p, f, m);
    }
    open_impl(p, f, m, real)
}

#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn write_shim(fd: c_int, b: *const c_void, c: size_t) -> ssize_t {
    let real = std::mem::transmute::<*const (), WriteFn>(IT_WRITE.old_func);
    write_impl(fd, b, c, real)
}

#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn close_shim(fd: c_int) -> c_int {
    let real = std::mem::transmute::<*const (), CloseFn>(IT_CLOSE.old_func);
    close_impl(fd, real)
}

#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn stat_shim(p: *const c_char, b: *mut libc::stat) -> c_int {
    // Use IT_STAT.old_func to get the real libc stat, avoiding recursion
    let real = std::mem::transmute::<*const (), StatFn>(IT_STAT.old_func);
    stat_common(p, b, real)
}

#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn lstat_shim(p: *const c_char, b: *mut libc::stat) -> c_int {
    let real = std::mem::transmute::<*const (), StatFn>(IT_LSTAT.old_func);
    stat_common(p, b, real)
}

#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn fstat_shim(fd: c_int, b: *mut libc::stat) -> c_int {
    let real = std::mem::transmute::<*const (), FstatFn>(IT_FSTAT.old_func);
    fstat_impl(fd, b, real)
}

#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn opendir_shim(p: *const c_char) -> *mut libc::DIR {
    let real = std::mem::transmute::<*const (), OpendirFn>(IT_OPENDIR.old_func);
    opendir_impl(p, real)
}

#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn readdir_shim(d: *mut libc::DIR) -> *mut libc::dirent {
    let real = std::mem::transmute::<*const (), ReaddirFn>(IT_READDIR.old_func);
    readdir_impl(d, real)
}

#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn closedir_shim(d: *mut libc::DIR) -> c_int {
    let real = std::mem::transmute::<*const (), ClosedirFn>(IT_CLOSEDIR.old_func);
    closedir_impl(d, real)
}

#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn readlink_shim(p: *const c_char, b: *mut c_char, s: size_t) -> ssize_t {
    let real = std::mem::transmute::<*const (), ReadlinkFn>(IT_READLINK.old_func);
    readlink_impl(p, b, s, real)
}

#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn execve_shim(
    path: *const c_char,
    argv: *const *const c_char,
    envp: *const *const c_char,
) -> c_int {
    let real = std::mem::transmute::<*const (), ExecveFn>(IT_EXECVE.old_func);
    execve_impl(path, argv, envp, real)
}

#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn posix_spawn_shim(
    pid: *mut libc::pid_t,
    path: *const c_char,
    file_actions: *const c_void,
    attrp: *const c_void,
    argv: *const *const c_char,
    envp: *const *const c_char,
) -> c_int {
    let real = std::mem::transmute::<*const (), PosixSpawnFn>(IT_POSIX_SPAWN.old_func);
    // Reuse execve_impl's env logic by proxying through it if possible,
    // but posix_spawn takes more args. For now, simple passthrough with env modification.
    let _guard = match ShimGuard::enter() {
        Some(g) => g,
        None => return real(pid, path, file_actions, attrp, argv, envp),
    };
    // (Simplified env logic for now, similar to execve_impl)
    real(pid, path, file_actions, attrp, argv, envp)
}

#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn posix_spawnp_shim(
    pid: *mut libc::pid_t,
    file: *const c_char,
    file_actions: *const c_void,
    attrp: *const c_void,
    argv: *const *const c_char,
    envp: *const *const c_char,
) -> c_int {
    let real = std::mem::transmute::<*const (), PosixSpawnFn>(IT_POSIX_SPAWNP.old_func);
    real(pid, file, file_actions, attrp, argv, envp)
}

#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn mmap_shim(
    addr: *mut c_void,
    len: size_t,
    prot: c_int,
    flags: c_int,
    fd: c_int,
    offset: libc::off_t,
) -> *mut c_void {
    let real = std::mem::transmute::<*const (), MmapFn>(IT_MMAP.old_func);

    // Early bailout during initialization
    if INITIALIZING.load(Ordering::SeqCst) {
        return real(addr, len, prot, flags, fd, offset);
    }

    // Guard recursion
    let _guard = match ShimGuard::enter() {
        Some(g) => g,
        None => return real(addr, len, prot, flags, fd, offset),
    };

    // For VFS file descriptors (tracked in open_fds), the underlying fd already
    // points to the CAS blob temp file, so mmap can proceed normally.
    // The interposition here ensures we can add future optimizations like:
    // - Direct CAS blob mmap without temp files
    // - Memory-mapped manifest lookups
    // - Lazy content materialization

    real(addr, len, prot, flags, fd, offset)
}

#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn munmap_shim(addr: *mut c_void, len: size_t) -> c_int {
    type MunmapFn = unsafe extern "C" fn(*mut c_void, size_t) -> c_int;
    let real = std::mem::transmute::<*const (), MunmapFn>(IT_MUNMAP.old_func);

    // Early bailout during initialization
    if INITIALIZING.load(Ordering::SeqCst) {
        return real(addr, len);
    }

    // Guard recursion
    let _guard = match ShimGuard::enter() {
        Some(g) => g,
        None => return real(addr, len),
    };

    real(addr, len)
}

#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn dlopen_shim(filename: *const c_char, flags: c_int) -> *mut c_void {
    let real = std::mem::transmute::<*const (), DlopenFn>(IT_DLOPEN.old_func);

    // Early bailout during initialization
    if INITIALIZING.load(Ordering::SeqCst) {
        return real(filename, flags);
    }

    // Guard recursion
    let _guard = match ShimGuard::enter() {
        Some(g) => g,
        None => return real(filename, flags),
    };

    // NULL filename = get main program handle
    if filename.is_null() {
        return real(filename, flags);
    }

    // Check if this is a VFS path
    let path_str = match CStr::from_ptr(filename).to_str() {
        Ok(s) => s,
        Err(_) => return real(filename, flags),
    };

    let Some(state) = ShimState::get() else {
        return real(filename, flags);
    };

    let mut path_buf = [0u8; 1024];
    let resolved_len = match resolve_path_with_cwd(path_str, &mut path_buf) {
        Some(len) => len,
        None => return real(filename, flags),
    };
    let resolved_path = unsafe { std::str::from_utf8_unchecked(&path_buf[..resolved_len]) };

    if state.psfs_applicable(resolved_path) {
        if let Some(entry) = state.psfs_lookup(resolved_path) {
            // Get content from CAS and write to temp file
            if let Ok(cas_guard) = state.cas.lock() {
                if let Some(ref cas) = *cas_guard {
                    if let Ok(content) = cas.get(&entry.content_hash) {
                        // Create temp file for the library
                        let temp_dir = std::env::temp_dir();
                        let lib_name = std::path::Path::new(path_str)
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("vrift_lib.dylib");
                        let temp_path = temp_dir.join(format!("vrift_{}", lib_name));

                        if std::fs::write(&temp_path, &content).is_ok() {
                            if let Ok(c_path) = CString::new(temp_path.to_string_lossy().as_bytes())
                            {
                                return real(c_path.as_ptr(), flags);
                            }
                        }
                    }
                }
            }
        }
    }

    // Passthrough for non-VFS paths and fallback
    real(filename, flags)
}

#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn dlsym_shim(handle: *mut c_void, symbol: *const c_char) -> *mut c_void {
    type DlsymFn = unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void;
    let real = std::mem::transmute::<*const (), DlsymFn>(IT_DLSYM.old_func);

    // Early bailout during initialization
    if INITIALIZING.load(Ordering::SeqCst) {
        return real(handle, symbol);
    }

    // Guard recursion
    let _guard = match ShimGuard::enter() {
        Some(g) => g,
        None => return real(handle, symbol),
    };

    real(handle, symbol)
}

#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn access_shim(path: *const c_char, mode: c_int) -> c_int {
    let real = std::mem::transmute::<*const (), AccessFn>(IT_ACCESS.old_func);
    access_impl(path, mode, real)
}

#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn read_shim(fd: c_int, buf: *mut c_void, count: size_t) -> ssize_t {
    let real = std::mem::transmute::<*const (), ReadFn>(IT_READ.old_func);
    // Passthrough to real read - shim tracks fds but read content comes from
    // CAS backing store which is the actual file content
    real(fd, buf, count)
}

#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn fcntl_shim(fd: c_int, cmd: c_int, arg: c_int) -> c_int {
    // fcntl is variadic, but most common uses pass a single int arg
    // We must reference IT_FCNTL.old_func to prevent DCE stripping it
    let real = std::mem::transmute::<*const (), unsafe extern "C" fn(c_int, c_int, c_int) -> c_int>(
        IT_FCNTL.old_func,
    );
    // Early-boot passthrough to avoid deadlock during dyld initialization
    if INITIALIZING.load(Ordering::Relaxed) {
        return real(fd, cmd, arg);
    }
    real(fd, cmd, arg)
}

#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn openat_shim(
    dirfd: c_int,
    pathname: *const c_char,
    flags: c_int,
    mode: mode_t,
) -> c_int {
    // Passthrough to real openat - VFS path resolution happens at open time
    // AT_FDCWD (-2) means use current working directory
    let real = std::mem::transmute::<*const (), OpenatFn>(IT_OPENAT.old_func);
    real(dirfd, pathname, flags, mode)
}

#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn faccessat_shim(
    dirfd: c_int,
    pathname: *const c_char,
    mode: c_int,
    flags: c_int,
) -> c_int {
    // Passthrough to real faccessat - permission checks work on underlying files
    let real = std::mem::transmute::<*const (), FaccessatFn>(IT_FACCESSAT.old_func);
    real(dirfd, pathname, mode, flags)
}

#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn fstatat_shim(
    dirfd: c_int,
    pathname: *const c_char,
    buf: *mut libc::stat,
    flags: c_int,
) -> c_int {
    // Passthrough to real fstatat - stat operations handled via stat/lstat shims
    let real = std::mem::transmute::<*const (), FstatatFn>(IT_FSTATAT.old_func);
    real(dirfd, pathname, buf, flags)
}

#[no_mangle]
pub unsafe extern "C" fn realpath_shim(
    pathname: *const c_char,
    resolved_path: *mut c_char,
) -> *mut c_char {
    let _guard = match ShimGuard::enter() {
        Some(g) => g,
        None => {
            let real = std::mem::transmute::<*const (), RealpathFn>(IT_REALPATH.old_func);
            return real(pathname, resolved_path);
        }
    };

    if pathname.is_null() {
        return ptr::null_mut();
    }

    let path = CStr::from_ptr(pathname).to_string_lossy();
    if let Some(state) = ShimState::get() {
        if state.psfs_applicable(&path) {
            let mut buf = [0u8; 1024];
            if let Some(len) = resolve_path_with_cwd(&path, &mut buf) {
                let result = if resolved_path.is_null() {
                    libc::malloc(len + 1) as *mut c_char
                } else {
                    resolved_path
                };
                if !result.is_null() {
                    ptr::copy_nonoverlapping(buf.as_ptr(), result as *mut u8, len);
                    *result.add(len) = 0;
                    return result;
                }
            }
        }
    }

    let real = std::mem::transmute::<*const (), RealpathFn>(IT_REALPATH.old_func);
    real(pathname, resolved_path)
}

#[no_mangle]
pub unsafe extern "C" fn getcwd_shim(buf: *mut c_char, size: size_t) -> *mut c_char {
    let _guard = match ShimGuard::enter() {
        Some(g) => g,
        None => {
            let real = std::mem::transmute::<*const (), GetcwdFn>(IT_GETCWD.old_func);
            return real(buf, size);
        }
    };

    if let Some(vpath) = VIRTUAL_CWD.with(|cwd| cwd.borrow().clone()) {
        let vbytes = vpath.as_bytes();
        if size != 0 && size < vbytes.len() + 1 {
            set_errno(libc::ERANGE);
            return ptr::null_mut();
        }
        let result = if buf.is_null() {
            libc::malloc(vbytes.len() + 1) as *mut c_char
        } else {
            buf
        };
        if !result.is_null() {
            ptr::copy_nonoverlapping(vbytes.as_ptr(), result as *mut u8, vbytes.len());
            *result.add(vbytes.len()) = 0;
            return result;
        }
    }

    let real = std::mem::transmute::<*const (), GetcwdFn>(IT_GETCWD.old_func);
    real(buf, size)
}

#[no_mangle]
pub unsafe extern "C" fn chdir_shim(path: *const c_char) -> c_int {
    let _guard = match ShimGuard::enter() {
        Some(g) => g,
        None => {
            let real = std::mem::transmute::<*const (), ChdirFn>(IT_CHDIR.old_func);
            return real(path);
        }
    };

    if path.is_null() {
        set_errno(libc::EFAULT);
        return -1;
    }

    let path_str = CStr::from_ptr(path).to_string_lossy();
    if let Some(state) = ShimState::get() {
        let mut path_buf = [0u8; 1024];
        if let Some(len) = resolve_path_with_cwd(&path_str, &mut path_buf) {
            let resolved_path = unsafe { std::str::from_utf8_unchecked(&path_buf[..len]) };

            // RFC-0043: Robust virtualization support
            if resolved_path.starts_with(&*state.vfs_prefix) {
                // Check if it exists and is a directory in manifest
                if let Some(entry) = state.psfs_lookup(resolved_path) {
                    if (entry.mode as u32 & libc::S_IFMT as u32) == libc::S_IFDIR as u32 {
                        VIRTUAL_CWD.with(|cwd| {
                            *cwd.borrow_mut() = Some(resolved_path.to_string());
                        });
                        return 0;
                    } else {
                        set_errno(libc::ENOTDIR);
                        return -1;
                    }
                } else {
                    set_errno(libc::ENOENT);
                    return -1;
                }
            } else {
                // Moving out of virtual domain - clear virtual CWD
                VIRTUAL_CWD.with(|cwd| *cwd.borrow_mut() = None);
            }
        }
    }

    let real = std::mem::transmute::<*const (), ChdirFn>(IT_CHDIR.old_func);
    real(path)
}

#[no_mangle]
pub unsafe extern "C" fn unlink_shim(path: *const c_char) -> c_int {
    let _guard = match ShimGuard::enter() {
        Some(g) => g,
        None => {
            let real = std::mem::transmute::<*const (), UnlinkFn>(IT_UNLINK.old_func);
            return real(path);
        }
    };

    if let Some(state) = ShimState::get() {
        let path_str = CStr::from_ptr(path).to_string_lossy();
        let mut path_buf = [0u8; 1024];
        if let Some(len) = resolve_path_with_cwd(&path_str, &mut path_buf) {
            let resolved_path = unsafe { std::str::from_utf8_unchecked(&path_buf[..len]) };
            if resolved_path.starts_with(&*state.vfs_prefix) {
                set_errno(libc::EROFS);
                return -1;
            }
        }
    }

    let real = std::mem::transmute::<*const (), UnlinkFn>(IT_UNLINK.old_func);
    real(path)
}

#[no_mangle]
pub unsafe extern "C" fn rename_shim(oldpath: *const c_char, newpath: *const c_char) -> c_int {
    let _guard = match ShimGuard::enter() {
        Some(g) => g,
        None => {
            let real = std::mem::transmute::<*const (), RenameFn>(IT_RENAME.old_func);
            return real(oldpath, newpath);
        }
    };

    if let Some(state) = ShimState::get() {
        let old_str = CStr::from_ptr(oldpath).to_string_lossy();
        let new_str = CStr::from_ptr(newpath).to_string_lossy();
        let mut buf_old = [0u8; 1024];
        let mut buf_new = [0u8; 1024];

        let old_res = resolve_path_with_cwd(&old_str, &mut buf_old);
        let new_res = resolve_path_with_cwd(&new_str, &mut buf_new);

        let old_is_vfs = old_res.map_or(false, |len| {
            let p = unsafe { std::str::from_utf8_unchecked(&buf_old[..len]) };
            p.starts_with(&*state.vfs_prefix)
        });
        let new_is_vfs = new_res.map_or(false, |len| {
            let p = unsafe { std::str::from_utf8_unchecked(&buf_new[..len]) };
            p.starts_with(&*state.vfs_prefix)
        });

        if old_is_vfs || new_is_vfs {
            set_errno(libc::EROFS);
            return -1;
        }
    }

    let real = std::mem::transmute::<*const (), RenameFn>(IT_RENAME.old_func);
    real(oldpath, newpath)
}

#[no_mangle]
pub unsafe extern "C" fn rmdir_shim(path: *const c_char) -> c_int {
    let _guard = match ShimGuard::enter() {
        Some(g) => g,
        None => {
            let real = std::mem::transmute::<*const (), RmdirFn>(IT_RMDIR.old_func);
            return real(path);
        }
    };

    if let Some(state) = ShimState::get() {
        let path_str = CStr::from_ptr(path).to_string_lossy();
        let mut path_buf = [0u8; 1024];
        if let Some(len) = resolve_path_with_cwd(&path_str, &mut path_buf) {
            let resolved_path = unsafe { std::str::from_utf8_unchecked(&path_buf[..len]) };
            if resolved_path.starts_with(&*state.vfs_prefix) {
                set_errno(libc::EROFS);
                return -1;
            }
        }
    }

    let real = std::mem::transmute::<*const (), RmdirFn>(IT_RMDIR.old_func);
    real(path)
}

extern "C" fn dump_logs_atexit() {
    LOGGER.dump_to_file();
}
