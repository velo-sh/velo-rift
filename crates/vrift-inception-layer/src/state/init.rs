// =============================================================================
// state/init.rs — Cold-path initialization code
// =============================================================================
//
// BUG-007b: All functions here are #[inline(never)] to prevent merging
// their large stack frames into get()'s prologue. This module contains:
//
//   - init_logger()       — read env vars for log level and debug mode
//   - boost_fd_limit()    — raise RLIMIT_NOFILE to 80% of hard cap
//   - open_manifest_mmap()— mmap the manifest file for O(1) stat lookup
//   - init()              — primary initialization, allocates state via raw_mmap
//   - audit_environment() — detect hazardous env vars
//   - init_reactor()      — initialize the ring buffer reactor
//   - setup_signal_handler() / dump_logs_atexit() — optional signal/exit handlers
// =============================================================================

use crate::path::PathResolver;
use crate::sync::RecursiveMutex;
use libc::c_void;
use std::collections::HashMap;
use std::ffi::CStr;
use std::path::PathBuf;
use std::ptr;
use std::sync::atomic::Ordering;

use super::{
    FixedString, IdentityBuildHasher, InceptionLayerState, LogLevel, CIRCUIT_BREAKER_THRESHOLD,
    DEBUG_ENABLED, FLIGHT_RECORDER, LOGGER, LOG_LEVEL,
};

impl InceptionLayerState {
    pub(crate) unsafe fn init_logger() {
        let debug_ptr = libc::getenv(c"VRIFT_DEBUG".as_ptr());
        if !debug_ptr.is_null() {
            DEBUG_ENABLED.store(true, Ordering::Relaxed);
        }

        // RFC-0050: Read log level
        let level_ptr = unsafe { libc::getenv(c"VRIFT_LOG_LEVEL".as_ptr()) };
        if !level_ptr.is_null() {
            // Zero-allocation parsing
            let level_bytes = unsafe { CStr::from_ptr(level_ptr).to_bytes() };
            let level = if level_bytes.eq_ignore_ascii_case(b"trace") {
                LogLevel::Trace
            } else if level_bytes.eq_ignore_ascii_case(b"debug") {
                LogLevel::Debug
            } else if level_bytes.eq_ignore_ascii_case(b"info") {
                LogLevel::Info
            } else if level_bytes.eq_ignore_ascii_case(b"warn") {
                LogLevel::Warn
            } else if level_bytes.eq_ignore_ascii_case(b"error") {
                LogLevel::Error
            } else if level_bytes.eq_ignore_ascii_case(b"off") {
                LogLevel::Off
            } else {
                LogLevel::Info
            };
            LOG_LEVEL.store(level as u8, Ordering::Relaxed);
        }

        // RFC-0050: Read circuit breaker threshold
        let threshold_ptr = unsafe { libc::getenv(c"VRIFT_CIRCUIT_BREAKER_THRESHOLD".as_ptr()) };
        if !threshold_ptr.is_null() {
            let threshold_bytes = unsafe { CStr::from_ptr(threshold_ptr).to_bytes() };
            if let Ok(s) = std::str::from_utf8(threshold_bytes) {
                if let Ok(threshold) = s.parse::<usize>() {
                    CIRCUIT_BREAKER_THRESHOLD.store(threshold, Ordering::Relaxed);
                }
            }
        }
    }

