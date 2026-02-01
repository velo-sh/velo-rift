use libc::{c_int, c_void};
use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::path::Path;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicUsize, Ordering};
use std::sync::Mutex;

use crate::ipc::*;
use crate::path::*;
use vrift_cas::CasStore;
use vrift_ipc;

// ============================================================================
// Global State & Recursion Guards
// ============================================================================

pub(crate) static SHIM_STATE: AtomicPtr<ShimState> = AtomicPtr::new(ptr::null_mut());
/// Flag to indicate shim is still initializing. All syscalls passthrough during this phase.
pub(crate) static INITIALIZING: AtomicBool = AtomicBool::new(false);
pub(crate) static DEBUG_ENABLED: AtomicBool = AtomicBool::new(false);

// Lock-free recursion key using atomic instead of OnceLock (avoids mutex deadlock during library init)
static RECURSION_KEY_INIT: AtomicBool = AtomicBool::new(false);
static RECURSION_KEY_VALUE: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn get_recursion_key() -> libc::pthread_key_t {
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

pub(crate) struct ShimGuard;
impl ShimGuard {
    pub(crate) fn enter() -> Option<Self> {
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

pub(crate) const LOG_BUF_SIZE: usize = 64 * 1024;
pub(crate) struct Logger {
    buffer: [u8; LOG_BUF_SIZE],
    head: std::sync::atomic::AtomicUsize,
}

impl Logger {
    pub(crate) const fn new() -> Self {
        Self {
            buffer: [0u8; LOG_BUF_SIZE],
            head: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    pub(crate) fn log(&self, msg: &str) {
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
    pub(crate) fn dump(&self) {
        let head = self.head.load(Ordering::SeqCst);
        let start = head.saturating_sub(LOG_BUF_SIZE);
        for i in start..head {
            unsafe {
                let c = self.buffer[i % LOG_BUF_SIZE];
                libc::write(2, &c as *const u8 as *const c_void, 1);
            }
        }
    }

    #[allow(dead_code)]
    pub(crate) fn dump_to_file(&self) {
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

pub(crate) static LOGGER: Logger = Logger::new();

pub(crate) unsafe fn shim_log(msg: &str) {
    LOGGER.log(msg);
    if DEBUG_ENABLED.load(Ordering::Relaxed) {
        libc::write(2, msg.as_ptr() as *const c_void, msg.len());
    }
}

pub(crate) struct OpenFile {
    pub vpath: String,
    // Path to the temporary file backing this FD (for CoW)
    pub temp_path: String,
    // Number of active mmap mappings for this FD
    pub mmap_count: usize,
}

/// Track active mmap regions for VFS files
pub(crate) struct MmapInfo {
    pub vpath: String,
    pub temp_path: String,
    pub len: usize,
}

/// Synthetic directory for VFS opendir/readdir
#[allow(dead_code)]
pub(crate) struct SyntheticDir {
    pub vpath: String,
    pub entries: Vec<vrift_ipc::DirEntry>, // IPC fallback
    pub mmap_children: Option<(*const vrift_ipc::MmapDirChild, usize)>, // mmap path: (start_ptr, count)
    pub position: usize,
}
unsafe impl Send for SyntheticDir {} // Raw pointers in open_dirs HashMap
unsafe impl Sync for SyntheticDir {}

pub(crate) static SYNTHETIC_DIR_COUNTER: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

// ============================================================================
// RFC-0044 Hot Stat Cache: mmap-based O(1) Stat Lookup
// ============================================================================

/// Open mmap'd manifest file for O(1) stat lookup.
/// Returns (ptr, size) or (null, 0) if unavailable.
/// Uses raw libc to avoid recursion through shim.
pub(crate) fn open_manifest_mmap() -> (*const u8, usize) {
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

    let _root_str = project_root.to_string_lossy();
    let mmap_path_dir = project_root.join(".vrift");
    let mmap_path = mmap_path_dir.join("manifest.mmap");

    let mmap_path_cstr = CString::new(mmap_path.to_string_lossy().as_ref()).unwrap_or_default();

    let fd = unsafe { libc::open(mmap_path_cstr.as_ptr(), libc::O_RDONLY) };
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
pub(crate) fn mmap_lookup(
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
pub(crate) fn mmap_dir_lookup(
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

pub(crate) struct ShimState {
    pub cas: std::sync::Mutex<Option<CasStore>>, // Lazy init to avoid fs calls during dylib load
    pub cas_root: std::borrow::Cow<'static, str>,
    pub vfs_prefix: std::borrow::Cow<'static, str>,
    pub socket_path: std::borrow::Cow<'static, str>,
    pub open_fds: Mutex<HashMap<c_int, OpenFile>>,
    /// Active mmap regions (Addr -> Info)
    pub active_mmaps: Mutex<HashMap<usize, MmapInfo>>,
    /// Synthetic directories for VFS readdir (DIR* pointer -> SyntheticDir)
    pub open_dirs: Mutex<HashMap<usize, SyntheticDir>>,
    pub bloom_ptr: *const u8,
    /// RFC-0044 Hot Stat Cache: mmap'd manifest for O(1) stat lookup
    pub mmap_ptr: *const u8,
    pub mmap_size: usize,
    /// Absolute path to project root
    pub project_root: String,
}

impl ShimState {
    pub(crate) fn init() -> Option<*mut Self> {
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
            active_mmaps: Mutex::new(HashMap::new()),
            open_dirs: Mutex::new(HashMap::new()),
            bloom_ptr,
            mmap_ptr,
            mmap_size,
            project_root,
        });

        Some(Box::into_raw(state))
    }

    /// Get or create CasStore lazily (only called when actually needed)
    pub(crate) fn get_cas(&self) -> Option<std::sync::MutexGuard<'_, Option<CasStore>>> {
        let mut cas = self.cas.lock().ok()?;
        if cas.is_none() {
            match CasStore::new(self.cas_root.as_ref()) {
                Ok(c) => *cas = Some(c),
                Err(_) => return None,
            }
        }
        Some(cas)
    }

    pub(crate) fn get() -> Option<&'static Self> {
        let ptr = SHIM_STATE.load(Ordering::Acquire);
        if !ptr.is_null() {
            return unsafe { Some(&*ptr) };
        }

        if INITIALIZING.swap(true, Ordering::SeqCst) {
            return None;
        }

        // Initialize state
        let ptr = Self::init()?;
        SHIM_STATE.store(ptr, Ordering::Release);
        INITIALIZING.store(false, Ordering::SeqCst);

        unsafe { Some(&*ptr) }
    }

    pub(crate) fn query_manifest(&self, path: &str) -> Option<vrift_manifest::VnodeEntry> {
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

        let manifest_path = path;

        let ok = (|| -> Option<vrift_manifest::VnodeEntry> {
            // 3. Manifest Get
            let req = VeloRequest::ManifestGet {
                path: manifest_path.to_string(),
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

    /// Check if path is in VFS domain (zero-alloc, O(1) string prefix check)
    /// Returns true if path should be considered for Hot Stat acceleration
    #[inline(always)]
    pub(crate) fn psfs_applicable(&self, path: &str) -> bool {
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
    pub(crate) fn psfs_lookup(&self, path: &str) -> Option<vrift_manifest::VnodeEntry> {
        let mut buf = [0u8; 1024];
        if let Some(len) = unsafe { resolve_path_with_cwd(path, &mut buf) } {
            let normalized = unsafe { std::str::from_utf8_unchecked(&buf[..len]) };
            self.query_manifest(normalized)
        } else {
            self.query_manifest(path)
        }
    }
    #[allow(dead_code)] // Will be called from close_impl when async re-ingest is implemented
    pub(crate) fn upsert_manifest(&self, path: &str, entry: vrift_manifest::VnodeEntry) -> bool {
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
    #[allow(dead_code)]
    pub(crate) fn query_dir_listing(&self, path: &str) -> Option<Vec<vrift_ipc::DirEntry>> {
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
    pub(crate) unsafe fn raw_connect_and_register(&self) -> c_int {
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

extern "C" fn dump_logs_atexit() {
    LOGGER.dump_to_file();
}
