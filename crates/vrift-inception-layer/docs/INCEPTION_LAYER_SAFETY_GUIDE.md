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
| `std::env::var()` | Internal Mutex/TLS | `libc::getenv()` ✅ |

## Golden Rules

### 1. Use `passthrough_if_init!` Macro (Pattern 2648/2649)

Every shim entry point MUST use this macro:

```rust
#[no_mangle]
pub unsafe extern "C" fn my_shim(arg: i32) -> i32 {
    let real = /* get real function */;
    passthrough_if_init!(real, arg);  // Early return if TLS unsafe
    
    // ... VFS logic here (only runs when TLS is safe)
}
```

### 2. INITIALIZING State Check: Use `>= 2`, NOT `!= 0`

```
State 2 (Early-Init) → TLS unmapped, MUST passthrough
State 3 (Busy)       → ShimState initializing, MUST passthrough
State 1 (Rust-Init)  → C constructor ran, TLS SAFE ✅
State 0 (Ready)      → Full VFS active, TLS SAFE ✅
```

❌ **WRONG**: `if INITIALIZING.load(Relaxed) != 0 { return real(); }`
✅ **CORRECT**: `if INITIALIZING.load(Relaxed) >= 2 { return real(); }`

### 3. Environment Variables: Use `libc::getenv()`

❌ **WRONG** (triggers TLS):
```rust
if let Ok(val) = std::env::var("VRIFT_VFS_PREFIX") { ... }
```

✅ **CORRECT** (TLS-free):
```rust
let env_name = b"VRIFT_VFS_PREFIX\0";
let ptr = libc::getenv(env_name.as_ptr() as *const c_char);
if !ptr.is_null() {
    let val = CStr::from_ptr(ptr).to_str().ok();
}
```

### 4. Never modify init-path code without testing

```bash
# Always test after changes to state.rs, lib.rs, or any shim entry point:
DYLD_INSERT_LIBRARIES=target/debug/libvrift_shim.dylib /tmp/test_minimal
```

If the process hangs, your change introduced a TLS trigger.

### 5. Check for hidden dependencies

```bash
# Audit TLS symbols before merging:
nm target/debug/libvrift_shim.dylib | grep -i tlv
```

### 6. Framework pollution check

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
| `macros.rs` | 🟡 CAUTION | `passthrough_if_init!` must be correct |
| `syscalls/*.rs` entry | 🟡 CAUTION | Must use `passthrough_if_init!` |
| Other internal code | 🟢 SAFE | Only called after initialization |

## Reference

- Full forensic analysis: `~/.gemini/antigravity/knowledge/vrift_comprehensive_reference/artifacts/qa/shim_init_hang_forensics.md`
- Pattern 2648: Bare-Metal Init
- Pattern 2649: Non-Blocking Passthrough