    /// Attempt to raise RLIMIT_NOFILE to exactly 80% of the true hard cap.
    pub(crate) fn boost_fd_limit() -> usize {
        let mut soft_limit = 1024; // Default
        let mut rl = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut rl) } == 0 {
            soft_limit = rl.rlim_cur as usize;
            // Determine the "true" hard cap even if RLIM_INFINITY is returned
            let hard_cap = if rl.rlim_max == libc::RLIM_INFINITY {
                #[cfg(target_os = "macos")]
                {
                    let mut max_files: libc::c_int = 0;
                    let mut size = std::mem::size_of_val(&max_files);
                    if unsafe {
                        libc::sysctlbyname(
                            c"kern.maxfilesperproc".as_ptr(),
                            &mut max_files as *mut _ as *mut _,
                            &mut size,
                            std::ptr::null_mut(),
                            0,
                        )
                    } == 0
                    {
                        max_files as libc::rlim_t
                    } else {
                        10240 // Sane fallback
                    }
                }
                #[cfg(target_os = "linux")]
                {
                    1048576
                }
                #[cfg(not(any(target_os = "macos", target_os = "linux")))]
                {
                    65536
                }
            } else {
                rl.rlim_max
            };

            // UX: Explicit guidance if hard limit is dangerously low
            if hard_cap < 4096 {
                let msg = "[vrift-inception] ⚠️  WARNING: System FD hard limit is extremely low. This will likely cause build failures.\n\
                     [vrift-inception] 👉 Action: Run 'ulimit -Hn 65536' or check /etc/security/limits.conf\n";
                unsafe { libc::write(2, msg.as_ptr() as *const _, msg.len()) };
            }

            // Policy: Boost to EXACTLY 80% of the true hard cap.
            let target = (hard_cap as f64 * 0.8) as libc::rlim_t;

            if rl.rlim_cur < target {
                let old_cur = rl.rlim_cur;
                rl.rlim_cur = target;
                if unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &rl) } == 0 {
                    // Safe logging without allocation
                    let mut buf = [0u8; 128];
                    let mut writer = crate::macros::StackWriter::new(&mut buf);
                    use std::fmt::Write;
                    let _ = writeln!(
                        writer,
                        "[vrift-inception] 🚀 Optimized FD limit: {} -> {} (target: 80%)",
                        old_cur, rl.rlim_cur
                    );
                    let msg = writer.as_str();
                    unsafe { libc::write(2, msg.as_ptr() as *const _, msg.len()) };
                    soft_limit = rl.rlim_cur as usize;
                }
            }
        }
        soft_limit
    }

    /// BUG-007b: MUST NOT be inlined into get().
    /// init() + open_manifest_mmap() together allocate ~605KB on stack
    /// (FixedStrings, PATH_MAX buffers, InceptionLayerState struct).
    /// macOS pthread stacks default to 512KB → stack overflow in get()'s prologue,
    /// silently hanging all threads in the stack probe loop.
    #[inline(never)]
    #[cold]
    pub(crate) fn init() -> Option<*mut Self> {
        let soft_limit = Self::boost_fd_limit();
        unsafe { Self::init_logger() };

        let mut cas_root = FixedString::<1024>::new();
        let cas_ptr = unsafe { libc::getenv(c"VR_THE_SOURCE".as_ptr()) };

        // 1. Determine raw path (Env or Default)
        let raw_path = if !cas_ptr.is_null() {
            unsafe { CStr::from_ptr(cas_ptr).to_string_lossy() }
        } else {
            std::borrow::Cow::Borrowed(vrift_ipc::DEFAULT_CAS_ROOT)
        };

        // 2. Perform safe tilde expansion
        if raw_path.starts_with("~/") {
            let home_ptr = unsafe { libc::getenv(c"HOME".as_ptr()) };
            if !home_ptr.is_null() {
                let home = unsafe { CStr::from_ptr(home_ptr).to_string_lossy() };

                // Safe concatenation on stack
                let mut path_buf = [0u8; 1024];
                let mut writer = crate::macros::StackWriter::new(&mut path_buf);
                use std::fmt::Write;
                let _ = write!(writer, "{}{}", home, &raw_path[1..]); // Skip '~'
                cas_root.set(writer.as_str());
            } else {
                cas_root.set(&raw_path);
            }
        } else {
            cas_root.set(&raw_path);
        }

        let mut vfs_prefix = FixedString::<256>::new();
        let prefix_ptr = unsafe { libc::getenv(c"VRIFT_VFS_PREFIX".as_ptr()) };
        if !prefix_ptr.is_null() {
            let raw_prefix_cstr = unsafe { CStr::from_ptr(prefix_ptr) };
            if let Ok(raw_prefix) = raw_prefix_cstr.to_str() {
                // BUG-007 + RFC-0050: Avoid raw_realpath/realpath during init to prevent deadlocks.
                // We use raw_path_normalize which is a pure string function (zero syscalls).
                let mut norm_buf = [0u8; 1024];
                if let Some(len) =
                    unsafe { crate::path::raw_path_normalize(raw_prefix, &mut norm_buf) }
                {
                    vfs_prefix.set(std::str::from_utf8(&norm_buf[..len]).unwrap_or(raw_prefix));
                } else {
                    vfs_prefix.set(raw_prefix);
                }
            }
        }

        let mut socket_path = FixedString::<1024>::new();
        let socket_ptr = unsafe { libc::getenv(c"VRIFT_SOCKET_PATH".as_ptr()) };
        if socket_ptr.is_null() {
            socket_path.set(vrift_ipc::DEFAULT_SOCKET_PATH);
        } else {
            socket_path.set(&unsafe { CStr::from_ptr(socket_ptr).to_string_lossy() });
        }

        let (mmap_ptr, mmap_size) = open_manifest_mmap();

        let mut project_root_fs = FixedString::<1024>::new();
        let manifest_ptr = unsafe { libc::getenv(c"VRIFT_MANIFEST".as_ptr()) };
        if !manifest_ptr.is_null() {
            let manifest_path_cstr = unsafe { CStr::from_ptr(manifest_ptr) };
            if let Ok(manifest_path) = manifest_path_cstr.to_str() {
                // Manual derivation of project root from manifest path (parent of .vrift)
                // Replaces PathBuf/std::fs to avoid interposition & allocations.
                let mut root_path = manifest_path;
                if let Some(idx) = manifest_path.find("/.vrift/") {
                    root_path = &manifest_path[..idx];
                } else if let Some(stripped) = manifest_path.strip_suffix("/.vrift") {
                    root_path = stripped;
                } else if let Some(idx) = manifest_path.rfind('/') {
                    root_path = &manifest_path[..idx];
                }

                if root_path.is_empty() {
                    root_path = "/";
                }

                // Normalize manually
                let mut norm_buf = [0u8; 1024];
                if let Some(len) =
                    unsafe { crate::path::raw_path_normalize(root_path, &mut norm_buf) }
                {
                    project_root_fs.set(std::str::from_utf8(&norm_buf[..len]).unwrap_or(root_path));
                } else {
                    project_root_fs.set(root_path);
                }

                // Resolve symlinks (e.g. /tmp → /private/tmp on macOS).
                // Critical: getcwd returns kernel-canonicalized paths, so project_root
                // must also be canonicalized for starts_with() reverse-mapping to work.
                //
                // NOTE: Cannot use raw_realpath() here because INITIALIZING is Busy (3)
                // during init(), which triggers raw_realpath's bootstrap guard that just
                // copies the path unchanged. Instead, use raw open+fcntl(F_GETPATH)+close
                // syscalls directly — the same technique raw_realpath uses internally.
                let root_cstr =
                    std::ffi::CString::new(project_root_fs.as_str()).unwrap_or_default();
                #[cfg(target_os = "macos")]
                {
                    let fd = unsafe {
                        crate::syscalls::macos_raw::raw_open(
                            root_cstr.as_ptr(),
                            libc::O_RDONLY | libc::O_CLOEXEC,
                            0,
                        )
                    };
                    if fd >= 0 {
                        let mut resolved_buf = [0u8; libc::PATH_MAX as usize];
                        let ret = unsafe {
                            crate::syscalls::macos_raw::raw_fcntl(
                                fd,
                                libc::F_GETPATH,
                                resolved_buf.as_mut_ptr() as i64,
                            )
                        };
                        unsafe { crate::syscalls::macos_raw::raw_close(fd) };
                        if ret >= 0 {
                            if let Ok(s) = unsafe {
                                CStr::from_ptr(resolved_buf.as_ptr() as *const libc::c_char)
                                    .to_str()
                            } {
                                project_root_fs.set(s);
                            }
                        }
                    }
                }
                #[cfg(target_os = "linux")]
                {
                    let mut resolved_buf = [0u8; libc::PATH_MAX as usize];
                    let resolved_ptr = unsafe {
                        libc::realpath(
                            root_cstr.as_ptr(),
                            resolved_buf.as_mut_ptr() as *mut libc::c_char,
                        )
                    };
                    if !resolved_ptr.is_null() {
                        if let Ok(s) = unsafe { CStr::from_ptr(resolved_ptr).to_str() } {
                            project_root_fs.set(s);
                        }
                    }
                }
            }
        }

        // RFC-CRIT-001: Bootstrap-Safe Allocation using raw_mmap
        // Replaces malloc to avoid fstat->shim->malloc deadlock on macOS (BUG-007)
        let size = std::mem::size_of::<InceptionLayerState>();

        #[cfg(target_os = "macos")]
        let ptr = unsafe {
            crate::syscalls::macos_raw::raw_mmap(
                ptr::null_mut(),
                size,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANON,
                -1,
                0,
            ) as *mut InceptionLayerState
        };

        #[cfg(target_os = "linux")]
        let ptr = unsafe {
            crate::syscalls::linux_raw::raw_mmap(
                ptr::null_mut(),
                size,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
                -1,
                0,
            ) as *mut InceptionLayerState
        };

        if ptr == libc::MAP_FAILED as *mut InceptionLayerState {
            return None;
        }

        unsafe {
            ptr::write(
                ptr,
                InceptionLayerState {
                    cas_root,
                    vfs_prefix,
                    socket_path,
                    vdird_socket_path: FixedString::new(),
                    open_fds: crate::sync::FdTable::new(),
                    active_mmaps: RecursiveMutex::new(HashMap::with_hasher(IdentityBuildHasher)),
                    open_dirs: RecursiveMutex::new(HashMap::with_hasher(IdentityBuildHasher)),
                    bloom_ptr: ptr::null(),
                    mmap_ptr,
                    mmap_size,
                    project_root: project_root_fs,
                    path_resolver: PathResolver::new(vfs_prefix.as_str(), project_root_fs.as_str()),
                    cached_soft_limit: std::sync::atomic::AtomicUsize::new(soft_limit),
                    last_usage_alert: std::sync::atomic::AtomicU64::new(0),
                    tasks: Self::init_reactor(),
                },
            );
        }

        // Perform proactive environment audit (Safe: uses getenv and safe logger)
        unsafe { Self::audit_environment() };

        // Install custom panic handler for better diagnostics (Phase 5)
        install_panic_handler();

        Some(ptr)
    }

    /// RFC-0050: Proactively detect hazardous environment variables
    /// that might cause conflicts during dyld bootstrap.
    unsafe fn audit_environment() {
        #[cfg(target_os = "macos")]
        let hazardous_vars = [c"DYLD_LIBRARY_PATH", c"DYLD_FALLBACK_LIBRARY_PATH"];
        #[cfg(target_os = "linux")]
        let hazardous_vars = [c"LD_LIBRARY_PATH", c"LD_PRELOAD"];

        for &var in &hazardous_vars {
            let val = libc::getenv(var.as_ptr());
            if !val.is_null() {
                let name = var.to_str().unwrap_or("UNKNOWN");
                inception_warn!("Hazardous env var detected during bootstrap: {}", name);
            }
        }
    }

    #[allow(clippy::large_stack_frames)] // Reactor contains RingBuffer (197KB) — runs once during cold init
    pub(super) fn init_reactor() -> &'static crate::sync::RingBuffer {
        unsafe {
            if crate::sync::get_reactor().is_none() {
                let reactor = crate::sync::Reactor {
                    fd_table: crate::sync::FdTable::new(),
                    ring_buffer: crate::sync::RingBuffer::new(),
                    started: std::sync::atomic::AtomicBool::new(true),
                };
                // Use addr_of_mut to avoid creating a reference to static mut
                let reactor_ptr = std::ptr::addr_of_mut!(crate::sync::REACTOR)
                    as *mut Option<crate::sync::Reactor>;
                *reactor_ptr = Some(reactor);

                // Start Worker Thread via pthread LATER
                // BUG-008: Spawning in ctor causes deadlock with dyld loader lock
                // Self::spawn_worker(); NO!

                // Now mark as ready for fast path in get_reactor()
                crate::sync::mark_reactor_ready();
            }

            // Safety: We just initialized it above if it was missing.
            match crate::sync::get_reactor() {
                Some(r) => &r.ring_buffer,
                None => {
                    // Should be unreachable.
                    libc::abort();
                }
            }
        }
    }
}

