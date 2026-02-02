# Vrift-Shim Safety Guide: Avoiding TLS Deadlock

> ⚠️ **CRITICAL**: This shim runs during macOS `dyld` bootstrap. Any Rust TLS access before `TLS_READY` will deadlock the process.

## The Problem

When loaded via `DYLD_INSERT_LIBRARIES`, the shim's code executes **before** dyld has finished setting up Thread Local Storage (TLS). Many Rust standard library features implicitly use TLS:

| Feature | TLS Trigger | Alternative |
|---------|-------------|-------------|
| `String` / `Cow<str>` | Allocator | `*const libc::c_char` + `libc::malloc` |
| `HashMap` | `RandomState` (KEYS TLS) | `rustc_hash::FxHashMap` or raw pointers |
| `std::sync::Mutex` | `DTORS` TLS for cleanup | `spin::Mutex` (but see warning below) |
| `std::thread::current()` | `ID` / `CURRENT` TLS | Avoid entirely |
| `println!` / `eprintln!` | `OUTPUT_CAPTURE` TLS | `libc::write(2, ...)` |
| `panic!` | `LOCAL_PANIC_COUNT` TLS | `libc::abort()` |

## Golden Rules

### 1. Never modify init-path code without testing

```bash
# Always test after changes to state.rs, lib.rs, or any shim entry point:
DYLD_INSERT_LIBRARIES=target/debug/libvrift_shim.dylib /tmp/test_minimal
```

If the process hangs, your change introduced a TLS trigger.

### 2. Check for hidden dependencies

```bash
# Audit TLS symbols before merging:
nm target/debug/libvrift_shim.dylib | grep -i tlv
```

Expected output should show only these 6 internal Rust TLS (which are safe because they're only accessed *after* `TLS_READY`):
- `OUTPUT_CAPTURE`, `DTORS`, `ID`, `CURRENT`, `LOCAL_PANIC_COUNT`, `REGISTERED`

### 3. Do NOT replace std types without careful analysis

❌ **What I tried (caused regression):**
```rust
// Replacing std::sync::Mutex with spin::Mutex broke lazy initialization
use spin::Mutex;  // Different API: .lock() returns guard directly, not Result
```

The issue wasn't just API difference—it was that the initialization order changed.

### 4. Use the three-state initialization guard

```
State 2 (Early-Init) → TLS unmapped, use raw syscalls only
State 1 (Rust-Init)  → Shim initializing, bounded spin-wait
State 0 (Ready)      → Full VFS active
```

All shim entry points must check `TLS_READY` before using any Rust features.

### 5. Framework pollution check

```bash
# Ensure no hidden framework linkage:
otool -L target/debug/libvrift_shim.dylib

# MUST NOT show: CoreFoundation, CoreServices, etc.
# Should only show: libSystem.B.dylib, libiconv.2.dylib
```

## File Safety Classification

| File | Safety Level | Notes |
|------|--------------|-------|
| `lib.rs` SET_READY | 🔴 CRITICAL | Constructor runs during dyld bootstrap |
| `state.rs` init() | 🔴 CRITICAL | Must use only libc, no Rust allocator |
| `syscalls/*.rs` entry | 🟡 CAUTION | Must check TLS_READY before Rust code |
| Other internal code | 🟢 SAFE | Only called after initialization |

## Reference

- Full forensic analysis: `~/.gemini/antigravity/knowledge/vrift_comprehensive_reference/artifacts/qa/shim_init_hang_forensics.md`
- Pattern 2648: Bare-Metal Init
- Pattern 2649: Non-Blocking Passthrough
