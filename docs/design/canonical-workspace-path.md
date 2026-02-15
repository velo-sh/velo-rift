# Canonical Workspace Path Projection

## Problem

Rust/Cargo fingerprints embed **absolute paths** in multiple artifact files:

| File | Format | Example |
|------|--------|---------|
| `deps/*.d` | text | `/Users/alice/proj/target/debug/deps/foo.d: /Users/alice/proj/src/lib.rs` |
| `.fingerprint/*/dep-lib-*` | binary | Contains `/Users/alice/proj/crates/core/src/../../../config/constants.toml` |
| `build/*/output` | text | `cargo:rerun-if-changed=/Users/alice/proj/build.rs` |
| debug info (DWARF) | binary | Source file references for debugger |

This means:
- **Same project, different directory** → cache miss (fingerprints contain old path)
- **Same project, different machine** → cache miss (different home dir)
- **CI/CD** → every job gets a unique checkout path → zero cache hits

sccache solves this at the rustc wrapper level with `SCCACHE_BASEDIRS` path stripping, but it doesn't cache Cargo fingerprints — only `.rlib`/`.rmeta` compilation outputs.

## Solution: Virtual Canonical Root

Use the inception layer's syscall interception to present every project under a canonical path `/vrift_workspace`, regardless of its physical location.

```
Physical:   /Users/alice/rust_source/velo/src/main.rs
Virtual:    /vrift_workspace/src/main.rs          ← what rustc/cargo see

Physical:   /Users/bob/code/velo/src/main.rs
Virtual:    /vrift_workspace/src/main.rs          ← identical path
```

All I/O still happens on the physical filesystem. The shim only translates paths at the syscall boundary.

### Architecture

```
┌──────────────────────────────────────────────────────┐
│  cargo build                                          │
│    sees: /vrift_workspace/src/main.rs                 │
│    writes: /vrift_workspace/target/debug/.fingerprint │
│    getcwd() → /vrift_workspace                        │
├──────────────────────────────────────────────────────┤
│  libvrift_shim.dylib (DYLD_INSERT_LIBRARIES)          │
│                                                        │
│  Inbound:  /vrift_workspace/... → $PHYSICAL_ROOT/...  │
│  Outbound: getcwd/fcntl(F_GETPATH) → /vrift_workspace │
├──────────────────────────────────────────────────────┤
│  Kernel / Real Filesystem                              │
│    actual I/O on: /Users/alice/rust_source/velo/       │
└──────────────────────────────────────────────────────┘
```

### Per-Process Isolation

`/vrift_workspace` is NOT a global mount. Each process tree gets its own mapping via environment variables inherited from the inception session:

```
Terminal 1:
  VRIFT_CANONICAL_ROOT=/vrift_workspace
  VRIFT_PHYSICAL_ROOT=/Users/alice/rust_source/velo
  └─ cargo build → /vrift_workspace = /Users/alice/rust_source/velo

Terminal 2 (simultaneous):
  VRIFT_CANONICAL_ROOT=/vrift_workspace
  VRIFT_PHYSICAL_ROOT=/Users/alice/rust_source/velo-rift
  └─ cargo build → /vrift_workspace = /Users/alice/rust_source/velo-rift
```

No conflict. No root privileges. No FUSE. Each process tree's shim reads its own `VRIFT_PHYSICAL_ROOT` and applies its own mapping.

### Graceful Degradation

If any syscall is NOT intercepted and leaks a physical path into a build artifact:
- The build still works correctly on the same machine/directory
- Only cache sharing fails for that specific crate (fingerprint mismatch)
- **No silent incorrectness** — the result is a cache miss, not a wrong build

## Syscall Interception Details

### Current Inception Layer Status

Based on code audit of `variadic_inception.c`, `interpose.rs`, and all syscall implementations:

| Syscall | Intercepted? | Translation Needed | Status |
|---------|-------------|-------------------|--------|
| `open`, `openat` | ✅ | Inbound | 🟡 Add translation |
| `stat`, `lstat`, `fstatat` | ✅ | Inbound | 🟡 Add translation |
| `access`, `faccessat` | ✅ | Inbound | 🟡 Add translation |
| `mkdir`, `mkdirat`, `rmdir` | ✅ | Inbound | 🟡 Add translation |
| `rename`, `renameat` | ✅ | Inbound | 🟡 Add translation |
| `unlink`, `unlinkat` | ✅ | Inbound | 🟡 Add translation |
| `readlink`, `readlinkat` | ✅ | Both | 🟡 Add translation |
| `realpath` | ✅ | Outbound | ✅ **Already returns VFS virtual paths** |
| `utimes`, `utimensat` | ✅ | Inbound | 🟡 Add translation |
| `getattrlist` | ✅ | Inbound | 🟡 Add translation |
| `clonefileat` | ✅ | Inbound | 🟡 Add translation |
| `fcntl(F_GETPATH)` | ✅ intercepted | Outbound | 🔴 **Currently passthrough — must fix** |
| `getcwd` | ❌ | Outbound | 🔴 **Must add** |
| `chdir` | ❌ | Inbound | 🔴 **Must add** |