// =============================================================================
// open_manifest_mmap: mmap-based O(1) stat lookup (BUG-007b: #[inline(never)])
// =============================================================================

/// Open mmap'd manifest file for O(1) stat lookup.
/// Returns (ptr, size) or (null, 0) if unavailable.
/// Uses raw libc to avoid recursion through inception layer.
/// BUG-007b: MUST NOT be inlined — allocates large stack buffers (PATH_MAX etc.)
/// that would overflow the 512KB default pthread stack if merged into get().
#[inline(never)]
#[cold]
#[allow(deprecated)]
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

    // Phase 1.3: Read VRIFT_VDIR_MMAP env (zero-RPC, set by CLI)
    let vdir_mmap_ptr = unsafe { libc::getenv(c"VRIFT_VDIR_MMAP".as_ptr()) };

    // Construct path on stack
    let mut path_buf = [0u8; 1024];
    let mut writer = crate::macros::StackWriter::new(&mut path_buf);
    use std::fmt::Write;

    if !vdir_mmap_ptr.is_null() {
        // Phase 1.3: Direct path from env — no derivation needed
        let vdir_str = unsafe { CStr::from_ptr(vdir_mmap_ptr) };
        let _ = write!(writer, "{}\0", vdir_str.to_str().unwrap_or(""));
    } else {
        // Fallback: Derive from VRIFT_MANIFEST (legacy path)
        let manifest_ptr = unsafe { libc::getenv(c"VRIFT_MANIFEST".as_ptr()) };
        if manifest_ptr.is_null() {
            return (ptr::null(), 0);
        }

        let root_bytes = unsafe { CStr::from_ptr(manifest_ptr).to_bytes() };

        // Naively assume project root is parent of manifest
        let mut last_slash = 0;
        for (i, &b) in root_bytes.iter().enumerate() {
            if b == b'/' {
                last_slash = i;
            }
        }

        let root_len = if last_slash > 0 {
            last_slash
        } else {
            root_bytes.len()
        };

        // If ending in .vrift, strip it too
        let root_part = &root_bytes[..root_len];
        let final_root_len = if root_part.ends_with(b"/.vrift") {
            root_len - 7
        } else if root_part.ends_with(b".vrift") {
            root_len - 6
        } else {
            root_len
        };

        let root_str = std::str::from_utf8(&root_bytes[..final_root_len]).unwrap_or("");

        // BUG-007b: Use raw_realpath instead of std::fs::canonicalize()
        let canon_root = unsafe {
            let mut resolved = [0u8; libc::PATH_MAX as usize];
            let root_cstr = std::ffi::CString::new(root_str).unwrap_or_default();
            #[cfg(target_os = "macos")]
            let result = crate::syscalls::macos_raw::raw_realpath(
                root_cstr.as_ptr(),
                resolved.as_mut_ptr() as *mut libc::c_char,
            );
            #[cfg(target_os = "linux")]
            let result = libc::realpath(
                root_cstr.as_ptr(),
                resolved.as_mut_ptr() as *mut libc::c_char,
            );
            if !result.is_null() {
                let resolved_str = CStr::from_ptr(result).to_string_lossy().to_string();
                PathBuf::from(resolved_str)
            } else {
                PathBuf::from(root_str)
            }
        };
        let canon_root_str = canon_root.to_string_lossy();

        let project_id = vrift_config::path::compute_project_id(canon_root_str.as_ref());
        let mmap_path = vrift_config::path::get_vdir_mmap_path(&project_id)
            .unwrap_or_else(|| PathBuf::from(format!("{}/.vrift/manifest.mmap", canon_root_str)));

        let _ = write!(writer, "{}\0", mmap_path.display());
    }

    let mmap_path_ptr = path_buf.as_ptr() as *const libc::c_char;

    #[cfg(target_os = "macos")]
    let fd = unsafe {
        crate::syscalls::macos_raw::raw_open(mmap_path_ptr, libc::O_RDONLY | libc::O_CLOEXEC, 0)
    };
    #[cfg(target_os = "linux")]
    let fd = unsafe {
        crate::syscalls::linux_raw::raw_openat(
            libc::AT_FDCWD,
            mmap_path_ptr,
            libc::O_RDONLY | libc::O_CLOEXEC,
            0,
        )
    };
    if fd < 0 {
        return (ptr::null(), 0);
    }

    // Get file size via fstat
    let mut stat_buf: libc::stat = unsafe { std::mem::zeroed() };
    #[cfg(target_os = "macos")]
    let fstat_result = unsafe { crate::syscalls::macos_raw::raw_fstat64(fd, &mut stat_buf) };
    #[cfg(target_os = "linux")]
    let fstat_result = unsafe { crate::syscalls::linux_raw::raw_fstat(fd, &mut stat_buf) };
    if fstat_result != 0 {
        #[cfg(target_os = "macos")]
        unsafe {
            crate::syscalls::macos_raw::raw_close(fd)
        };
        #[cfg(target_os = "linux")]
        unsafe {
            crate::syscalls::linux_raw::raw_close(fd)
        };
        return (ptr::null(), 0);
    }
    let size = stat_buf.st_size as usize;

    // mmap the file read-only
    #[cfg(target_os = "macos")]
    let ptr = unsafe {
        crate::syscalls::macos_raw::raw_mmap(
            ptr::null_mut(),
            size,
            libc::PROT_READ,
            libc::MAP_SHARED, // Phase 1.3: MAP_SHARED for real-time vDird visibility
            fd,
            0,
        )
    };
    #[cfg(target_os = "linux")]
    let ptr = unsafe {
        crate::syscalls::linux_raw::raw_mmap(
            ptr::null_mut(),
            size,
            libc::PROT_READ,
            libc::MAP_SHARED, // Phase 1.3: MAP_SHARED for real-time vDird visibility
            fd,
            0,
        )
    };
    #[cfg(target_os = "macos")]
    unsafe {
        crate::syscalls::macos_raw::raw_close(fd)
    };
    #[cfg(target_os = "linux")]
    unsafe {
        crate::syscalls::linux_raw::raw_close(fd)
    };

    if ptr == libc::MAP_FAILED {
        return (ptr::null(), 0);
    }

    // Phase 1.3: Validate VDirHeader magic instead of ManifestMmapHeader
    use vrift_ipc::vdir_types::{VDIR_HEADER_SIZE, VDIR_MAGIC};
    if size < VDIR_HEADER_SIZE {
        unsafe { libc::munmap(ptr, size) };
        return (ptr::null(), 0);
    }
    let magic = unsafe { *(ptr as *const u32) };
    if magic != VDIR_MAGIC {
        // Fallback: Try legacy ManifestMmapHeader format
        if size >= vrift_ipc::ManifestMmapHeader::SIZE {
            let header = unsafe { &*(ptr as *const vrift_ipc::ManifestMmapHeader) };
            if header.is_valid() {
                return (ptr as *const u8, size);
            }
        }
        unsafe { libc::munmap(ptr, size) };
        return (ptr::null(), 0);
    }

    (ptr as *const u8, size)
}

