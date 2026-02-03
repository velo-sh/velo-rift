pub mod fd_table;
pub mod recursive_mutex;
pub mod ring_buffer;

pub use fd_table::FdTable;
pub use recursive_mutex::RecursiveMutex;
pub use ring_buffer::{RingBuffer, Task};

use std::cell::UnsafeCell;
use std::sync::atomic::AtomicBool;

/// Global Reactor State
pub struct Reactor {
    pub fd_table: FdTable,
    pub ring_buffer: RingBuffer,
    pub started: AtomicBool,
}

// We'll use a manually managed static for the Reactor to avoid init hazards
pub static mut REACTOR: UnsafeCell<Option<Reactor>> = UnsafeCell::new(None);

pub fn get_reactor() -> Option<&'static Reactor> {
    unsafe { (*REACTOR.get()).as_ref() }
}