> [!IMPORTANT]
> **`fcntl(F_GETPATH)` is the single most critical fix.** Rust's `std::env::current_dir()` on macOS internally calls `fcntl(fd, F_GETPATH)`. Without outbound translation, Cargo will see the physical project root despite all other interceptions. The current implementation is a pure passthrough:
> ```rust
> pub unsafe extern "C" fn velo_fcntl_impl(fd: c_int, cmd: c_int, arg: c_long) -> c_int {
>     libc::fcntl(fd, cmd, arg)  // ← no translation!
> }
> ```

### Path Filtering (Already Implemented)

The C-level fast path (`path_needs_inception` in `variadic_inception.c`) already supports dual-prefix matching:
- `VFS_PREFIX` → set to `/vrift_workspace` at init
- `PROJECT_ROOT` → set to the physical project path

Paths not matching either prefix bypass to raw syscalls (e.g., `/usr/lib`, `/tmp`). **No changes needed here.**

### Inbound Translation (canonical → physical)

Every syscall that accepts a path argument translates `/vrift_workspace/...` → `$PHYSICAL_ROOT/...`:

```c
static const char* translate_inbound(const char* path) {
    if (strncmp(path, "/vrift_workspace/", 17) == 0) {
        // return PHYSICAL_ROOT + (path + 16)
    }
    if (strcmp(path, "/vrift_workspace") == 0) {
        // return PHYSICAL_ROOT
    }
    return path;  // no translation
}
```

### Outbound Translation (physical → canonical)

Syscalls that **return** paths translate `$PHYSICAL_ROOT/...` → `/vrift_workspace/...`:

```c
static const char* translate_outbound(const char* path) {
    size_t root_len = strlen(PHYSICAL_ROOT);
    if (strncmp(path, PHYSICAL_ROOT, root_len) == 0) {
        // return "/vrift_workspace" + (path + root_len)
    }
    return path;
}
```

Outbound syscalls requiring translation:

| Syscall | Notes |
|---------|-------|
| `getcwd` | Must return `/vrift_workspace/...` instead of physical path |
| `fcntl(F_GETPATH)` | Kernel writes physical path to buffer — must rewrite after return |
| `readlink` | If target is under `$PHYSICAL_ROOT`, remap to `/vrift_workspace/...` |
| `realpath` | **Already implemented** — returns VFS virtual paths |
| `getdirentries` / `readdir` | Returns relative names only — no change needed |

### Bootstrap Safety

During inception initialization (`INITIALIZING > 0`), all interceptions bypass to raw syscalls. The shim's own internal `raw_getcwd()` uses `raw_open(".") + raw_fcntl(F_GETPATH)` which bypasses interception. This means:
- Internal code always sees physical paths ✅
- Only application-level calls see `/vrift_workspace` ✅
- No bootstrap chicken-and-egg problem ✅

### Additional Canonical Paths (Phase 4)

Besides the project root, other paths that appear in compilation artifacts also need canonicalization for cross-machine sharing:

| Physical Path | Canonical Path | Reason |
|---------------|---------------|--------|
| `~/rust_source/velo` | `/vrift_workspace` | Project root |
| `~/.cargo/registry/src/...` | `/cargo-registry/...` | Third-party crate sources |
| `~/.rustup/toolchains/...` | `/rustup/...` | Compiler & stdlib paths |

> [!NOTE]
> Cargo registry and rustup paths are the same across builds on the same machine. Canonicalizing them is only needed for cross-machine cache sharing and can be deferred to Phase 4.

## Impact on Compilation Cache

### Before (current)

```
Snapshot on machine A:
  fingerprint contains: /Users/alice/proj/src/lib.rs
  
Restore on machine B:
  source is at: /Users/bob/proj/src/lib.rs
  → mismatch → full recompile
```

### After (with canonical paths)

```
Snapshot on machine A (under inception):
  fingerprint contains: /vrift_workspace/src/lib.rs
  
Restore on machine B (under inception):
  getcwd() returns: /vrift_workspace
  source appears at: /vrift_workspace/src/lib.rs
  → match → FRESH!
```

### Cache Key Changes

The `project_id` should be based on **project content identity** (e.g. hash of `Cargo.lock` + `Cargo.toml` + rustc version + target triple + profile + features), not the physical path. This allows the same project in different directories to share the same cache.

## Edge Cases

### 1. Build Scripts That Probe the Environment

