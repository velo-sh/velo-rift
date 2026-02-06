use std::cell::UnsafeCell;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicBool, Ordering};

/// A Recursive Mutex using raw pthread primitives.
///
/// This allows a thread to acquire the same lock multiple times without deadlocking.
/// Designed for use in inception-layer's deep call chains.
pub struct RecursiveMutex<T> {
    inner: UnsafeCell<libc::pthread_mutex_t>,
    data: UnsafeCell<T>,
    initialized: AtomicBool,
    init_lock: AtomicBool,
}

unsafe impl<T: Send> Send for RecursiveMutex<T> {}
unsafe impl<T: Send> Sync for RecursiveMutex<T> {}

impl<T> RecursiveMutex<T> {
    pub const fn new(data: T) -> Self {
        Self {
            inner: UnsafeCell::new(libc::PTHREAD_MUTEX_INITIALIZER),
            data: UnsafeCell::new(data),
            initialized: AtomicBool::new(false),
            init_lock: AtomicBool::new(false),
        }
    }

    fn ensure_init(&self) {
        if self.initialized.load(Ordering::Acquire) {
            return;
        }

        // Spinlock to serialize initialization
        while self
            .init_lock
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            std::hint::spin_loop();
        }

        if !self.initialized.load(Ordering::Relaxed) {
            unsafe {
                let mut attr: libc::pthread_mutexattr_t = std::mem::zeroed();
                libc::pthread_mutexattr_init(&mut attr);
                libc::pthread_mutexattr_settype(&mut attr, libc::PTHREAD_MUTEX_RECURSIVE);
                libc::pthread_mutex_init(self.inner.get(), &attr);
                libc::pthread_mutexattr_destroy(&mut attr);
                self.initialized.store(true, Ordering::Release);
            }
        }

        self.init_lock.store(false, Ordering::Release);
    }

    pub fn lock(&self) -> RecursiveMutexGuard<'_, T> {
        self.ensure_init();
        unsafe {
            libc::pthread_mutex_lock(self.inner.get());
        }
        RecursiveMutexGuard { mutex: self }
    }
}

impl<T> Drop for RecursiveMutex<T> {
    fn drop(&mut self) {
        if self.initialized.load(Ordering::Acquire) {
            unsafe {
                libc::pthread_mutex_destroy(self.inner.get());
            }
        }
    }
}

pub struct RecursiveMutexGuard<'a, T> {
    mutex: &'a RecursiveMutex<T>,
}

impl<'a, T> Deref for RecursiveMutexGuard<'a, T> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { &*self.mutex.data.get() }
    }
}

impl<'a, T> DerefMut for RecursiveMutexGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.mutex.data.get() }
    }
}

impl<'a, T> Drop for RecursiveMutexGuard<'a, T> {
    fn drop(&mut self) {
        unsafe {
            libc::pthread_mutex_unlock(self.mutex.inner.get());
        }
    }
}
