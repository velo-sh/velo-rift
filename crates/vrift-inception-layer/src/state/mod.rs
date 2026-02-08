// =============================================================================
// state/mod.rs — Hot-path code for inception layer state
// =============================================================================
//
// This module contains code that runs on EVERY interposed syscall:
//   - get() / get_no_spawn() — global state access (small stack frame!)
//   - query_manifest() / resolve_path() — per-call lookups
//   - InceptionLayerGuard — recursion prevention
//   - FlightRecorder / Logger / DirtyTracker — always-hot infrastructure
//
// Cold-path init code lives in state/init.rs (behind #[inline(never)])
// Background worker code lives in state/worker.rs
// =============================================================================

mod init;
mod worker;

use crate::ipc::*;
use crate::path::{PathResolver, VfsPath};
use crate::sync::RecursiveMutex;
use libc::{c_int, c_void};
use std::collections::HashMap;
use std::ffi::CStr;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, AtomicU8, AtomicUsize, Ordering};

// ============================================================================
// Global State & Recursion Guards
// ============================================================================
//
// ⚠️ TLS SAFETY CRITICAL SECTION (Pattern 2648/2649)
//
// This module manages initialization state during the hazardous dyld bootstrap
// phase. The following invariants MUST be maintained:
//
// 1. INITIALIZING starts at EarlyInit (2) (passthrough mode) - set to Ready (0)
//    only after dyld completes loading all symbols (via SET_READY in lib.rs)
//
// 2. TLS_READY (RFC-0049) is implicitly handled by the transition from
//    EarlyInit/RustInit to Ready.
//
// 3. All inception layer entry points MUST check these flags BEFORE using any Rust
//    features that might trigger TLS (String, HashMap, Mutex, etc.)
//
// 4. InceptionLayerState::init() uses ONLY libc primitives (malloc, memcpy) to avoid
//    touching Rust allocator which may trigger TLS
//
// Violation of these invariants will cause process deadlock on macOS ARM64.
// See docs/INCEPTION_LAYER_SAFETY_GUIDE.md for details.
// ============================================================================

pub(crate) static INCEPTION_LAYER_STATE: AtomicPtr<InceptionLayerState> =
    AtomicPtr::new(ptr::null_mut());
// Flag to indicate inception layer is still initializing. All syscalls passthrough during this phase.
extern "C" {
    /// RFC-0050: Initialization state machine
    /// 0: Ready (Active), 1: Rust-Init (Safe), 2: Early-Init (Hazardous), 3: Busy (Initializing)
    pub static INITIALIZING: std::sync::atomic::AtomicU8;
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InceptionState {
    Ready = 0,
    RustInit = 1,
    EarlyInit = 2,
    Busy = 3,
}

impl InceptionState {
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Ready,
            1 => Self::RustInit,
            2 => Self::EarlyInit,
            3 => Self::Busy,
            _ => Self::EarlyInit, // Default to safe-passthrough
        }
    }
}

/// Flag to prevent recursion during TLS key creation (bootstrap phase)
pub(crate) static BOOTSTRAPPING: AtomicBool = AtomicBool::new(false);
pub(crate) static DEBUG_ENABLED: AtomicBool = AtomicBool::new(false);
pub(crate) static WORKER_STARTED: AtomicBool = AtomicBool::new(false);

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
    #[allow(clippy::large_stack_frames)] // const fn for static init — never on runtime stack
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
    #[allow(clippy::large_stack_frames)] // delegates to const fn new() for static init
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

// ============================================================================
// DirtyTracker: Lock-Free Pending Write Tracking (M3: Dirty Bit Logic)
// ============================================================================
//
// Tracks paths that have been opened for writing and are in staging files.
// Uses a lock-free fixed-size hash table with linear probing.
// ZERO ALLOCATIONS - safe to call during dyld bootstrap phase.

/// Dirty tracker slot: stores path_hash and staging path offset
/// Format: [32-bit path_hash | 32-bit staging_idx]
/// path_hash = 0 means empty slot
const DIRTY_TRACKER_SIZE: usize = 1024; // Max concurrent dirty files

/// Tombstone marker for deleted slots (allows linear probing to continue)
const TOMBSTONE: u64 = u64::MAX;

/// Global dirty tracker instance
pub static DIRTY_TRACKER: DirtyTracker = DirtyTracker::new();

/// FNV-1a hash for path strings (same as vdir.rs)
#[inline(always)]
pub fn fnv1a_hash(path: &str) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;
    let mut hash = FNV_OFFSET;
    for byte in path.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// Lock-free dirty file tracker
