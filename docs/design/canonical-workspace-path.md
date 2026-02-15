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

Use the inception layer's syscall interception to present every project under a canonical path `/workspace`, regardless of its physical location.

```
Physical:   /Users/alice/rust_source/velo/src/main.rs
Virtual:    /workspace/src/main.rs          ← what rustc/cargo see

Physical:   /Users/bob/code/velo/src/main.rs
Virtual:    /workspace/src/main.rs          ← identical path
```

All I/O still happens on the physical filesystem. The shim only translates paths at the syscall boundary.

### Architecture

```
┌─────────────────────────────────────────────────┐
│  cargo build                                     │
│    sees: /workspace/src/main.rs                  │
│    writes: /workspace/target/debug/.fingerprint/ │
│    getcwd() → /workspace                         │
├─────────────────────────────────────────────────┤
│  libvrift_shim.dylib (DYLD_INSERT_LIBRARIES)     │
│                                                   │
│  Inbound:  /workspace/... → $PHYSICAL_ROOT/...   │
│  Outbound: getcwd() → /workspace/...             │
├─────────────────────────────────────────────────┤
│  Kernel / Real Filesystem                         │
│    actual I/O on: /Users/alice/rust_source/velo/  │
└─────────────────────────────────────────────────┘
```

### Per-Process Isolation

`/workspace` is NOT a global mount. Each process tree gets its own mapping via environment variables inherited from the inception session:

```
Terminal 1:
  VRIFT_CANONICAL_ROOT=/workspace
  VRIFT_PHYSICAL_ROOT=/Users/alice/rust_source/velo
  └─ cargo build → /workspace = /Users/alice/rust_source/velo

Terminal 2 (simultaneous):
  VRIFT_CANONICAL_ROOT=/workspace
  VRIFT_PHYSICAL_ROOT=/Users/alice/rust_source/velo-rift
  └─ cargo build → /workspace = /Users/alice/rust_source/velo-rift
```

No conflict. No root privileges. No FUSE. Each process tree's shim reads its own `VRIFT_PHYSICAL_ROOT` and applies its own mapping.

## Syscall Interception Details

### Inbound Path Translation (canonical → physical)

Every syscall that accepts a path argument must translate `/workspace/...` → `$PHYSICAL_ROOT/...`:

| Syscall | Notes |
|---------|-------|
| `open`, `openat` | Already intercepted by inception layer |
| `stat`, `lstat`, `fstatat` | Already intercepted |
| `access`, `faccessat` | Already intercepted |
| `mkdir`, `mkdirat` | Already intercepted |
| `rename`, `renameat` | Already intercepted |
| `unlink`, `unlinkat` | Already intercepted |
| `readlink`, `readlinkat` | Must translate both input and output |
| `chdir` | `chdir("/workspace/src")` → `chdir("$PHYSICAL_ROOT/src")` |
| `execve` | Translate argv[0] and the binary path if under `/workspace` |

### Outbound Path Translation (physical → canonical)

Syscalls that **return** paths must translate `$PHYSICAL_ROOT/...` → `/workspace/...`:

| Syscall | Notes |
|---------|-------|
| `getcwd` | Must return `/workspace/...` instead of physical path |
| `readlink` | If target is under `$PHYSICAL_ROOT`, remap to `/workspace/...` |
| `realpath` | If resolved path is under `$PHYSICAL_ROOT`, remap |
| `getdirentries` / `readdir` | Usually relative names, no change needed |

### Additional Canonical Paths

Besides the project root, other paths that appear in compilation artifacts also need canonicalization:

| Physical Path | Canonical Path | Reason |
|---------------|---------------|--------|
| `~/rust_source/velo` | `/workspace` | Project root |
| `~/.cargo/registry/src/...` | `/cargo-registry/...` | Third-party crate sources |
| `~/.rustup/toolchains/...` | `/rustup/...` | Compiler & stdlib paths |