// =============================================================================
// Signal handler & atexit (optional, env-gated)
// =============================================================================

pub(crate) extern "C" fn dump_logs_atexit() {
    LOGGER.dump_to_file();
    if DEBUG_ENABLED.load(Ordering::Relaxed) {
        super::vfs_dump_flight_recorder();
    }
}

pub(crate) unsafe fn setup_signal_handler() {
    #[cfg(target_os = "macos")]
    {
        use libc::{signal, SIGUSR1};
        extern "C" fn handle_sigusr1(_sig: libc::c_int) {
            super::vfs_dump_flight_recorder();
        }
        signal(SIGUSR1, handle_sigusr1 as usize);
    }
}

// =============================================================================
// Phase 5: Custom panic handler for better diagnostics
// =============================================================================

/// Install a custom panic hook that uses raw I/O for zero-allocation diagnostics.
/// The hook itself must be zero-alloc since it may fire during interposed syscalls.
/// The Box::new allocation here is safe because init() runs after TLS is ready.
fn install_panic_handler() {
    std::panic::set_hook(Box::new(|info| {
        // Format: [vrift-shim] FATAL: panic at <location>
        let msg = b"[vrift-shim] FATAL: panic in inception layer";
        unsafe {
            #[cfg(target_os = "macos")]
            crate::syscalls::macos_raw::raw_write(2, msg.as_ptr() as *const c_void, msg.len());
            #[cfg(target_os = "linux")]
            crate::syscalls::linux_raw::raw_write(2, msg.as_ptr() as *const c_void, msg.len());
        }

        // Try to include location info using a stack buffer
        if let Some(location) = info.location() {
            let mut buf = [0u8; 256];
            let mut writer = crate::macros::StackWriter::new(&mut buf);
            use std::fmt::Write;
            let _ = write!(writer, " at {}:{}", location.file(), location.line());
            let loc_msg = writer.as_str();
            unsafe {
                #[cfg(target_os = "macos")]
                crate::syscalls::macos_raw::raw_write(
                    2,
                    loc_msg.as_ptr() as *const c_void,
                    loc_msg.len(),
                );
                #[cfg(target_os = "linux")]
                crate::syscalls::linux_raw::raw_write(
                    2,
                    loc_msg.as_ptr() as *const c_void,
                    loc_msg.len(),
                );
            }
        }

        let newline = b"\n";
        unsafe {
            #[cfg(target_os = "macos")]
            crate::syscalls::macos_raw::raw_write(
                2,
                newline.as_ptr() as *const c_void,
                newline.len(),
            );
            #[cfg(target_os = "linux")]
            crate::syscalls::linux_raw::raw_write(
                2,
                newline.as_ptr() as *const c_void,
                newline.len(),
            );
        }

        // Dump flight recorder for post-mortem
        FLIGHT_RECORDER.record(super::EventType::IpcFail, 0, -99);

        // Abort instead of unwinding — unwinding in interposed context is catastrophic
        unsafe { libc::abort() };
    }));
}