/// Tracks which paths have pending writes in staging files.
pub struct DirtyTracker {
    /// Fixed-size hash table: path_hash -> (staging_idx, active flag)
    /// 0 = empty slot, non-zero = path_hash of dirty file
    slots: [std::sync::atomic::AtomicU64; DIRTY_TRACKER_SIZE],
}

impl Default for DirtyTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl DirtyTracker {
    pub const fn new() -> Self {
        // Initialize all slots to 0 (empty)
        #[allow(clippy::declare_interior_mutable_const)]
        const ZERO: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        Self {
            slots: [ZERO; DIRTY_TRACKER_SIZE],
        }
    }

    /// Mark a path as dirty (has pending writes in staging)
    /// Returns true if successfully marked, false if table is full
    #[inline]
    pub fn mark_dirty(&self, path: &str) -> bool {
        let hash = fnv1a_hash(path);
        if hash == 0 {
            return false; // 0 is reserved for empty
        }

        let start_slot = (hash as usize) % DIRTY_TRACKER_SIZE;
        for i in 0..DIRTY_TRACKER_SIZE {
            let slot = (start_slot + i) % DIRTY_TRACKER_SIZE;
            let current = self.slots[slot].load(Ordering::Acquire);

            // Empty slot - try to claim it
            if current == 0
                && self.slots[slot]
                    .compare_exchange(0, hash, Ordering::SeqCst, Ordering::Acquire)
                    .is_ok()
            {
                return true;
            }
            // CAS failed or slot occupied, continue probing

            // Already marked dirty
            if current == hash {
                return true;
            }
        }
        false // Table full
    }

    /// Clear dirty status for a path
    /// Called after staging file is committed to CAS
    pub fn clear_dirty(&self, path: &str) {
        let hash = fnv1a_hash(path);
        if hash == 0 {
            return;
        }

        let start_slot = (hash as usize) % DIRTY_TRACKER_SIZE;
        for i in 0..DIRTY_TRACKER_SIZE {
            let slot = (start_slot + i) % DIRTY_TRACKER_SIZE;
            let current = self.slots[slot].load(Ordering::Acquire);

            if current == 0 {
                return; // Empty slot - not found
            }

            if current == hash {
                // Found - mark as tombstone (allows probing to continue)
                self.slots[slot].store(TOMBSTONE, Ordering::Release);
                return;
            }

            // Skip tombstones during search
            if current == TOMBSTONE {
                continue;
            }
        }
    }

    /// Check if a path is dirty (has pending writes)
    /// Used in stat/read to redirect to staging file
    #[inline]
    pub fn is_dirty(&self, path: &str) -> bool {
        let hash = fnv1a_hash(path);
        if hash == 0 {
            return false;
        }

        let start_slot = (hash as usize) % DIRTY_TRACKER_SIZE;
        for i in 0..DIRTY_TRACKER_SIZE {
            let slot = (start_slot + i) % DIRTY_TRACKER_SIZE;
            let current = self.slots[slot].load(Ordering::Acquire);

            if current == 0 {
                return false; // Empty slot - not found
            }

            if current == hash {
                return true; // Found - is dirty
            }

            // Skip tombstones during search
            if current == TOMBSTONE {
                continue;
            }
        }
        false
    }

    /// Get count of dirty entries (for debugging)
    #[allow(dead_code)]
    pub fn count(&self) -> usize {
        self.slots
            .iter()
            .filter(|s| {
                let v = s.load(Ordering::Relaxed);
                v != 0 && v != TOMBSTONE
            })
            .count()
    }
}

