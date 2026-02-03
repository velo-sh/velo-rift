use crate::syscalls::io::FdEntry;
use std::ptr;
use std::sync::atomic::{AtomicPtr, Ordering};

// RFC-0051: Tiered atomic array for lock-free FD tracking
// 256K FDs should cover 99.9% of real-world scenarios while saving memory
const TIER1_SIZE: usize = 256;
const TIER2_SIZE: usize = 1024;
pub const MAX_FDS: usize = TIER1_SIZE * TIER2_SIZE; // 262,144 FDs

/// A tiered atomic array for wait-free FD tracking.
/// Supports up to 262,144 FDs with lazy tier-2 allocation.
#[repr(align(64))]
pub struct FdTable {
    // Level 1: Sparse array of chunks
    table: [AtomicPtr<Tier2>; TIER1_SIZE],
}

#[repr(align(64))]
struct Tier2 {
    entries: [AtomicPtr<FdEntry>; TIER2_SIZE],
}

impl Default for FdTable {
    fn default() -> Self {
        Self::new()
    }
}

impl FdTable {
    pub const fn new() -> Self {
        // Initialize with null pointers for lazy allocation
        const NULL_TIER: *mut Tier2 = ptr::null_mut();
        Self {
            table: [const { AtomicPtr::new(ptr::null_mut()) }; TIER1_SIZE],
        }
    }

    /// Set the entry for a given FD. Returns the OLD entry if any.
    #[inline(always)]
    pub fn set(&self, fd: u32, entry: *mut FdEntry) -> *mut FdEntry {
        if fd >= MAX_FDS as u32 {
            return ptr::null_mut();
        }

        let i1 = (fd / TIER2_SIZE as u32) as usize;
        let i2 = (fd % TIER2_SIZE as u32) as usize;

        let mut tier2_ptr = self.table[i1].load(Ordering::Acquire);
        if tier2_ptr.is_null() {
            // Lazy allocation of the second tier
            let new_tier = Box::into_raw(Box::new(Tier2 {
                entries: [const { AtomicPtr::new(ptr::null_mut()) }; TIER2_SIZE],
            }));

            match self.table[i1].compare_exchange(
                ptr::null_mut(),
                new_tier,
                Ordering::Release,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    tier2_ptr = new_tier;
                }
                Err(existing) => {
                    // Someone else initialized it
                    unsafe { drop(Box::from_raw(new_tier)) };
                    tier2_ptr = existing;
                }
            }
        }

        unsafe { (&*tier2_ptr).entries[i2].swap(entry, Ordering::AcqRel) }
    }

    /// Get a clone of the entry for a given FD.
    #[inline(always)]
    pub fn get(&self, fd: u32) -> *mut FdEntry {
        if fd >= MAX_FDS as u32 {
            return ptr::null_mut();
        }

        let i1 = (fd / TIER2_SIZE as u32) as usize;
        let i2 = (fd % TIER2_SIZE as u32) as usize;

        // Use Relaxed for reads - we don't need synchronization for lookups
        let tier2_ptr = self.table[i1].load(Ordering::Relaxed);
        if tier2_ptr.is_null() {
            return ptr::null_mut();
        }

        unsafe { (&*tier2_ptr).entries[i2].load(Ordering::Relaxed) }
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