Some build scripts use `std::env::current_dir()` or `env!("CARGO_MANIFEST_DIR")`. These must return canonical paths:

- `current_dir()` → intercepted via `getcwd()` + `fcntl(F_GETPATH)` ✅
- `CARGO_MANIFEST_DIR` → set by Cargo using the path it sees (already `/vrift_workspace/...`) ✅
- `OUT_DIR` → set by Cargo, will be `/vrift_workspace/target/debug/build/...` ✅

### 2. Symlinks and `realpath`

If the project directory contains symlinks, `realpath()` might resolve through the physical path. The inception layer **already intercepts `realpath`** and returns VFS virtual paths (implemented in `syscalls/path.rs`). ✅

> [!NOTE]
> The shim should `realpath($PHYSICAL_ROOT)` at init time and match against both the raw and resolved physical root, in case the path contains symlinks.

### 3. Paths Escaping `/vrift_workspace`

Build scripts might reference files outside the project (e.g. system libraries at `/usr/lib`). These paths are NOT remapped — only paths under `$PHYSICAL_ROOT` are translated. This is correct behavior.

### 4. Error Messages and IDE Integration

rustc error messages will contain `/vrift_workspace/src/main.rs:42:5`. For IDE integration:

**Solution**: Pass `--remap-path-prefix=/vrift_workspace=$PHYSICAL_ROOT` to rustc via `RUSTFLAGS`. This makes:
- **Error messages** → physical paths (IDE can jump to file) ✅
- **Fingerprints/dep-info** → canonical paths (cache-portable) ✅
- **`file!()` / `panic!()` macros** → physical paths (acceptable, mainly for logging) ✅

> [!IMPORTANT]
> **IDE / rust-analyzer must be launched from within the inception shell.** Otherwise, `cargo metadata` output will contain `/vrift_workspace/...` paths that the IDE cannot resolve. Running the IDE under inception ensures it sees the same virtual filesystem as cargo.

### 5. Path Conflicts

The canonical path `/vrift_workspace` is chosen to avoid collisions with real directories. It does not need to physically exist — the shim intercepts all access before it reaches the kernel.

### 6. Incremental Compilation

The `target/debug/incremental/` directory contains binary data with embedded paths. Since these paths are written by rustc **while running under inception**, they are born with canonical `/vrift_workspace/...` paths. No special handling needed — incremental cache is safe to cache and restore. ✅

## Implementation Plan

### Step 1: `fcntl(F_GETPATH)` outbound translation [~5 lines, highest impact]

Fix the existing passthrough in `velo_fcntl_impl` to detect `F_GETPATH` and apply outbound translation:

```rust
pub unsafe extern "C" fn velo_fcntl_impl(fd: c_int, cmd: c_int, arg: c_long) -> c_int {
    let ret = libc::fcntl(fd, cmd, arg);
    if ret == 0 && cmd == libc::F_GETPATH {
        // arg is a *mut c_char buffer — translate physical → canonical
        translate_outbound(arg as *mut c_char);
    }
    ret
}
```

### Step 2: `chdir()` inbound translation [~20 lines]

Add `chdir` interception. Translate `/vrift_workspace/...` → `$PHYSICAL_ROOT/...` before passing to kernel.

### Step 3: `getcwd()` outbound translation [~30 lines]

Intercept `getcwd()`. After the real syscall returns, translate `$PHYSICAL_ROOT/...` → `/vrift_workspace/...` in the output buffer.

### Step 4: Set `/vrift_workspace` as `VFS_PREFIX` [~3 lines]

During inception init, call `set_vfs_prefix("/vrift_workspace")` so that `path_needs_inception()` routes canonical paths to Rust for translation.

### Step 5: Inbound translation in existing syscalls [~50 lines]

Add `translate_inbound()` call at the entry point of each already-intercepted syscall (`open`, `stat`, `access`, etc.). This is a uniform pattern — same 2-line change in each function.

### Step 6: Cache key update (Phase 3)

Change `compute_project_id()` to use content-based hash instead of path-based hash.

### Step 7: Additional canonical paths (Phase 4)

Add cargo registry and rustup canonicalization for cross-machine sharing.

## Testing

1. **Same project, two directories**: Clone velo to `/tmp/velo2`, build both under inception, verify cache hit
2. **Snapshot in dir A, restore in dir B**: Verify `cargo build` is FRESH
3. **Build script correctness**: Verify `CARGO_MANIFEST_DIR`, `OUT_DIR` are canonical
4. **Error messages**: Verify IDE can jump to errors (physical paths in output)
5. **Concurrent projects**: Two terminals, different projects, both `/vrift_workspace`, no interference
6. **`std::env::current_dir()` test**: Verify it returns `/vrift_workspace` under inception
7. **Incremental compilation**: Verify incremental cache works after restore
