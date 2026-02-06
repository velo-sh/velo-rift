/// Pattern 2648/2649: Passthrough guard for shim initialization safety.
/// Use this at the start of every shim function to bypass VFS logic during dyld bootstrap.
///
/// # INITIALIZING State Machine:
/// - State 2: Early-Init (dyld loading) - TLS unsafe, MUST passthrough
/// - State 3: Busy (ShimState initializing) - TLS unsafe, MUST passthrough  
/// - State 1: C constructor ran - TLS safe, can run Rust
/// - State 0: Fully initialized - TLS safe, can run Rust
///
/// # Usage:
/// ```ignore
/// passthrough_if_init!(real, arg1, arg2);  // Expands to early return if state >= 2
/// ```
#[macro_export]
macro_rules! passthrough_if_init {
    ($real:expr $(, $arg:expr)*) => {
        if $crate::state::INITIALIZING.load(std::sync::atomic::Ordering::Relaxed) >= 2
            || $crate::state::CIRCUIT_TRIPPED.load(std::sync::atomic::Ordering::Relaxed)
        {
            return $real($($arg),*);
        }
    };
}

/// BUG-007 Pattern: Safe early passthrough using interpose old_func.
/// This MUST be called BEFORE any dlsym call to avoid malloc recursion deadlock.
///
/// During __malloc_init, syscalls like fstat, mmap, close are called before dlsym is safe.
/// This macro uses the interpose table's old_func pointer directly (resolved by dyld).
///
/// # Usage:
/// ```ignore
/// safe_early_passthrough!(IT_FSTAT, fn(c_int, *mut libc_stat) -> c_int, fd, buf);
/// ```
#[macro_export]
macro_rules! safe_early_passthrough {
    ($interpose:expr, fn($($ptype:ty),*) -> $rtype:ty $(, $arg:expr)*) => {
        if $crate::state::INITIALIZING.load(std::sync::atomic::Ordering::Relaxed) >= 2
            || $crate::state::CIRCUIT_TRIPPED.load(std::sync::atomic::Ordering::Relaxed)
        {
            let real_fn = std::mem::transmute::<
                *const (),
                unsafe extern "C" fn($($ptype),*) -> $rtype,
            >($interpose.old_func);
            return real_fn($($arg),*);
        }
    };
}

#[macro_export]
macro_rules! shim_log {
    ($msg:expr) => {
        $crate::LOGGER.log($msg)
    };
}

#[macro_export]
macro_rules! vfs_log_at_level {
    ($level:expr, $tag:expr, $($arg:tt)*) => {
        {
            if $crate::state::LOG_LEVEL.load(std::sync::atomic::Ordering::Relaxed) <= ($level as u8) {
                // Stack-based formatting to avoid heap allocation
                // Use a local buffer and recursion guard
                if let Some(_guard) = $crate::state::ShimGuard::enter() {
                    use std::fmt::Write;
                    let mut buf = [0u8; 512];
                    let mut wrapper = $crate::macros::StackWriter::new(&mut buf);
                    let pid = unsafe { libc::getpid() };
                    let _ = write!(wrapper, "[VFS][{}][{}] ", pid, $tag);
                    let _ = write!(wrapper, $($arg)*);
                    let _ = writeln!(wrapper);

                    let msg = wrapper.as_str();
                    unsafe {
                        $crate::state::LOGGER.log(msg);
                        if $crate::state::DEBUG_ENABLED.load(std::sync::atomic::Ordering::Relaxed) {
                            libc::write(2, msg.as_ptr() as *const libc::c_void, msg.len());
                        }
                    }
                }
            }
        }
    };
}

// Zero-allocation structured event recording (Flight Recorder)
#[macro_export]
macro_rules! vfs_record {
    ($event:expr, $file_id:expr, $result:expr) => {{
        $crate::state::FLIGHT_RECORDER.record($event, $file_id, $result as i32);
    }};
}

#[macro_export]
macro_rules! vfs_trace { ($($arg:tt)*) => { $crate::vfs_log_at_level!($crate::state::LogLevel::Trace, "TRACE", $($arg)*) }; }
#[macro_export]
macro_rules! vfs_debug { ($($arg:tt)*) => { $crate::vfs_log_at_level!($crate::state::LogLevel::Debug, "DEBUG", $($arg)*) }; }
#[macro_export]
macro_rules! vfs_info { ($($arg:tt)*) => { $crate::vfs_log_at_level!($crate::state::LogLevel::Info, "INFO", $($arg)*) }; }
#[macro_export]
macro_rules! vfs_warn { ($($arg:tt)*) => { $crate::vfs_log_at_level!($crate::state::LogLevel::Warn, "WARN", $($arg)*) }; }
#[macro_export]
macro_rules! vfs_error { ($($arg:tt)*) => { $crate::vfs_log_at_level!($crate::state::LogLevel::Error, "ERROR", $($arg)*) }; }

// Compatibility shim for existing code
#[macro_export]
macro_rules! vfs_log { ($($arg:tt)*) => { $crate::vfs_info!($($arg)*) }; }

#[macro_export]
macro_rules! get_real {
    ($storage:ident, $name:literal, $t:ty) => {{
        let p = $storage.load(std::sync::atomic::Ordering::Acquire);
        if !p.is_null() {
            std::mem::transmute::<*mut libc::c_void, $t>(p)
        } else {
            // zero-alloc C string constant
            let f = libc::dlsym(
                libc::RTLD_NEXT,
                concat!($name, "\0").as_ptr() as *const libc::c_char,
            );
            $storage.store(f, std::sync::atomic::Ordering::Release);
            std::mem::transmute::<*mut libc::c_void, $t>(f)
        }
    }};
}

#[macro_export]
macro_rules! get_real_shim {
    ($storage:ident, $name:literal, $it:ident, $t:ty) => {{
        #[cfg(target_os = "macos")]
        {
            // RFC-0051: Bypass dlsym(RTLD_NEXT) by using the old_func pointer
            // already resolved by dyld in the interpose table.
            // This eliminates dyld lock contention and recursion deadlocks (Pattern 2682.v2).
            let p = $it.old_func;
            std::mem::transmute::<*const (), $t>(p)
        }
        #[cfg(not(target_os = "macos"))]
        {
            $crate::get_real!($storage, $name, $t)
        }
    }};
}

pub struct StackWriter<'a> {
    buf: &'a mut [u8],
    pos: usize,
}

impl<'a> StackWriter<'a> {
    pub fn new(buf: &'a mut [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.buf[..self.pos]).unwrap_or("")
    }
}

impl<'a> std::fmt::Write for StackWriter<'a> {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        let bytes = s.as_bytes();
        let remaining = self.buf.len() - self.pos;
        let to_copy = std::cmp::min(bytes.len(), remaining);
        self.buf[self.pos..self.pos + to_copy].copy_from_slice(&bytes[..to_copy]);
        self.pos += to_copy;
        Ok(())
    }
}
