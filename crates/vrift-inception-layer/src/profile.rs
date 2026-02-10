//! RFC-0045: VFS Performance Profiling — Phase 1+2
//!
//! Zero-overhead when disabled (VRIFT_PROFILE unset).
//! When enabled (VRIFT_PROFILE=1), tracks:
//!   - Syscall call counts (Phase 1)
//!   - Per-syscall latency in nanoseconds (Phase 2)
//!   - VFS contribution and cache stats
//!   - Top-N hot paths (sampled)
//!
//! On process exit (atexit), writes JSON to `/tmp/vrift-profile-<pid>.json`.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Master enable flag — checked by profile_count!/profile_timed! macros.
pub static PROFILE_ENABLED: AtomicBool = AtomicBool::new(false);

/// Global profile counters — always present in .bss, zero cost when disabled.
pub static PROFILE: VriftProfile = VriftProfile::new();

// ── High-resolution timer ──

/// Get monotonic time in nanoseconds. Uses mach_absolute_time on macOS (~5ns)
/// or clock_gettime on Linux (~20ns).
#[inline(always)]
pub fn now_ns() -> u64 {
    #[cfg(target_os = "macos")]
    {
        // mach_absolute_time is ~5ns on Apple Silicon, much cheaper than clock_gettime
        extern "C" {
            fn mach_absolute_time() -> u64;
        }
        // On Apple Silicon, mach_absolute_time returns nanoseconds directly
        // (timebase is 1:1). On Intel Macs this would need conversion, but
        // we target ARM64 primarily.
        unsafe { mach_absolute_time() }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let mut ts = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        unsafe {
            libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts);
        }
        ts.tv_sec as u64 * 1_000_000_000 + ts.tv_nsec as u64
    }
}

/// RFC-0045 Phase 2: Performance profile counters with latency tracking
#[repr(C)]
pub struct VriftProfile {
    // ── Syscall Counters (count + cumulative latency in ns) ──
    pub stat_calls: AtomicU64,
    pub stat_ns: AtomicU64,
    pub fstat_calls: AtomicU64,
    pub fstat_ns: AtomicU64,
    pub lstat_calls: AtomicU64,
    pub lstat_ns: AtomicU64,
    pub open_calls: AtomicU64,
    pub open_ns: AtomicU64,
    pub close_calls: AtomicU64,
    pub close_ns: AtomicU64,
    pub read_calls: AtomicU64,
    pub read_ns: AtomicU64,
    pub write_calls: AtomicU64,
    pub write_ns: AtomicU64,
    pub readdir_calls: AtomicU64,
    pub readdir_ns: AtomicU64,
    pub access_calls: AtomicU64,
    pub access_ns: AtomicU64,

    // ── VFS Contribution ──
    pub vfs_handled: AtomicU64,
    pub vfs_passthrough: AtomicU64,

    // ── Cache Stats ──
    pub vdir_hits: AtomicU64,
    pub vdir_misses: AtomicU64,
    pub ipc_calls: AtomicU64,
    pub cas_materializations: AtomicU64,

    // ── Latency breakdown (cumulative ns) ──
    pub vdir_lookup_ns: AtomicU64,   // Time spent in VDir mmap lookups
    pub ipc_roundtrip_ns: AtomicU64, // Time spent in IPC to daemon

    // ── Timestamp ──
    pub start_time_ns: AtomicU64,

    // ── Top-N Hot Paths (simple sampled atomic counter) ──
    pub sample_counter: AtomicU64, // Total sampled paths recorded

    // ── Init time (ns from dylib load to init complete) ──
    pub init_ns: AtomicU64,
}

// Safety: All fields are AtomicU64/AtomicBool — inherently Sync.
unsafe impl Sync for VriftProfile {}

impl VriftProfile {
    pub const fn new() -> Self {
        Self {
            stat_calls: AtomicU64::new(0),
            stat_ns: AtomicU64::new(0),
            fstat_calls: AtomicU64::new(0),
            fstat_ns: AtomicU64::new(0),
            lstat_calls: AtomicU64::new(0),
            lstat_ns: AtomicU64::new(0),
            open_calls: AtomicU64::new(0),
            open_ns: AtomicU64::new(0),
            close_calls: AtomicU64::new(0),
            close_ns: AtomicU64::new(0),
            read_calls: AtomicU64::new(0),
            read_ns: AtomicU64::new(0),
            write_calls: AtomicU64::new(0),
            write_ns: AtomicU64::new(0),
            readdir_calls: AtomicU64::new(0),
            readdir_ns: AtomicU64::new(0),
            access_calls: AtomicU64::new(0),
            access_ns: AtomicU64::new(0),
            vfs_handled: AtomicU64::new(0),
            vfs_passthrough: AtomicU64::new(0),
            vdir_hits: AtomicU64::new(0),
            vdir_misses: AtomicU64::new(0),
            ipc_calls: AtomicU64::new(0),
            cas_materializations: AtomicU64::new(0),
            vdir_lookup_ns: AtomicU64::new(0),
            ipc_roundtrip_ns: AtomicU64::new(0),
            start_time_ns: AtomicU64::new(0),
            sample_counter: AtomicU64::new(0),
            init_ns: AtomicU64::new(0),
        }
    }
}

