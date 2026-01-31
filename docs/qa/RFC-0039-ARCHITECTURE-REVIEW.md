# Architecture Review: RFC-0039 Transparent Virtual Projection

## 1. Executive Summary

RFC-0039 establishes a robust foundation for VFS-based environment projection. The "Tiered Asset Model" cleverly balances performance (Tier-1 zero-copy symlinks) and safety (Tier-2 hardlinks with BBW). However, current implementation gaps in the LD_PRELOAD shim layer pose a risk of metadata desync, and the absence of system-level immutability for Tier-2 assets weakens the "Iron Law" protection.

## 2. Identified Design & Implementation Gaps

### 2.1 Manifest Desync in LD_PRELOAD Shim (CRITICAL)
**Issue**: The `velo-shim` implements Break-Before-Write (BBW) and computes a new hash on `close()`, but it **cannot update the persistent Manifest**.
**Impact**: Modified files will stay persistent in the CAS, but the Manifest will still point to the old hash. Future runs or other processes using the same Manifest will see the old content, violating **Invariant P0-b**.
**Recommendation**: 
- Introduce a **Manifest Delta Server** or use the existing `vrift-daemon`. 
- The shim should signal the daemon via IPC on `close()` to commit the new hash to the Delta Layer.

### 2.2 Tier-2 Hardlink Safety Gap
**Issue**: Tier-2 assets use `hard_link` but do NOT apply `chattr +i` (reserved for Tier-1).
**Impact**: While the shim intercepts writes, a program NOT protected by the shim (or a direct `echo "data" > file`) can modify the inode directly. Since Tier-2 shares the inode with the CAS, this **corrupts the global CAS**, violating **Invariant P0-a**.
**Recommendation**:
- Tier-2 assets MUST also be `chmod 444` at the source path.
- Consider utilizing **OverlayFS (RFC-0043)** for Tier-2 rather than simple hardlinks to provide a more robust "copy-up" mechanism at the kernel level.

### 2.3 Locking Granularity
**Issue**: `flock(LOCK_SH)` is acquired on the source file during ingest.
**Impact**: On some networked filesystems (NFS) or older kernels, `flock` may be advisory or have high latency.
**Recommendation**:
- Implement a fallback to `.lock` sidecar files if `flock` fails with `ENOTSUP`.

## 3. Scalability Analysis

- **LMDB Performance**: The choice of LMDB for the Manifest is excellent for O(1) reads and crash-safety.
- **Rayon Ingest**: Parallel ingest at 14k files/sec is state-of-the-art.
- **Memory Pressure**: The shim uses thread-local `FD_MAP`. For processes with thousands of open files (e.g., a massive build), this could increase memory overhead. Tracking only "modified" files should stay in `FD_MAP` to minimize impact.

## 4. Final Verdict

**Status**: 🟢 **Solid Foundation / 🟡 Implementation Warning**

The design is sound, but the **Manifest commit path** must be implemented to make the VFS stateful across process boundaries. Without this, RFC-0039 remains a "read-mostly" projection system rather than a true "environment provider."