> [!NOTE]
> Cargo registry and rustup paths are usually the same across builds on the same machine but differ across machines. Canonicalizing them enables cross-machine cache sharing.

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
  fingerprint contains: /workspace/src/lib.rs
  
Restore on machine B (under inception):
  getcwd() returns: /workspace
  source appears at: /workspace/src/lib.rs
  → match → FRESH!
```

### Cache Key Changes

The `project_id` should be based on **project content identity** (e.g. hash of `Cargo.lock` + `Cargo.toml` + rustc version), not the physical path. This allows the same project in different directories to share the same cache.

## Edge Cases

### 1. Build Scripts That Probe the Environment

Some build scripts use `std::env::current_dir()` or `env!("CARGO_MANIFEST_DIR")`. These must also return canonical paths:

- `current_dir()` → intercepted via `getcwd()`
- `CARGO_MANIFEST_DIR` → set by Cargo using the path it sees (already `/workspace/...`)
- `OUT_DIR` → set by Cargo, will be `/workspace/target/debug/build/...`

### 2. Symlinks and `realpath`

If the project directory contains symlinks, `realpath()` might resolve through the physical path. The shim must intercept `realpath` and re-map the result.

### 3. Paths Escaping `/workspace`

Build scripts might reference files outside the project (e.g. system libraries at `/usr/lib`). These paths are NOT remapped — only paths under `$PHYSICAL_ROOT` are translated.

### 4. Error Messages and IDE Integration

rustc error messages will contain `/workspace/src/main.rs:42:5`. For IDE integration (jump-to-error), two options:

1. **Reverse-map in output layer**: inception wraps stdout/stderr and replaces `/workspace/...` → physical path
2. **`--remap-path-prefix`**: Pass `--remap-path-prefix=/workspace=$PHYSICAL_ROOT` to rustc via `RUSTFLAGS`, so error messages use physical paths while internal fingerprints use canonical paths

Option 2 is cleaner — it only affects human-readable output, not fingerprints.

### 5. `/workspace` Conflicts with Real Paths

If `/workspace` actually exists on the system, it would conflict. Mitigation:
- Use a more unique canonical path like `/.vrift/workspace` or `/dev/vrift/workspace`
- The path doesn't need to physically exist — the shim intercepts all access before it reaches the kernel

## Implementation Plan

### Phase 1: Shim Path Translation

Add two translation functions to the inception shim:

```c
// Canonical → Physical (for syscall inputs)
static const char* translate_inbound(const char* path) {
    if (strncmp(path, "/workspace/", 11) == 0) {
        // return PHYSICAL_ROOT + (path + 10)
    }
    if (strcmp(path, "/workspace") == 0) {
        // return PHYSICAL_ROOT
    }
    return path;  // no translation
}

// Physical → Canonical (for syscall outputs)
static const char* translate_outbound(const char* path) {
    size_t root_len = strlen(PHYSICAL_ROOT);
    if (strncmp(path, PHYSICAL_ROOT, root_len) == 0) {
        // return "/workspace" + (path + root_len)
    }
    return path;
}
```

### Phase 2: getcwd / chdir

Intercept `getcwd()` to return `/workspace/...` when the real cwd is under `$PHYSICAL_ROOT`. Track the logical cwd to handle relative paths correctly.

### Phase 3: Cache Key Update

Change `compute_project_id()` to use a content-based hash instead of path-based hash, enabling cross-directory cache sharing.

### Phase 4: Additional Canonical Paths

Add cargo registry and rustup canonicalization for cross-machine sharing.

## Testing

1. **Same project, two directories**: Clone velo to `/tmp/velo2`, build both under inception, verify cache hit
2. **Snapshot in dir A, restore in dir B**: Verify `cargo build` is FRESH
3. **Build script correctness**: Verify `CARGO_MANIFEST_DIR`, `OUT_DIR` are canonical
4. **Error messages**: Verify IDE can jump to errors (physical paths in output)
5. **Concurrent projects**: Two terminals, different projects, both `/workspace`, no interference
