use crate::path::{PathResolver, VfsPath};
use libc::{c_int, c_void};
use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::path::Path;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU8, AtomicUsize, Ordering};
use std::sync::Mutex;

use crate::ipc::*;
// use vrift_cas::CasStore;
// use vrift_ipc;

// ============================================================================
// Global State & Recursion Guards
// ============================================================================
//
// ⚠️ TLS SAFETY CRITICAL SECTION (Pattern 2648/2649)
//
// This module manages initialization state during the hazardous dyld bootstrap
// phase. The following invariants MUST be maintained:
//
// 1. INITIALIZING starts at 1 (passthrough mode) - set to 0 only after dyld
//    completes loading all symbols (via SET_READY in lib.rs)
//
// 2. TLS_READY starts at false - set to true only after pthread TLS is
//    confirmed working (also in SET_READY)
//
// 3. All shim entry points MUST check these flags BEFORE using any Rust
//    features that might trigger TLS (String, HashMap, Mutex, etc.)
//
// 4. ShimState::init() uses ONLY libc primitives (malloc, memcpy) to avoid
//    touching Rust allocator which may trigger TLS
//
// Violation of these invariants will cause process deadlock on macOS ARM64.
// See docs/SHIM_SAFETY_GUIDE.md for details.
// ============================================================================

pub(crate) static SHIM_STATE: AtomicPtr<ShimState> = AtomicPtr::new(ptr::null_mut());
// Flag to indicate shim is still initializing. All syscalls passthrough during this phase.
extern "C" {
    pub static INITIALIZING: std::sync::atomic::AtomicU8;
}

/// Flag to prevent recursion during TLS key creation (bootstrap phase)
pub(crate) static BOOTSTRAPPING: AtomicBool = AtomicBool::new(false);
pub(crate) static DEBUG_ENABLED: AtomicBool = AtomicBool::new(false);

/// VFS activation flag - starts 0 (FALSE), becomes 1 (TRUE) when daemon connection is established.
/// Until VFS_READY is true, all open/openat calls passthrough to kernel directly.
/// This enables "zero config" UX: boot fast, activate VFS seamlessly when ready.
/// Exported with no_mangle so C wrapper can check it directly without FFI call.
#[no_mangle]
pub static VFS_READY: AtomicU8 = AtomicU8::new(0);

// ============================================================================
// Granular Logging & Circuit Breaker (RFC-0050)
// ============================================================================

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    Trace = 0,
    Debug = 1,
    Info = 2,
    Warn = 3,
    Error = 4,
    Off = 5,
}

