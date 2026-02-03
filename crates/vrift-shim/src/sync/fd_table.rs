use crate::syscalls::io::FdEntry;
use std::collections::HashMap;
use std::ptr;
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::Mutex;

// RFC-0051: Flat atomic array for lock-free FD tracking
// Direct indexing for maximum performance (eliminates one indirection)
const MAX_FDS: usize = 262144; // 256K FDs (fast path)

/// A flat atomic array for wait-free FD tracking.
/// Optimized for extreme performance with zero indirection.
/// Fixed 2MB memory cost (~262K × 8 bytes).
///
/// For FDs >= 262K, falls back to a Mutex<HashMap> (slow path).
#[repr(align(64))]
pub struct FdTable {
    // Fast path: Direct flat array for FD < 262K (one atomic load)
    entries: [AtomicPtr<FdEntry>; MAX_FDS],

    // Slow path: Overflow for FD >= 262K (rare, use mutex)
    overflow: Mutex<HashMap<u32, *mut FdEntry>>,
}

impl Default for FdTable {
    fn default() -> Self {
        Self::new()
    }
}

impl FdTable {
    pub fn new() -> Self {
        Self {
            entries: [const { AtomicPtr::new(ptr::null_mut()) }; MAX_FDS],
            overflow: Mutex::new(HashMap::new()),
        }
    }

    /// Set the entry for a given FD. Returns the OLD entry if any.
    #[inline(always)]
    pub fn set(&self, fd: u32, entry: *mut FdEntry) -> *mut FdEntry {
        // Fast path: use flat array (hot path, zero contention)
        if fd < MAX_FDS as u32 {
            return self.entries[fd as usize].swap(entry, Ordering::AcqRel);
        }

        // Slow path: overflow HashMap (rare, acceptable mutex overhead)
        let mut overflow = self.overflow.lock().unwrap();
        if entry.is_null() {
            overflow.remove(&fd).unwrap_or(ptr::null_mut())
        } else {
            overflow.insert(fd, entry).unwrap_or(ptr::null_mut())
        }
    }

    /// Get the entry for a given FD.
    #[inline(always)]
    pub fn get(&self, fd: u32) -> *mut FdEntry {
        // Fast path: use flat array (hot path, zero contention)
        if fd < MAX_FDS as u32 {
            return self.entries[fd as usize].load(Ordering::Relaxed);
        }

        // Slow path: overflow HashMap (rare)
        let overflow = self.overflow.lock().unwrap();
        overflow.get(&fd).copied().unwrap_or(ptr::null_mut())
    }

    /// Remove an entry. Returns the removed entry.
    #[inline(always)]
    pub fn remove(&self, fd: u32) -> *mut FdEntry {
        self.set(fd, ptr::null_mut())
    }
}

// Safety: FdTable handles its own synchronization via atomics.
unsafe impl Send for FdTable {}
unsafe impl Sync for FdTable {}