impl Default for VriftProfile {
    fn default() -> Self {
        Self::new()
    }
}

/// Increment a profile counter if profiling is enabled.
/// Compiles to a single atomic branch + fetch_add on hot path.
#[macro_export]
macro_rules! profile_count {
    ($field:ident) => {
        if $crate::profile::PROFILE_ENABLED.load(std::sync::atomic::Ordering::Relaxed) {
            $crate::profile::PROFILE
                .$field
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    };
}

/// Time a block and record both count and cumulative latency.
/// Usage: profile_timed!(stat_calls, stat_ns, { ... actual_syscall ... })
/// Cost: ~10ns overhead (2x mach_absolute_time) when profiling enabled.
#[macro_export]
macro_rules! profile_timed {
    ($count_field:ident, $ns_field:ident, $body:expr) => {{
        if $crate::profile::PROFILE_ENABLED.load(std::sync::atomic::Ordering::Relaxed) {
            let _t0 = $crate::profile::now_ns();
            let _result = $body;
            let _elapsed = $crate::profile::now_ns().wrapping_sub(_t0);
            $crate::profile::PROFILE
                .$count_field
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            $crate::profile::PROFILE
                .$ns_field
                .fetch_add(_elapsed, std::sync::atomic::Ordering::Relaxed);
            _result
        } else {
            $body
        }
    }};
}

/// Record latency for a sub-operation (VDir lookup, IPC roundtrip).
/// Usage: profile_latency!(vdir_lookup_ns, { vdir_lookup(...) })
#[macro_export]
macro_rules! profile_latency {
    ($ns_field:ident, $body:expr) => {{
        if $crate::profile::PROFILE_ENABLED.load(std::sync::atomic::Ordering::Relaxed) {
            let _t0 = $crate::profile::now_ns();
            let _result = $body;
            let _elapsed = $crate::profile::now_ns().wrapping_sub(_t0);
            $crate::profile::PROFILE
                .$ns_field
                .fetch_add(_elapsed, std::sync::atomic::Ordering::Relaxed);
            _result
        } else {
            $body
        }
    }};
}

// ── Per-file hot path tracking ──
// 256-slot open-addressing hash table tracking per-file latency.
// Zero cost when profiling disabled. Lock-free with atomic operations.

const HOT_PATH_SLOTS: usize = 256;
const HOT_PATH_STR_LEN: usize = 128;

/// Single entry in the hot path table.
/// Layout: path_hash (8B) + total_ns (8B) + call_count (8B) + path_str (128B) = 152B
/// Total table: 256 * 152 = ~38KB in .bss — zero cost when unused.
#[repr(C)]
pub struct HotPathEntry {
    pub path_hash: AtomicU64,  // FNV-1a hash of path (0 = empty slot)
    pub total_ns: AtomicU64,   // Cumulative latency
    pub call_count: AtomicU64, // Number of calls
    pub path_str: [std::sync::atomic::AtomicU8; HOT_PATH_STR_LEN], // Truncated path
}

impl HotPathEntry {
    const fn new() -> Self {
        // SAFETY: AtomicU8::new(0) repeated — we need a workaround for const init
        // Using transmute from zeroed bytes since AtomicU8 has same repr
        Self {
            path_hash: AtomicU64::new(0),
            total_ns: AtomicU64::new(0),
            call_count: AtomicU64::new(0),
            path_str: unsafe { std::mem::zeroed() },
        }
    }
}

// SAFETY: All fields are atomic.
unsafe impl Sync for HotPathEntry {}

// Macro to create the static array since we can't use const generics with static
#[allow(unused_macros)]
macro_rules! make_hot_path_table {
    ($n:expr) => {{
        // SAFETY: HotPathEntry is all-zeros by default (atomics init to 0)
        unsafe { std::mem::zeroed::<[HotPathEntry; $n]>() }
    }};
}

/// Global hot path table — lives in .bss, zero cost when disabled.
pub static HOT_PATH_TABLE: std::sync::LazyLock<Box<[HotPathEntry; HOT_PATH_SLOTS]>> =
    std::sync::LazyLock::new(|| {
        // SAFETY: All-zeros is valid for HotPathEntry (all atomics start at 0)
        unsafe { Box::new(std::mem::zeroed()) }
    });

/// Record a file path + latency into the hot path table.
/// Called from open/stat interpositions with the elapsed time.
/// Uses open-addressing with linear probing. Lock-free via atomic CAS.
#[inline]
pub unsafe fn profile_record_path(path: *const libc::c_char, elapsed_ns: u64) {
    if !PROFILE_ENABLED.load(Ordering::Relaxed) || path.is_null() {
        return;
    }

    // Hash the path using FNV-1a (same as VDir)
    let path_cstr = unsafe { std::ffi::CStr::from_ptr(path) };
    let path_bytes = path_cstr.to_bytes();
    if path_bytes.is_empty() {
        return;
    }
    let hash = fnv1a_hash(path_bytes);
    if hash == 0 {
        return; // 0 = empty sentinel
    }

    let table = &*HOT_PATH_TABLE;
    let start_slot = (hash as usize) % HOT_PATH_SLOTS;

    for i in 0..16 {
        // Only probe 16 slots to bound worst case
        let slot = (start_slot + i) % HOT_PATH_SLOTS;
        let entry = &table[slot];

        let existing = entry.path_hash.load(Ordering::Relaxed);
        if existing == hash {
            // Found existing entry — accumulate
            entry.total_ns.fetch_add(elapsed_ns, Ordering::Relaxed);
            entry.call_count.fetch_add(1, Ordering::Relaxed);
            return;
        }
        if existing == 0 {
            // Empty slot — try to claim it
            match entry
                .path_hash
                .compare_exchange(0, hash, Ordering::AcqRel, Ordering::Relaxed)
            {
                Ok(_) => {
                    // Claimed! Write path string (one-time, truncated)
                    let copy_len = path_bytes.len().min(HOT_PATH_STR_LEN - 1);
                    for (j, &byte) in path_bytes.iter().enumerate().take(copy_len) {
                        entry.path_str[j].store(byte, Ordering::Relaxed);
                    }
                    entry.total_ns.fetch_add(elapsed_ns, Ordering::Relaxed);
                    entry.call_count.fetch_add(1, Ordering::Relaxed);
                    return;
                }
                Err(actual) if actual == hash => {
                    // Another thread claimed it with same hash — accumulate
                    entry.total_ns.fetch_add(elapsed_ns, Ordering::Relaxed);
                    entry.call_count.fetch_add(1, Ordering::Relaxed);
                    return;
                }
                Err(_) => {
                    // Another thread claimed it with different hash — continue probing
                    continue;
                }
            }
        }
        // Slot occupied by different hash — continue probing
    }
    // Table full for this probe chain — drop sample (rare)
}

/// FNV-1a hash for path bytes
#[inline]
fn fnv1a_hash(data: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in data {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// Collect hot path entries sorted by total_ns descending.
/// Returns vec of (path_string, total_ns, call_count).
pub fn collect_hot_paths(top_n: usize) -> Vec<(String, u64, u64)> {
    let table = &*HOT_PATH_TABLE;
    let mut entries: Vec<(String, u64, u64)> = Vec::new();

    for entry in table.iter() {
        let hash = entry.path_hash.load(Ordering::Relaxed);
        if hash == 0 {
            continue;
        }
        let ns = entry.total_ns.load(Ordering::Relaxed);
        let count = entry.call_count.load(Ordering::Relaxed);

        // Read path string
        let mut path_buf = Vec::with_capacity(HOT_PATH_STR_LEN);
        for j in 0..HOT_PATH_STR_LEN {
            let b = entry.path_str[j].load(Ordering::Relaxed);
            if b == 0 {
                break;
            }
            path_buf.push(b);
        }
        let path = String::from_utf8_lossy(&path_buf).to_string();
        entries.push((path, ns, count));
    }

    entries.sort_by(|a, b| b.1.cmp(&a.1)); // Sort by total_ns descending
    entries.truncate(top_n);
    entries
}

/// Initialize profiling: check VRIFT_PROFILE env var, record start time.
/// Called from InceptionLayerState::init() after env is safe to read.
pub fn init_profile() {
    // Read env var — safe because this runs after dyld bootstrap
    let enabled = std::env::var("VRIFT_PROFILE")
        .map(|v| v == "1" || v.to_lowercase() == "true")
        .unwrap_or(false);

    if !enabled {
        return;
    }

    PROFILE_ENABLED.store(true, Ordering::Release);

    // Record session start time using monotonic clock
    PROFILE.start_time_ns.store(now_ns(), Ordering::Relaxed);

    // Register atexit handler to dump profile on normal exit
    unsafe {
        libc::atexit(profile_atexit_handler);
    }
}

/// atexit handler — writes profile JSON to /tmp/vrift-profile-<pid>.json
extern "C" fn profile_atexit_handler() {
    if !PROFILE_ENABLED.load(Ordering::Relaxed) {
        return;
    }
    dump_profile_json();
}

/// Write profile data as JSON to /tmp/vrift-profile-<pid>.json
fn dump_profile_json() {
    use std::fmt::Write;

    let pid = unsafe { libc::getpid() };

    // Snapshot all counters (Relaxed is fine — atexit is single-threaded)
    let stat = PROFILE.stat_calls.load(Ordering::Relaxed);
    let stat_t = PROFILE.stat_ns.load(Ordering::Relaxed);
    let fstat = PROFILE.fstat_calls.load(Ordering::Relaxed);
    let fstat_t = PROFILE.fstat_ns.load(Ordering::Relaxed);
    let lstat = PROFILE.lstat_calls.load(Ordering::Relaxed);
    let lstat_t = PROFILE.lstat_ns.load(Ordering::Relaxed);
    let open = PROFILE.open_calls.load(Ordering::Relaxed);
    let open_t = PROFILE.open_ns.load(Ordering::Relaxed);
    let close = PROFILE.close_calls.load(Ordering::Relaxed);
    let close_t = PROFILE.close_ns.load(Ordering::Relaxed);
    let read = PROFILE.read_calls.load(Ordering::Relaxed);
    let read_t = PROFILE.read_ns.load(Ordering::Relaxed);
    let write_c = PROFILE.write_calls.load(Ordering::Relaxed);
    let write_t = PROFILE.write_ns.load(Ordering::Relaxed);
    let readdir = PROFILE.readdir_calls.load(Ordering::Relaxed);
    let readdir_t = PROFILE.readdir_ns.load(Ordering::Relaxed);
    let access = PROFILE.access_calls.load(Ordering::Relaxed);
    let access_t = PROFILE.access_ns.load(Ordering::Relaxed);

    let handled = PROFILE.vfs_handled.load(Ordering::Relaxed);
    let passthrough = PROFILE.vfs_passthrough.load(Ordering::Relaxed);
    let vdir_hit = PROFILE.vdir_hits.load(Ordering::Relaxed);
    let vdir_miss = PROFILE.vdir_misses.load(Ordering::Relaxed);
    let ipc = PROFILE.ipc_calls.load(Ordering::Relaxed);
    let vdir_ns = PROFILE.vdir_lookup_ns.load(Ordering::Relaxed);
    let ipc_ns = PROFILE.ipc_roundtrip_ns.load(Ordering::Relaxed);
    let start = PROFILE.start_time_ns.load(Ordering::Relaxed);

    let end = now_ns();
    let duration_ns = end.saturating_sub(start);
    let duration_ms = duration_ns / 1_000_000;
    let init_time = PROFILE.init_ns.load(Ordering::Relaxed);

    let total_calls = stat + fstat + lstat + open + close + read + write_c + readdir + access;
    let total_ns =
        stat_t + fstat_t + lstat_t + open_t + close_t + read_t + write_t + readdir_t + access_t;

    let mut buf = String::with_capacity(4096);
    let _ = writeln!(buf, "{{");
    let _ = writeln!(buf, "  \"pid\": {},", pid);
    let _ = writeln!(buf, "  \"duration_ms\": {},", duration_ms);
    let _ = writeln!(buf, "  \"init_ns\": {},", init_time);
    let _ = writeln!(buf, "  \"total_syscalls\": {},", total_calls);
    let _ = writeln!(buf, "  \"total_syscall_ns\": {},", total_ns);

    // Syscalls with latency
    let _ = writeln!(buf, "  \"syscalls\": {{");
    let _ = writeln!(
        buf,
        "    \"stat\": {{ \"count\": {}, \"total_ns\": {} }},",
        stat, stat_t
    );
    let _ = writeln!(
        buf,
        "    \"fstat\": {{ \"count\": {}, \"total_ns\": {} }},",
        fstat, fstat_t
    );
    let _ = writeln!(
        buf,
        "    \"lstat\": {{ \"count\": {}, \"total_ns\": {} }},",
        lstat, lstat_t
    );
    let _ = writeln!(
        buf,
        "    \"open\": {{ \"count\": {}, \"total_ns\": {} }},",
        open, open_t
    );
    let _ = writeln!(
        buf,
        "    \"close\": {{ \"count\": {}, \"total_ns\": {} }},",
        close, close_t
    );
    let _ = writeln!(
        buf,
        "    \"read\": {{ \"count\": {}, \"total_ns\": {} }},",
        read, read_t
    );
    let _ = writeln!(
        buf,
        "    \"write\": {{ \"count\": {}, \"total_ns\": {} }},",
        write_c, write_t
    );
    let _ = writeln!(
        buf,
        "    \"readdir\": {{ \"count\": {}, \"total_ns\": {} }},",
        readdir, readdir_t
    );
    let _ = writeln!(
        buf,
        "    \"access\": {{ \"count\": {}, \"total_ns\": {} }}",
        access, access_t
    );
    let _ = writeln!(buf, "  }},");

    // VFS contribution
    let _ = writeln!(buf, "  \"vfs\": {{");
    let _ = writeln!(buf, "    \"handled\": {},", handled);
    let _ = writeln!(buf, "    \"passthrough\": {},", passthrough);
    let handled_pct = if handled + passthrough > 0 {
        100.0 * handled as f64 / (handled + passthrough) as f64
    } else {
        0.0
    };
    let _ = writeln!(buf, "    \"handled_pct\": {:.1}", handled_pct);
    let _ = writeln!(buf, "  }},");

    // Cache stats with latency
    let _ = writeln!(buf, "  \"cache\": {{");
    let _ = writeln!(buf, "    \"vdir_hits\": {},", vdir_hit);
    let _ = writeln!(buf, "    \"vdir_misses\": {},", vdir_miss);
    let hit_rate = if vdir_hit + vdir_miss > 0 {
        100.0 * vdir_hit as f64 / (vdir_hit + vdir_miss) as f64
    } else {
        0.0
    };
    let _ = writeln!(buf, "    \"hit_rate_pct\": {:.1},", hit_rate);
    let _ = writeln!(buf, "    \"ipc_calls\": {},", ipc);
    let _ = writeln!(buf, "    \"vdir_lookup_ns\": {},", vdir_ns);
    let _ = writeln!(buf, "    \"ipc_roundtrip_ns\": {}", ipc_ns);
    let _ = writeln!(buf, "  }},");

    // Hot paths — top 20 files by cumulative latency
    let hot_paths = collect_hot_paths(20);
    let _ = writeln!(buf, "  \"hot_paths\": [");
    for (i, (path, ns, count)) in hot_paths.iter().enumerate() {
        let avg = if *count > 0 { ns / count } else { 0 };
        let comma = if i + 1 < hot_paths.len() { "," } else { "" };
        // Escape path for JSON (simple: replace \ with \\, " with \")
        let escaped = path.replace('\\', "\\\\").replace('"', "\\\"");
        let _ = writeln!(
            buf,
            "    {{ \"path\": \"{}\", \"total_ns\": {}, \"count\": {}, \"avg_ns\": {} }}{}",
            escaped, ns, count, avg, comma
        );
    }
    let _ = writeln!(buf, "  ],");

    let _ = write!(buf, "}}");

    // Write to file — use raw libc to avoid allocator issues in atexit
    let path = format!("/tmp/vrift-profile-{}.json\0", pid);
    unsafe {
        let fd = libc::open(
            path.as_ptr() as *const libc::c_char,
            libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC,
            0o644,
        );
        if fd >= 0 {
            libc::write(fd, buf.as_ptr() as *const libc::c_void, buf.len());
            libc::close(fd);
        }
    }

    // Summary to stderr
    let avg_ns = if total_calls > 0 {
        total_ns / total_calls
    } else {
        0
    };
    let summary = format!(
        "\n[vrift-profile] PID {} | {:.1}s | init {:.1}ms | {} syscalls (avg {}ns) | VFS {}/{} ({:.0}%) | VDir hit {:.0}% | wrote {}\n",
        pid,
        duration_ms as f64 / 1000.0,
        init_time as f64 / 1_000_000.0,
        total_calls,
        avg_ns,
        handled,
        handled + passthrough,
        handled_pct,
        hit_rate,
        &path[..path.len() - 1],
    );
    unsafe {
        libc::write(2, summary.as_ptr() as *const libc::c_void, summary.len());
    }
}