// ============================================================================
// Flight Recorder (RFC-0039 §82)
// ============================================================================

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventType {
    OpenHit = 1,
    OpenMiss = 2,
    StatHit = 3,
    StatMiss = 4,
    CowTriggered = 5,
    IpcFail = 6,
    IpcSuccess = 7,
    CircuitTripped = 8,
    VfsInit = 9,
    Close = 10,
    ReingestSuccess = 11,
    ReingestFail = 12,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct FlightEntry {
    pub timestamp: u64,
    pub event_type: u8,
    pub _pad: [u8; 7], // Alignment for u64
    pub file_id: u64,  // 64-bit hash (FNV1a)
    pub result: i32,
    pub _pad2: [u8; 4], // Pad to 32 bytes
}

pub const FLIGHT_RECORDER_SIZE: usize = 32768; // 32K entries * 32 bytes = 1MB
pub struct FlightRecorder {
    pub buffer: [FlightEntry; FLIGHT_RECORDER_SIZE],
    pub head: AtomicUsize,
}

impl FlightRecorder {
    pub const fn new() -> Self {
        Self {
            buffer: [FlightEntry {
                timestamp: 0,
                event_type: 0,
                _pad: [0; 7],
                file_id: 0,
                result: 0,
                _pad2: [0; 4],
            }; FLIGHT_RECORDER_SIZE],
            head: AtomicUsize::new(0),
        }
    }
}

impl Default for FlightRecorder {
    fn default() -> Self {
        Self::new()
    }
}

impl FlightRecorder {
    #[inline(always)]
    pub fn record(&self, event: EventType, file_id: u64, result: i32) {
        let idx = self.head.fetch_add(1, Ordering::Relaxed) % FLIGHT_RECORDER_SIZE;
        // Safety: We are writing to a pre-allocated buffer.
        // In a high-concurrency dylib, we accept that entries might be semi-corrupted
        // if two threads write to the same index exactly at the same time,
        // but this is extremely rare and better than locking.
        unsafe {
            let entry = &mut *self.buffer.as_ptr().add(idx).cast_mut();
            entry.timestamp = rdtsc();
            entry.event_type = event as u8;
            entry.file_id = file_id;
            entry.result = result;
        }
    }
}

pub static FLIGHT_RECORDER: FlightRecorder = FlightRecorder::new();

pub static EVENT_NAMES: &[&str] = &[
    "UNKNOWN",
    "OpenHit",
    "OpenMiss",
    "StatHit",
    "StatMiss",
    "CowTriggered",
    "IpcFail",
    "IpcSuccess",
    "CircuitTripped",
    "VfsInit",
    "Close",
    "ReingestSuccess",
    "ReingestFail",
];

#[inline(always)]
fn rdtsc() -> u64 {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        std::arch::x86_64::_rdtsc()
    }
    #[cfg(target_arch = "aarch64")]
    unsafe {
        let mut cntpct: u64;
        std::arch::asm!("mrs {0}, cntpct_el0", out(reg) cntpct);
        cntpct
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    {
        0
    }
}

impl LogLevel {
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => LogLevel::Trace,
            1 => LogLevel::Debug,
            2 => LogLevel::Info,
            3 => LogLevel::Warn,
            4 => LogLevel::Error,
            _ => LogLevel::Off,
        }
    }
}

pub static LOG_LEVEL: AtomicU8 = AtomicU8::new(LogLevel::Info as u8);

/// Circuit breaker state: trips after consecutive failures
pub static CIRCUIT_BREAKER_FAILED_COUNT: AtomicUsize = AtomicUsize::new(0);
pub static CIRCUIT_BREAKER_THRESHOLD: AtomicUsize = AtomicUsize::new(5);
pub static CIRCUIT_TRIPPED: AtomicBool = AtomicBool::new(false);

/// Activate VFS - called when daemon handshake succeeds
#[inline]
pub fn activate_vfs() {
    VFS_READY.store(1, Ordering::Release);
}

/// Check if VFS is ready for use
#[inline]
pub fn is_vfs_ready() -> bool {
    VFS_READY.load(Ordering::Acquire) != 0
}

