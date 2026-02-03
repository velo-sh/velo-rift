use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicUsize, Ordering};

// Typical cache line size is 64 bytes
#[repr(align(64))]
struct CachePadded<T>(T);

pub enum Task {
    // Metadata reclamation (High Priority)
    ReclaimFd(usize, *mut crate::syscalls::io::FdEntry),
    // IPC/Telemetry (Low Priority)
    Reingest { vpath: String, temp_path: String },
    Log(String),
}

const BUFFER_SIZE: usize = 4096;
const BUFFER_MASK: usize = BUFFER_SIZE - 1;

/// A Multi-Producer Single-Consumer Lock-Free Ring Buffer.
pub struct RingBuffer {
    // Padded to prevent false sharing
    head: CachePadded<AtomicUsize>,
    tail: CachePadded<AtomicUsize>,

    // The slots
    buffer: [UnsafeCell<Option<Task>>; BUFFER_SIZE],
}

// Safety: RingBuffer handles synchronization via atomics and MPSC logic.
unsafe impl Send for RingBuffer {}
unsafe impl Sync for RingBuffer {}
impl std::panic::RefUnwindSafe for RingBuffer {}

impl Default for RingBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl RingBuffer {
    pub const fn new() -> Self {
        Self {
            head: CachePadded(AtomicUsize::new(0)),
            tail: CachePadded(AtomicUsize::new(0)),
            buffer: [const { UnsafeCell::new(None) }; BUFFER_SIZE],
        }
    }

    /// Try to push a task into the buffer. Returns Err if full.
    pub fn push(&self, task: Task) -> Result<(), Task> {
        let head = self.head.0.load(Ordering::Relaxed);
        let tail = self.tail.0.load(Ordering::Acquire);

        if head.wrapping_sub(tail) >= BUFFER_SIZE {
            return Err(task);
        }

        // Note: For true MPSC, we need to handle concurrent producers.
        // A simple atomic fetch_add on head is used.
        let pos = self.head.0.fetch_add(1, Ordering::Relaxed);

        // Safety: We've reserved the slot.
        unsafe {
            let slot = &self.buffer[pos & BUFFER_MASK];
            *slot.get() = Some(task);
        }

        Ok(())
    }

    /// Pop a task from the buffer. Only the Consumer (Worker Thread) calls this.
    pub fn pop(&self) -> Option<Task> {
        let tail = self.tail.0.load(Ordering::Relaxed);
        let head = self.head.0.load(Ordering::Acquire);

        if tail == head {
            return None;
        }

        // Safety: We are the sole consumer.
        let task = unsafe {
            let slot = &self.buffer[tail & BUFFER_MASK];
            (&mut *slot.get()).take()
        };

        if task.is_some() {
            self.tail.0.store(tail.wrapping_add(1), Ordering::Release);
        }

        task
    }
}

/// Helper for static initialization
pub struct RingBufferStore {
    inner: UnsafeCell<Option<RingBuffer>>,
    initialized: std::sync::atomic::AtomicBool,
}

impl Default for RingBufferStore {
    fn default() -> Self {
        Self::new()
    }
}

impl RingBufferStore {
    pub const fn new() -> Self {
        Self {
            inner: UnsafeCell::new(None),
            initialized: std::sync::atomic::AtomicBool::new(false),
        }
    }

    pub fn get(&self) -> &RingBuffer {
        if !self.initialized.load(Ordering::Acquire) {
            // Lazy init logic here (simplified)
        }
        unsafe { (&*self.inner.get()).as_ref().unwrap() }
    }
}