#[inline(always)]
fn rdtsc() -> u64 {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        std::arch::x86_64::_rdtsc()
    }
    #[cfg(target_arch = "aarch64")]
    /* unsafe */
    {
        0
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
/// Unix timestamp when circuit was tripped (for auto-recovery)
pub static CIRCUIT_TRIP_TIME: AtomicU64 = AtomicU64::new(0);
/// Recovery delay in seconds (default 30s, configurable via VRIFT_CIRCUIT_RECOVERY_DELAY)
pub static CIRCUIT_RECOVERY_DELAY: AtomicU64 = AtomicU64::new(30);

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
// BUG-007b: Dedicated lock for TLS key creation — MUST NOT reuse BOOTSTRAPPING.
// InceptionLayerGuard::enter() sets BOOTSTRAPPING=true before calling get_recursion_key(),
// so reusing BOOTSTRAPPING here would always return key=0, disabling the recursion guard.
static TLS_KEY_LOCK: AtomicBool = AtomicBool::new(false);

pub(crate) fn get_recursion_key() -> libc::pthread_key_t {
    // Fast path: already initialized
    if RECURSION_KEY_INIT.load(Ordering::Acquire) {
        return RECURSION_KEY_VALUE.load(Ordering::Relaxed) as libc::pthread_key_t;
    }

    // Slow path: initialize (only one thread will succeed)
    // BUG-007b: Use dedicated TLS_KEY_LOCK, NOT BOOTSTRAPPING.
    if TLS_KEY_LOCK.swap(true, Ordering::SeqCst) {
        // Another thread is creating the key — spin briefly waiting for it
        for _ in 0..1000 {
            std::hint::spin_loop();
            if RECURSION_KEY_INIT.load(Ordering::Acquire) {
                return RECURSION_KEY_VALUE.load(Ordering::Relaxed) as libc::pthread_key_t;
            }
        }
        return 0; // Give up after spin — TLS guard disabled for this call
    }

    let mut key: libc::pthread_key_t = 0;
    let ret = unsafe { libc::pthread_key_create(&mut key, None) };
    if ret != 0 {
        TLS_KEY_LOCK.store(false, Ordering::SeqCst);
        return 0;
    }

    // Try to be the one to set the value (CAS)
    let expected = 0usize;
    if RECURSION_KEY_VALUE
        .compare_exchange(expected, key as usize, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
    {
        RECURSION_KEY_INIT.store(true, Ordering::Release);
        TLS_KEY_LOCK.store(false, Ordering::SeqCst);
        key
    } else {
        // Another thread beat us, clean up and use their key
        unsafe { libc::pthread_key_delete(key) };
        TLS_KEY_LOCK.store(false, Ordering::SeqCst);
        RECURSION_KEY_VALUE.load(Ordering::Relaxed) as libc::pthread_key_t
    }
}

pub(crate) struct InceptionLayerGuard(bool); // bool: true = has active TLS guard
impl InceptionLayerGuard {
    pub(crate) fn enter() -> Option<Self> {
        // RFC-0050: Only return None when actively initializing (state 3), not for early-init (2) or ready (1)
        // States 1 and 2 should be allowed to proceed so that velo_open_impl can trigger InceptionLayerState::get()
        if (unsafe { INITIALIZING.load(Ordering::Relaxed) }) == 3
            || BOOTSTRAPPING.load(Ordering::Relaxed)
        {
            return None;
        }

        // RFC-0049: Lazy TLS initialization.
        // If INCEPTION_LAYER_STATE is null, we are in the middle of (or about to start) initialization.
        // During this phase, we don't use the TLS recursion guard yet. We rely on the
        // INITIALIZING flag which is set by InceptionLayerState::get() to prevent recursion.
        // This avoids calling pthread_key_create() too early during dyld's initialization.
        if INCEPTION_LAYER_STATE.load(Ordering::Acquire).is_null() {
            return Some(InceptionLayerGuard(false));
        }

        // Set BOOTSTRAPPING true while accessing TLS
        if BOOTSTRAPPING.swap(true, Ordering::SeqCst) {
            return None;
        }

        let res = (|| {
            let key = get_recursion_key();
            if key == 0 {
                // TLS key creation failed - allow proceed without recursion guard
                // This is safe because at this point INCEPTION_LAYER_STATE is initialized
                return Some(InceptionLayerGuard(false));
            }
            let val = unsafe { libc::pthread_getspecific(key) };
            if !val.is_null() {
                None // Already in inception layer - recursion detected
            } else {
                unsafe { libc::pthread_setspecific(key, std::ptr::dangling::<c_void>()) };
                Some(InceptionLayerGuard(true))
            }
        })();

        BOOTSTRAPPING.store(false, Ordering::SeqCst);
        res
    }
}
impl Drop for InceptionLayerGuard {
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
        let path = format!("/tmp/vrift-inception-layer-{}.log", pid);
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

pub(crate) unsafe fn inception_log(msg: &str) {
    LOGGER.log(msg);
    if DEBUG_ENABLED.load(Ordering::Relaxed) {
        #[cfg(target_os = "macos")]
        {
            crate::syscalls::macos_raw::raw_write(2, msg.as_ptr() as *const c_void, msg.len());
        }
        #[cfg(target_os = "linux")]
        {
            libc::write(2, msg.as_ptr() as *const c_void, msg.len());
        }
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

// ============================================================================
// FixedString: Zero-Allocation String Storage
// ============================================================================

#[repr(C)]
#[derive(Clone, Copy)]
pub struct FixedString<const N: usize> {
    pub(crate) data: [u8; N],
    pub(crate) len: usize,
}

impl<const N: usize> FixedString<N> {
    pub const fn new() -> Self {
        Self {
            data: [0u8; N],
            len: 0,
        }
    }

    pub fn set(&mut self, s: &str) {
        let bytes = s.as_bytes();
        let to_copy = std::cmp::min(bytes.len(), N);
        self.data[..to_copy].copy_from_slice(&bytes[..to_copy]);
        self.len = to_copy;
    }

    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.data[..self.len]).unwrap_or("")
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl<const N: usize> std::fmt::Display for FixedString<N> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl<const N: usize> std::fmt::Debug for FixedString<N> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.as_str())
    }
}

impl<const N: usize> std::ops::Deref for FixedString<N> {
    type Target = str;
    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl<const N: usize> AsRef<str> for FixedString<N> {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl<const N: usize> Default for FixedString<N> {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// IdentityHasher: Safe, deterministic hasher for bootstrap safety
// Avoiding RandomState prevents getrandom/open syscalls and TLS usage during init
// ============================================================================

pub(crate) struct IdentityHasher(u64);

impl std::hash::Hasher for IdentityHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        // FNV-1a simple mix
        for &byte in bytes {
            self.0 ^= byte as u64;
            self.0 = self.0.wrapping_mul(0x100000001b3);
        }
    }
    fn write_usize(&mut self, i: usize) {
        // For usize keys (pointers), use them directly mixed
        self.0 ^= i as u64;
        self.0 = self.0.wrapping_mul(0x100000001b3);
    }
    fn write_i32(&mut self, i: i32) {
        // For FD keys
        self.0 ^= i as u64;
        self.0 = self.0.wrapping_mul(0x100000001b3);
    }
}

pub(crate) struct IdentityBuildHasher;

impl std::hash::BuildHasher for IdentityBuildHasher {
    type Hasher = IdentityHasher;
    fn build_hasher(&self) -> Self::Hasher {
        IdentityHasher(0xcbf29ce484222325)
    }
}

impl Default for IdentityBuildHasher {
    fn default() -> Self {
        Self
    }
}

pub(crate) struct OpenFile {
    pub vpath: FixedString<1024>,
    pub temp_path: FixedString<1024>,
    pub mmap_count: usize,
}

pub(crate) struct MmapInfo {
    pub vpath: FixedString<1024>,
    pub temp_path: FixedString<1024>,
    pub len: usize,
}

pub(crate) struct SyntheticDir {
    pub vpath: FixedString<1024>,
    pub entries: Vec<vrift_ipc::DirEntry>,
    pub position: usize,
}
unsafe impl Send for SyntheticDir {} // Raw pointers in open_dirs HashMap
unsafe impl Sync for SyntheticDir {}

pub(crate) static SYNTHETIC_DIR_COUNTER: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

// ============================================================================
// mmap_lookup / mmap_dir_lookup: O(1) manifest lookups
// ============================================================================

/// O(1) mmap-based stat lookup for Hot Stat Cache.
/// Returns None if entry not found or mmap not available.
/// ZERO ALLOCATIONS - safe to call from any context.
#[inline(always)]
pub(crate) fn mmap_lookup(
    mmap_ptr: *const u8,
    mmap_size: usize,
    path: &str,
) -> Option<vrift_ipc::MmapStatEntry> {
    if mmap_ptr.is_null() || mmap_size < vrift_ipc::ManifestMmapHeader::SIZE {
        return None;
    }

    let header = unsafe { &*(mmap_ptr as *const vrift_ipc::ManifestMmapHeader) };

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

// ============================================================================
// InceptionLayerState: Core struct & hot-path methods
// ============================================================================

pub(crate) struct InceptionLayerState {
    pub cas_root: FixedString<1024>,
    pub vfs_prefix: FixedString<256>,
    pub socket_path: FixedString<1024>,
    /// Phase 1.2: vDird socket path, populated from RegisterAck.
    /// Manifest operations are routed here instead of to vriftd.
    pub vdird_socket_path: FixedString<1024>,
    pub open_fds: crate::sync::FdTable,
    pub active_mmaps: RecursiveMutex<HashMap<usize, MmapInfo, IdentityBuildHasher>>,
    pub open_dirs: RecursiveMutex<HashMap<usize, SyntheticDir, IdentityBuildHasher>>,
    pub bloom_ptr: *const u8,
    pub mmap_ptr: *const u8,
    pub mmap_size: usize,
    pub project_root: FixedString<1024>,
    pub path_resolver: PathResolver,
    pub cached_soft_limit: AtomicUsize,
    pub last_usage_alert: std::sync::atomic::AtomicU64,
    pub tasks: &'static crate::sync::RingBuffer,
}

impl InceptionLayerState {
    // Internal helper to avoid infinite recursion when worker needs state
    pub(crate) fn get_no_spawn() -> Option<&'static Self> {
        let ptr = INCEPTION_LAYER_STATE.load(Ordering::Acquire);
        if !ptr.is_null() {
            return unsafe { Some(&*ptr) };
        }
        None
    }

    /// Returns the global InceptionLayerState, initializing it on first call.
    ///
    /// # BUG-007b Safety Constraints
    ///
    /// This function is called from EVERY interposed syscall on EVERY thread.
    /// Its stack frame MUST stay small (<4KB). The init() and open_manifest_mmap()
    /// functions are marked `#[inline(never)]` to prevent the compiler from merging
    /// their ~600KB of local variables into this function's prologue.
    ///
    /// If the stack frame exceeds 512KB (macOS default pthread stack), all worker
    /// threads will silently hang in the prologue's stack probe loop. This was the
    /// root cause of BUG-007b. Verify with:
    ///   `objdump -d libvrift_inception_layer.dylib | grep -A5 'get.*:'`
    ///   Expected: `sub sp, sp, #<small>` (should be < 4096)
    pub(crate) fn get() -> Option<&'static Self> {
        // Phase 6: Stack size guard (debug builds only)
        #[cfg(debug_assertions)]
        {
            if stack_remaining() < 64 * 1024 {
                return None; // Bail out to raw passthrough — not enough stack
            }
        }

        let ptr = INCEPTION_LAYER_STATE.load(Ordering::Acquire);
        if !ptr.is_null() {
            // Lazy spawn worker if not started
            if !WORKER_STARTED.load(Ordering::Relaxed) {
                Self::spawn_worker();
            }
            return unsafe { Some(&*ptr) };
        }

        // RFC-0050: Tiered Readiness Model
        let current = unsafe { INITIALIZING.load(Ordering::Acquire) };
        if current >= InceptionState::EarlyInit as u8 {
            // Still in hazardous dyld phase or already initializing (Busy), return None to fallback to raw syscalls
            return None;
        }

        // Attempt to transition to Busy (3) only from Ready (0)
        let transitioned = unsafe {
            INITIALIZING
                .compare_exchange(
                    InceptionState::Ready as u8,
                    InceptionState::Busy as u8,
                    Ordering::SeqCst,
                    Ordering::Acquire,
                )
                .is_ok()
        };
        if !transitioned {
            return None;
        }

        // Initialize state - reset INITIALIZING to Ready (0) on success
        let ptr = match Self::init() {
            Some(p) => {
                INCEPTION_LAYER_STATE.store(p, Ordering::Release);
                unsafe { INITIALIZING.store(InceptionState::Ready as u8, Ordering::SeqCst) };
                p
            }
            None => {
                unsafe { INITIALIZING.store(InceptionState::Ready as u8, Ordering::SeqCst) };
                return None;
            }
        };

        // RFC-0039 §82: Record initialization event
        inception_record!(EventType::VfsInit, 0, 0);

        // BUG-004: setup_signal_handler and atexit are dangerous during dyld bootstrap.
        let enable_handlers = unsafe {
            let env_key = c"VRIFT_ENABLE_SIGNAL_HANDLERS";
            let val = libc::getenv(env_key.as_ptr());
            !val.is_null() && CStr::from_ptr(val).to_str().unwrap_or("0") == "1"
        };

        if enable_handlers {
            unsafe { init::setup_signal_handler() };
            unsafe { libc::atexit(init::dump_logs_atexit) };
        }

        // Activate VFS - now it's safe to call into Rust from C wrappers.
        activate_vfs();

        unsafe { Some(&*ptr) }
    }

    pub(crate) fn query_manifest(&self, vpath: &VfsPath) -> Option<vrift_ipc::VnodeEntry> {
        // First try Hot Stat Cache (O(1) mmap lookup)
        if let Some(entry) = mmap_lookup(self.mmap_ptr, self.mmap_size, vpath.manifest_key.as_str())
        {
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
        unsafe { sync_ipc_manifest_get(&self.vdird_socket_path, vpath.manifest_key.as_str()) }
    }

    /// Query manifest directly via IPC (bypasses mmap cache)
    /// Required for open() which needs content_hash to locate CAS blob
    pub(crate) fn query_manifest_ipc(&self, vpath: &VfsPath) -> Option<vrift_ipc::VnodeEntry> {
        // Use the centrally resolved manifest key
        unsafe { sync_ipc_manifest_get(&self.vdird_socket_path, &vpath.manifest_key) }
    }

    /// Resolve an incoming path into a VfsPath if it belongs to the VFS.
    pub(crate) fn resolve_path(&self, path: &str) -> Option<VfsPath> {
        self.path_resolver.resolve(path)
    }

    /// Check if path is in VFS domain
    pub(crate) fn inception_applicable(&self, path: &str) -> bool {
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
    /// Phase 3: Fire-and-forget — queued to worker thread
    pub(crate) fn manifest_remove(&self, path: &str) -> Result<(), ()> {
        let request = vrift_ipc::VeloRequest::ManifestRemove {
            path: path.to_string(),
        };
        if unsafe { fire_and_forget_ipc(&self.vdird_socket_path, &request) } {
            Ok(())
        } else {
            Err(())
        }
    }

    /// RFC-0047: Rename/move entry in manifest
    /// Phase 3: Fire-and-forget — queued to worker thread
    pub(crate) fn manifest_rename(&self, old: &str, new: &str) -> Result<(), ()> {
        let request = vrift_ipc::VeloRequest::ManifestRename {
            old_path: old.to_string(),
            new_path: new.to_string(),
        };
        if unsafe { fire_and_forget_ipc(&self.vdird_socket_path, &request) } {
            Ok(())
        } else {
            Err(())
        }
    }

    /// RFC-0047: Create directory entry in manifest
    /// Phase 3: Fire-and-forget — queued to worker thread
    #[allow(clippy::unnecessary_cast)] // mode_t is u16 on macOS, u32 on Linux
    pub(crate) fn manifest_mkdir(&self, path: &str, mode: libc::mode_t) -> Result<(), ()> {
        use std::time::{SystemTime, UNIX_EPOCH};
        let request = vrift_ipc::VeloRequest::ManifestUpsert {
            path: path.to_string(),
            entry: vrift_ipc::VnodeEntry {
                content_hash: [0u8; 32],
                size: 0,
                mtime: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0),
                mode: mode as u32,
                flags: 1, // is_dir flag
                _pad: 0,
            },
        };
        if unsafe { fire_and_forget_ipc(&self.vdird_socket_path, &request) } {
            Ok(())
        } else {
            Err(())
        }
    }

    /// RFC-0039: Create symlink entry in manifest for Live Ingest
    /// Phase 3: Fire-and-forget — queued to worker thread
    pub(crate) fn manifest_symlink(&self, path: &str, _target: &str) -> Result<(), ()> {
        use std::time::{SystemTime, UNIX_EPOCH};
        let request = vrift_ipc::VeloRequest::ManifestUpsert {
            path: path.to_string(),
            entry: vrift_ipc::VnodeEntry {
                content_hash: [0u8; 32],
                size: 0,
                mtime: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0),
                mode: 0o777,
                flags: 2, // is_symlink pseudo-flag
                _pad: 0,
            },
        };
        if unsafe { fire_and_forget_ipc(&self.vdird_socket_path, &request) } {
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
        unsafe { sync_ipc_manifest_list_dir(&self.vdird_socket_path, path) }
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
        use vrift_ipc::{next_seq_id, IpcHeader};

        unsafe {
            let fd = raw_unix_connect(&self.socket_path);
            if fd < 0 {
                return None;
            }

            // Serialize payload
            let payload = rkyv::to_bytes::<rkyv::rancor::Error>(request).ok()?;
            if payload.len() > vrift_ipc::IpcHeader::MAX_LENGTH {
                libc::close(fd);
                return None;
            }

            // Send request frame
            let seq_id = next_seq_id();
            let header = IpcHeader::new_request(payload.len() as u32, seq_id);
            if !raw_write_all(fd, &header.to_bytes()) || !raw_write_all(fd, &payload) {
                libc::close(fd);
                return None;
            }

            // Read response header
            let mut header_buf = [0u8; IpcHeader::SIZE];
            if !raw_read_exact(fd, &mut header_buf) {
                libc::close(fd);
                return None;
            }

            let resp_header = IpcHeader::from_bytes(&header_buf);
            if !resp_header.is_valid() {
                libc::close(fd);
                return None;
            }

            // Read response payload
            let mut resp_buf = vec![0u8; resp_header.length as usize];
            if !raw_read_exact(fd, &mut resp_buf) {
                libc::close(fd);
                return None;
            }

            libc::close(fd);
            rkyv::from_bytes::<vrift_ipc::VeloResponse, rkyv::rancor::Error>(&resp_buf).ok()
        }
    }

    /// Smart FD usage monitoring with zero-overhead, lock-free packed state.
    /// Thresholds: 70% (Warning), 85% (Critical)
    pub(crate) fn check_fd_usage(&self) {
        let soft = self.cached_soft_limit.load(Ordering::Relaxed);
        if soft == 0 {
            return;
        }

        let count = crate::syscalls::io::OPEN_FD_COUNT.load(Ordering::Relaxed);
        let usage_pct = (count * 100) / soft;

        // Determine current threshold level
        let threshold = if usage_pct >= 85 {
            85
        } else if usage_pct >= 70 {
            70
        } else {
            0
        };

        if threshold > 0 {
            let packed = self.last_usage_alert.load(Ordering::Relaxed);
            let last_threshold = (packed >> 32) as usize;
            let last_time = packed & 0xFFFFFFFF;

            let now = unsafe {
                let mut ts = libc::timespec {
                    tv_sec: 0,
                    tv_nsec: 0,
                };
                libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts);
                ts.tv_sec as u64
            };

            // Condition: Higher threshold reached OR 10 seconds pass at same/higher threshold
            if threshold > last_threshold || (threshold == last_threshold && now >= last_time + 10)
            {
                let new_packed = ((threshold as u64) << 32) | (now & 0xFFFFFFFF);
                // Atomic CAS to ensure ONLY ONE thread logs at this second
                if self
                    .last_usage_alert
                    .compare_exchange(packed, new_packed, Ordering::SeqCst, Ordering::Relaxed)
                    .is_ok()
                {
                    let level = if threshold >= 85 {
                        "CRITICAL"
                    } else {
                        "WARNING"
                    };
                    let msg = format!(
                        "[vrift-inception] {}: FD usage at {}% ({} of {}). Build may hang soon!\n",
                        level, usage_pct, count, soft
                    );
                    unsafe { libc::write(2, msg.as_ptr() as *const _, msg.len()) };
                }
            }
        } else if usage_pct < 50 && self.last_usage_alert.load(Ordering::Relaxed) != 0 {
            // Hysteresis: Reset threshold if usage drops safely
            self.last_usage_alert.store(0, Ordering::Release);
        }
    }
}

// ============================================================================
// Phase 6: Stack size guard (debug builds only)
// ============================================================================

/// Check remaining stack space for the current thread.
/// Returns approximate bytes of stack remaining.
/// Only active in debug builds (#[cfg(debug_assertions)]).
#[cfg(debug_assertions)]
fn stack_remaining() -> usize {
    // Read current stack pointer via inline assembly
    let sp: usize;
    #[cfg(target_arch = "aarch64")]
    unsafe {
        std::arch::asm!("mov {}, sp", out(reg) sp);
    }
    #[cfg(target_arch = "x86_64")]
    unsafe {
        std::arch::asm!("mov {}, rsp", out(reg) sp);
    }
    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
    {
        return usize::MAX; // Can't check, assume ok
    }

    // pthread_get_stackaddr_np gives the TOP of the stack (highest address)
    // Stack grows downward, so remaining = sp - stack_bottom
    let thread = unsafe { libc::pthread_self() };
    #[cfg(target_os = "macos")]
    {
        let stack_addr = unsafe { libc::pthread_get_stackaddr_np(thread) } as usize;
        let stack_size = unsafe { libc::pthread_get_stacksize_np(thread) };
        let stack_bottom = stack_addr.saturating_sub(stack_size);
        sp.saturating_sub(stack_bottom)
    }
    #[cfg(target_os = "linux")]
    {
        let mut attr: libc::pthread_attr_t = unsafe { std::mem::zeroed() };
        if unsafe { libc::pthread_getattr_np(thread, &mut attr) } == 0 {
            let mut stack_addr: *mut libc::c_void = std::ptr::null_mut();
            let mut stack_size: libc::size_t = 0;
            unsafe { libc::pthread_attr_getstack(&attr, &mut stack_addr, &mut stack_size) };
            unsafe { libc::pthread_attr_destroy(&mut attr) };
            let stack_bottom = stack_addr as usize;
            sp.saturating_sub(stack_bottom)
        } else {
            usize::MAX // Can't determine, assume ok
        }
    }
}

// ============================================================================
// Tests for DirtyTracker
// ============================================================================
#[cfg(test)]
mod dirty_tracker_tests {
    use super::*;

    #[test]
    fn test_mark_dirty_basic() {
        let tracker = DirtyTracker::new();
        assert!(!tracker.is_dirty("src/main.rs"));

        assert!(tracker.mark_dirty("src/main.rs"));
        assert!(tracker.is_dirty("src/main.rs"));
    }

    #[test]
    fn test_clear_dirty() {
        let tracker = DirtyTracker::new();
        tracker.mark_dirty("src/lib.rs");
        assert!(tracker.is_dirty("src/lib.rs"));

        tracker.clear_dirty("src/lib.rs");
        assert!(!tracker.is_dirty("src/lib.rs"));
    }

    #[test]
    fn test_multiple_paths() {
        let tracker = DirtyTracker::new();
        let paths = [
            "src/main.rs",
            "src/lib.rs",
            "Cargo.toml",
            "README.md",
            "tests/integration.rs",
        ];

        for path in &paths {
            tracker.mark_dirty(path);
        }

        for path in &paths {
            assert!(tracker.is_dirty(path), "Expected {} to be dirty", path);
        }

        assert!(!tracker.is_dirty("nonexistent.rs"));
    }

    #[test]
    fn test_clear_nonexistent() {
        let tracker = DirtyTracker::new();
        // Should not panic or error
        tracker.clear_dirty("nonexistent.rs");
        assert!(!tracker.is_dirty("nonexistent.rs"));
    }

    #[test]
    fn test_mark_same_path_twice() {
        let tracker = DirtyTracker::new();
        assert!(tracker.mark_dirty("src/main.rs"));
        assert!(tracker.mark_dirty("src/main.rs")); // Should succeed (idempotent)
        assert!(tracker.is_dirty("src/main.rs"));

        assert_eq!(tracker.count(), 1); // Should only have one entry
    }

    #[test]
    fn test_count() {
        let tracker = DirtyTracker::new();
        assert_eq!(tracker.count(), 0);

        tracker.mark_dirty("file1.rs");
        assert_eq!(tracker.count(), 1);

        tracker.mark_dirty("file2.rs");
        assert_eq!(tracker.count(), 2);

        tracker.clear_dirty("file1.rs");
        assert_eq!(tracker.count(), 1);
    }

    #[test]
    fn test_fnv1a_hash_deterministic() {
        let path = "src/main.rs";
        let h1 = fnv1a_hash(path);
        let h2 = fnv1a_hash(path);
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_fnv1a_hash_different_paths() {
        let h1 = fnv1a_hash("src/main.rs");
        let h2 = fnv1a_hash("src/lib.rs");
        assert_ne!(h1, h2);
    }

    #[test]
    fn test_fnv1a_hash_empty_string() {
        let h = fnv1a_hash("");
        assert_ne!(h, 0); // Empty string should still produce valid hash
    }

    #[test]
    fn test_long_path() {
        let tracker = DirtyTracker::new();
        let long_path = "a".repeat(1000) + "/very/long/path/to/file.rs";

        assert!(tracker.mark_dirty(&long_path));
        assert!(tracker.is_dirty(&long_path));

        tracker.clear_dirty(&long_path);
        assert!(!tracker.is_dirty(&long_path));
    }

    #[test]
    fn test_stress_many_entries() {
        let tracker = DirtyTracker::new();

        // Add 500 entries (half capacity)
        for i in 0..500 {
            let path = format!("file_{}.rs", i);
            assert!(tracker.mark_dirty(&path), "Failed to mark {}", path);
        }

        assert_eq!(tracker.count(), 500);

        // Verify all are dirty
        for i in 0..500 {
            let path = format!("file_{}.rs", i);
            assert!(tracker.is_dirty(&path), "Expected {} to be dirty", path);
        }

        // Clear half
        for i in 0..250 {
            let path = format!("file_{}.rs", i);
            tracker.clear_dirty(&path);
        }

        for i in 0..250 {
            let path = format!("file_{}.rs", i);
            assert!(
                !tracker.is_dirty(&path),
                "Expected {} to NOT be dirty",
                path
            );
        }

        // And remaining paths should still be dirty
        for i in 250..500 {
            let path = format!("file_{}.rs", i);
            assert!(tracker.is_dirty(&path), "Expected {} to remain dirty", path);
        }
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn test_concurrent_mark_dirty() {
        use std::sync::Arc;
        use std::thread;

        let tracker = Arc::new(DirtyTracker::new());
        let mut handles = vec![];

        // Spawn 4 threads, each marking 100 unique paths
        for t in 0..4 {
            let tracker = Arc::clone(&tracker);
            handles.push(thread::spawn(move || {
                for i in 0..100 {
                    let path = format!("thread_{}_file_{}.rs", t, i);
                    tracker.mark_dirty(&path);
                }
            }));
        }

        for handle in handles {
            handle.join().unwrap();
        }

        // All 400 entries should be marked
        assert_eq!(tracker.count(), 400);

        // Verify each entry
        for t in 0..4 {
            for i in 0..100 {
                let path = format!("thread_{}_file_{}.rs", t, i);
                assert!(tracker.is_dirty(&path), "Expected {} to be dirty", path);
            }
        }
    }
}