// Lock-free recursion key using atomic instead of OnceLock (avoids mutex deadlock during library init)
static RECURSION_KEY_INIT: AtomicBool = AtomicBool::new(false);
static RECURSION_KEY_VALUE: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn get_recursion_key() -> libc::pthread_key_t {
    // Fast path: already initialized
    if RECURSION_KEY_INIT.load(Ordering::Acquire) {
        return RECURSION_KEY_VALUE.load(Ordering::Relaxed) as libc::pthread_key_t;
    }

    // Slow path: initialize (only one thread will succeed)
    // RFC-0049: Use BOOTSTRAPPING flag to prevent recursion if pthread_key_create
    // or its internal calls are intercepted.
    if BOOTSTRAPPING.swap(true, Ordering::SeqCst) {
        return 0; // Already bootstrapping, avoid recursion
    }

    let mut key: libc::pthread_key_t = 0;
    let ret = unsafe { libc::pthread_key_create(&mut key, None) };
    if ret != 0 {
        BOOTSTRAPPING.store(false, Ordering::SeqCst);
        return 0;
    }

    // Try to be the one to set the value (CAS)
    let expected = 0usize;
    if RECURSION_KEY_VALUE
        .compare_exchange(expected, key as usize, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
    {
        RECURSION_KEY_INIT.store(true, Ordering::Release);
        BOOTSTRAPPING.store(false, Ordering::SeqCst);
        key
    } else {
        // Another thread beat us, clean up and use their key
        unsafe { libc::pthread_key_delete(key) };
        BOOTSTRAPPING.store(false, Ordering::SeqCst);
        RECURSION_KEY_VALUE.load(Ordering::Relaxed) as libc::pthread_key_t
    }
}

pub(crate) struct ShimGuard(bool); // bool: true = has active TLS guard
impl ShimGuard {
    pub(crate) fn enter() -> Option<Self> {
        if (unsafe { INITIALIZING.load(Ordering::Relaxed) }) != 0
            || BOOTSTRAPPING.load(Ordering::Relaxed)
        {
            return None;
        }

        // RFC-0049: Lazy TLS initialization.
        // If SHIM_STATE is null, we are in the middle of (or about to start) initialization.
        // During this phase, we don't use the TLS recursion guard yet. We rely on the
        // INITIALIZING flag which is set by ShimState::get() to prevent recursion.
        // This avoids calling pthread_key_create() too early during dyld's initialization.
        if SHIM_STATE.load(Ordering::Acquire).is_null() {
            return Some(ShimGuard(false));
        }

        // Set BOOTSTRAPPING true while accessing TLS
        if BOOTSTRAPPING.swap(true, Ordering::SeqCst) {
            return None;
        }

        let res = (|| {
            let key = get_recursion_key();
            if key == 0 {
                // TLS key creation failed - allow proceed without recursion guard
                // This is safe because at this point SHIM_STATE is initialized
                return Some(ShimGuard(false));
            }
            let val = unsafe { libc::pthread_getspecific(key) };
            if !val.is_null() {
                None // Already in shim - recursion detected
            } else {
                unsafe { libc::pthread_setspecific(key, std::ptr::dangling::<c_void>()) };
                Some(ShimGuard(true))
            }
        })();

        BOOTSTRAPPING.store(false, Ordering::SeqCst);
        res
    }
}
impl Drop for ShimGuard {
    fn drop(&mut self) {
        if self.0 {
            let key = get_recursion_key();
            if key != 0 {
                unsafe { libc::pthread_setspecific(key, ptr::null()) };
            }
        }
    }
}

pub(crate) const LOG_BUF_SIZE: usize = 64 * 1024;
pub struct Logger {
    buffer: [u8; LOG_BUF_SIZE],
    pub(crate) head: std::sync::atomic::AtomicUsize,
}

impl Default for Logger {
    fn default() -> Self {
        Self::new()
    }
}

impl Logger {
    pub const fn new() -> Self {
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

pub static LOGGER: Logger = Logger::new();

pub(crate) unsafe fn shim_log(msg: &str) {
    LOGGER.log(msg);
    if DEBUG_ENABLED.load(Ordering::Relaxed) {
        libc::write(2, msg.as_ptr() as *const c_void, msg.len());
    }
}

pub fn vfs_dump_flight_recorder() {
    let head = FLIGHT_RECORDER.head.load(Ordering::Relaxed);
    let start = head.saturating_sub(FLIGHT_RECORDER_SIZE);

    // Use a fixed buffer to avoid allocations during dump
    let mut buf = [0u8; 256];

    let pid = unsafe { libc::getpid() };
    let header = format!("\n--- [VFS] Flight Recorder Dump (PID: {}) ---\n", pid);
    let _ = unsafe { libc::write(2, header.as_ptr() as *const c_void, header.len()) };

    for i in start..head {
        let entry = &FLIGHT_RECORDER.buffer[i % FLIGHT_RECORDER_SIZE];
        if entry.event_type == 0 {
            continue;
        }

        let event_name = EVENT_NAMES
            .get(entry.event_type as usize)
            .unwrap_or(&"INVALID");

        let mut wrapper = crate::macros::StackWriter::new(&mut buf);
        use std::fmt::Write;
        let _ = writeln!(
            wrapper,
            "[{:>15}] {:<16} ID:0x{:016x} RES:{}",
            entry.timestamp, event_name, entry.file_id, entry.result
        );
        let msg = wrapper.as_str();
        let _ = unsafe { libc::write(2, msg.as_ptr() as *const c_void, msg.len()) };
    }
    let footer = "--- End of Dump ---\n";
    let _ = unsafe { libc::write(2, footer.as_ptr() as *const c_void, footer.len()) };
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
    // pub mmap_children: Option<(*const vrift_ipc::MmapDirChild, usize)>, // mmap path: (start_ptr, count)
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
    // pub cas: std::sync::Mutex<Option<CasStore>>, // Lazy init to avoid fs calls during dylib load
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
    pub path_resolver: PathResolver,
}

impl ShimState {
    pub(crate) unsafe fn init_logger() {
        let debug_ptr = libc::getenv(c"VRIFT_DEBUG".as_ptr());
        if !debug_ptr.is_null() {
            DEBUG_ENABLED.store(true, Ordering::Relaxed);
        }

        // RFC-0050: Read log level
        let level_ptr = unsafe { libc::getenv(c"VRIFT_LOG_LEVEL".as_ptr()) };
        if !level_ptr.is_null() {
            let level_str = unsafe { CStr::from_ptr(level_ptr).to_string_lossy() };
            let level = match level_str.to_lowercase().as_str() {
                "trace" => LogLevel::Trace,
                "debug" => LogLevel::Debug,
                "info" => LogLevel::Info,
                "warn" => LogLevel::Warn,
                "error" => LogLevel::Error,
                "off" => LogLevel::Off,
                _ => LogLevel::Info,
            };
            LOG_LEVEL.store(level as u8, Ordering::Relaxed);
        }

        // RFC-0050: Read circuit breaker threshold
        let threshold_ptr = unsafe { libc::getenv(c"VRIFT_CIRCUIT_BREAKER_THRESHOLD".as_ptr()) };
        if !threshold_ptr.is_null() {
            if let Ok(threshold) = unsafe {
                CStr::from_ptr(threshold_ptr)
                    .to_string_lossy()
                    .parse::<usize>()
            } {
                CIRCUIT_BREAKER_THRESHOLD.store(threshold, Ordering::Relaxed);
            }
        }
    }

    pub(crate) fn init() -> Option<*mut Self> {
        unsafe { Self::init_logger() };
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

        let socket_ptr = unsafe { libc::getenv(c"VRIFT_SOCKET_PATH".as_ptr()) };
        let socket_path: std::borrow::Cow<'static, str> = if socket_ptr.is_null() {
            std::borrow::Cow::Borrowed("/tmp/vrift.sock")
        } else {
            std::borrow::Cow::Owned(unsafe {
                CStr::from_ptr(socket_ptr).to_string_lossy().into_owned()
            })
        };

        // NOTE: Bloom mmap is deferred - don't call during init
        let bloom_ptr = ptr::null();

        // Hot Stat Cache deferred - avoid syscalls during init
        let (mmap_ptr, mmap_size) = (ptr::null(), 0);

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
            // cas: std::sync::Mutex::new(None),
            cas_root,
            vfs_prefix: vfs_prefix.clone(),
            socket_path,
            open_fds: Mutex::new(HashMap::new()),
            active_mmaps: Mutex::new(HashMap::new()),
            open_dirs: Mutex::new(HashMap::new()),
            bloom_ptr,
            mmap_ptr,
            mmap_size,
            project_root: project_root.clone(),
            path_resolver: PathResolver::new(&vfs_prefix, &project_root),
        });

        Some(Box::into_raw(state))
    }

    pub(crate) fn get() -> Option<&'static Self> {
        let ptr = SHIM_STATE.load(Ordering::Acquire);
        if !ptr.is_null() {
            return unsafe { Some(&*ptr) };
        }
        // RFC-0050: Four-state initialization lifecycle
        // 2: Early-Init (Hazardous/Waiting), 1: Gate Open (Ready), 3: Busy (Initializing), 0: Done
        let current = unsafe { INITIALIZING.load(Ordering::Acquire) };
        if current == 3 {
            // 3 (Already init-in-progress), return None to fallback
            return None;
        }

        // Attempt to transition to 3 (Busy)
        // Try from 1 (Ready - C constructor ran), then from 2 (Early - C constructor didn't run yet)
        let transitioned = unsafe {
            INITIALIZING
                .compare_exchange(1, 3, Ordering::SeqCst, Ordering::Acquire)
                .is_ok()
                || INITIALIZING
                    .compare_exchange(2, 3, Ordering::SeqCst, Ordering::Acquire)
                    .is_ok()
        };
        if !transitioned {
            // Someone else is initializing or we are already Done (0) but raced with SHIM_STATE load
            return None;
        }

        // Initialize state - MUST reset INITIALIZING to 0 on success, or back to 1 on failure
        let ptr = match Self::init() {
            Some(p) => p,
            None => {
                unsafe { INITIALIZING.store(1, Ordering::SeqCst) };
                return None;
            }
        };
        SHIM_STATE.store(ptr, Ordering::Release);
        unsafe { INITIALIZING.store(0, Ordering::SeqCst) };

        // RFC-0039 §82: Record initialization event
        vfs_record!(EventType::VfsInit, 0, 0);
        // Setup signal handler for on-demand log dumping
        unsafe { setup_signal_handler() };
        // Register atexit hook for final log flush
        unsafe { libc::atexit(dump_logs_atexit) };

        // Activate VFS - now it's safe to call into Rust from C wrappers.
        activate_vfs();

        unsafe { Some(&*ptr) }
    }

    pub(crate) fn query_manifest(&self, path: &str) -> Option<vrift_ipc::VnodeEntry> {
        // Strip VFS prefix to get relative path for manifest lookup
        let rel_path = if path.starts_with(&*self.vfs_prefix) {
            let rel = &path[self.vfs_prefix.len()..];
            if rel.is_empty() {
                "/"
            } else if !rel.starts_with('/') {
                // Should not happen with normalized paths, but safety first
                return None;
            } else {
                rel
            }
        } else {
            path
        };

        // First try Hot Stat Cache (O(1) mmap lookup)
        if let Some(entry) = mmap_lookup(self.mmap_ptr, self.mmap_size, rel_path) {
            return Some(vrift_ipc::VnodeEntry {
                content_hash: [0u8; 32],
                size: entry.size,
                mtime: entry.mtime as u64,
                mode: entry.mode,
                flags: entry.flags as u16,
                _pad: 0,
            });
        }
        // Fall back to IPC query
        unsafe { sync_ipc_manifest_get(&self.socket_path, rel_path) }
    }

    /// Query manifest directly via IPC (bypasses mmap cache)
    /// Required for open() which needs content_hash to locate CAS blob
    pub(crate) fn query_manifest_ipc(&self, vpath: &VfsPath) -> Option<vrift_ipc::VnodeEntry> {
        // Use the centrally resolved manifest key
        unsafe { sync_ipc_manifest_get(&self.socket_path, &vpath.manifest_key) }
    }

    /// Resolve an incoming path into a VfsPath if it belongs to the VFS.
    pub(crate) fn resolve_path(&self, path: &str) -> Option<VfsPath> {
        self.path_resolver.resolve(path)
    }

    /// Check if path is in VFS domain (Backwards compatibility shim)
    pub(crate) fn psfs_applicable(&self, path: &str) -> bool {
        self.resolve_path(path).is_some()
    }

    /// Attempt O(1) stat lookup from manifest cache
    pub(crate) fn psfs_lookup(&self, _path: &str) -> Option<vrift_ipc::VnodeEntry> {
        None
    }
    #[allow(dead_code)]
    pub(crate) fn upsert_manifest(&self, _path: &str, _entry: ()) -> bool {
        false
    }

    /// RFC-0047: Remove entry from manifest (for unlink/rmdir)
    pub(crate) fn manifest_remove(&self, path: &str) -> Result<(), ()> {
        if unsafe { sync_ipc_manifest_remove(&self.socket_path, path) } {
            Ok(())
        } else {
            Err(())
        }
    }

    /// RFC-0047: Create directory entry in manifest
    #[allow(clippy::unnecessary_cast)] // mode_t is u16 on macOS, u32 on Linux
    pub(crate) fn manifest_mkdir(&self, path: &str, mode: libc::mode_t) -> Result<(), ()> {
        if unsafe { sync_ipc_manifest_mkdir(&self.socket_path, path, mode as u32) } {
            Ok(())
        } else {
            Err(())
        }
    }

    /// RFC-0047: Rename manifest entry
    pub(crate) fn manifest_rename(&self, old_path: &str, new_path: &str) -> Result<(), ()> {
        if unsafe { sync_ipc_manifest_rename(&self.socket_path, old_path, new_path) } {
            Ok(())
        } else {
            Err(())
        }
    }

    /// Query daemon for directory listing (for opendir/readdir)
    #[allow(dead_code)]
    pub(crate) fn query_dir_listing(&self, path: &str) -> Option<Vec<vrift_ipc::DirEntry>> {
        // First try mmap directory lookup
        if let Some((children_ptr, count)) = mmap_dir_lookup(self.mmap_ptr, self.mmap_size, path) {
            let mut entries = Vec::with_capacity(count);
            for i in 0..count {
                let child = unsafe { &*children_ptr.add(i) };
                entries.push(vrift_ipc::DirEntry {
                    name: child.name_as_str().to_string(),
                    is_dir: child.is_dir != 0,
                });
            }
            return Some(entries);
        }
        // Fall back to IPC
        unsafe { sync_ipc_manifest_list_dir(&self.socket_path, path) }
    }

    fn try_connect(&self) -> i32 {
        -1
    }

    fn try_register(&self) -> i32 {
        -1
    }

    /// Internal helper: connect, handshake, and register workspace.
    /// Returns fd or -1 on error.
    pub(crate) unsafe fn raw_connect_and_register(&self) -> c_int {
        -1
    }

    fn rpc(&self, request: &vrift_ipc::VeloRequest) -> Option<vrift_ipc::VeloResponse> {
        unsafe {
            let fd = raw_unix_connect(&self.socket_path);
            if fd < 0 {
                return None;
            }
            // Serialize and send
            let req_bytes = bincode::serialize(request).ok()?;
            let len_bytes = (req_bytes.len() as u32).to_le_bytes();
            if !raw_write_all(fd, &len_bytes) || !raw_write_all(fd, &req_bytes) {
                libc::close(fd);
                return None;
            }
            // Read response
            let mut resp_len_buf = [0u8; 4];
            if !raw_read_exact(fd, &mut resp_len_buf) {
                libc::close(fd);
                return None;
            }
            let resp_len = u32::from_le_bytes(resp_len_buf) as usize;
            let mut resp_buf = vec![0u8; resp_len];
            if !raw_read_exact(fd, &mut resp_buf) {
                libc::close(fd);
                return None;
            }
            libc::close(fd);
            bincode::deserialize(&resp_buf).ok()
        }
    }
}

extern "C" fn dump_logs_atexit() {
    LOGGER.dump_to_file();
    // Also dump flight recorder to stderr if debug is enabled
    if DEBUG_ENABLED.load(Ordering::Relaxed) {
        vfs_dump_flight_recorder();
    }
}

pub(crate) unsafe fn setup_signal_handler() {
    #[cfg(target_os = "macos")]
    {
        use libc::{signal, SIGUSR1};
        extern "C" fn handle_sigusr1(_sig: libc::c_int) {
            vfs_dump_flight_recorder();
        }
        signal(SIGUSR1, handle_sigusr1 as usize);
    }
}
