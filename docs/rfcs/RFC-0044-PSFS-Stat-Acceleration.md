# RFC-0044: PSFS Stat Acceleration Architecture

**Status**: Accepted  
**Created**: 2026-02-01  
**Author**: Velo-Rift Core Team

## Summary

This RFC establishes the **PSFS (Provably-Side-Effect-Free Stat)** design principle for VFS-level stat acceleration. It defines the architectural boundary between accelerable "Hot Stat" paths and passthrough "Cold Stat" paths.

## Motivation

### The Problem

When intercepting `stat`/`fstat`/`lstat` at the syscall level via `DYLD_INSERT_LIBRARIES` (macOS) or `LD_PRELOAD` (Linux), any "smart logic" in the shim triggers **recursive deadlock**:

```
stat → VFS lookup → path normalize → alloc → mmap → stat (DEADLOCK)
```

This is not a bug—it's a **structural conflict** between:
- syscall-level interception
- Runtime operations that internally call stat (Python import, Cargo dependency resolution, npm module lookup)

### Root Cause Analysis

```
dyld __malloc_init → fstat_shim → ShimState::init → malloc (DEADLOCK)
```

During early process initialization, `dyld` calls `fstat`/`close` before `malloc` is ready. Any shim logic that allocates memory will deadlock.

### Key Insight

> **stat is control flow, not I/O.**

In Python/Cargo/npm/Go, stat serves as "existence probe" for import graphs and dependency DAGs. It's not file I/O—it's a control flow primitive. Intercepting it with "intelligent" logic at the syscall layer is fundamentally unsafe.

## Design Principle

### Core Rule

> **VFS does not infer intent. It consumes capability.**
> 
> Capability = "Is this path in VFS domain with resident metadata?"
> 
> Deadlocks eliminated **by construction**, not by heuristics.

### The PSFS Model

#### Hot Stat (Accelerable)

Only for paths that satisfy ALL conditions:

| Condition | Requirement |
|-----------|-------------|
| Domain | Path is in VFS namespace (`/vrift/*`, CAS mount, overlay) |
| Metadata | Already resident in read-only memory |
| Immutable | mtime/inode/size are fixed |
| Implementation | async-signal-safe superset |

Hard constraints:
- ❌ No alloc (`malloc` = forbidden)
- ❌ No lock (`mutex`/`futex` = forbidden)
- ❌ No log (absolutely forbidden)
- ❌ No syscall (including stat)
- ✅ O(1) constant time
- ✅ Read-only (no cache writes)

#### Cold Stat (Everything Else)

Pure transparent passthrough to `real_stat()`:
- No logging
- No caching
- No judgment

### Implementation Pattern

```rust
fn stat(path: *const c_char, buf: *mut stat) -> c_int {
    // Skip during early init (malloc not ready)
    if SHIM_STATE.load(Ordering::Acquire).is_null() {
        return real_stat(path, buf);
    }
    
    // Hot path: VFS domain with resident metadata
    if psfs_applicable(path) {
        if psfs_lookup(path, buf) {
            return 0;
        }
    }
    
    // Cold path: pure passthrough
    real_stat(path, buf)
}
```

## VFS Positioning

### What Velo-Rift VFS IS

> "Zero-side-effect acceleration for paths it controls."

Accelerable domains:
- `/vrift/*` (VFS prefix)
- CAS mounts
- Overlay namespaces

### What Velo-Rift VFS is NOT

> "Universal stat accelerator."

All paths outside VFS domain → transparent passthrough.

## Implications for Tool Support

| Tool | stat Purpose | VFS Strategy |
|------|--------------|--------------|
| Python | import probe | Accelerate if in VFS vendored packages |
| Cargo | dependency DAG | Accelerate if in VFS build cache |
| npm | module resolution | Accelerate if in VFS node_modules |
| Go | package discovery | Accelerate if in VFS GOPATH |

For paths outside VFS domain, these tools use `real_stat()` with zero overhead.

## Why "Intent Detection" is Impossible

An independent VFS layer cannot:
- Parse call stacks (requires alloc + dlopen)
- Match path patterns (requires string ops)
- Use frequency heuristics (first call already deadlocks)
- Detect syscall patterns (mmap/stat cross-recursion)

The only safe approach: **physical domain membership**, not semantic inference.

## Compatibility

This design is compatible with:
- macOS `DYLD_INSERT_LIBRARIES` with Mach-O `__interpose`
- Linux `LD_PRELOAD` with `dlsym(RTLD_NEXT)`
- Any process lifecycle (including early dyld init)

## References

- Commit `8d420cd`: fix(shim): resolve macOS dyld __malloc_init recursion deadlock
- `tests/poc/test_issue1_recursion_deadlock.sh`: Verification test
