# RFC-0044: Velo Native Runtime Architecture

> **Status**: DRAFT  
> **Revision**: 0.1.0  
> **Author**: Velo Architect  
> **Date**: 2026-01-29  
> **Dependencies**: [RFC-0043](0043-velo-vfs-cas-layer.md) (VeloVFS)

---

## 1. Executive Summary

Traditional runtime environments treat dependencies as a collection of files in a directory (mutable state). RFC-0044 proposes a paradigm shift to **Velo Native**, where the runtime environment is treated as a **Versioned Repository**.

We introduce a split-brain architecture:
1.  **The Body (Data)**: A raw, flat, deduplicated **Content-Addressable Storage (CAS)** system.
2.  **The Brain (Metadata)**: A **Git-compatible Object Database** running entirely in memory (via LMDB), managing the relationships between files.

This architecture treats "Package Installation" not as file copying, but as **Pointer Manipulation**, enabling O(1) installation, instant snapshots ("The Snapshot Compass"), and zero-cost multi-tenancy.

---

## 2. Core Philosophy: "Derived Data"

In Velo Native, the runtime environment view is **ephemeral and derived**.

*   **Source of Truth**: 
    1.  The user's intent (`uv.lock` / `requirements.txt`).
    2.  The global pool of immutable CAS Blobs (The "Warehouse").
*   **The Runtime**: A temporary projection of the Source of Truth, assembled instantly on demand.

> **The "Git" Analogy**: 
> Velo does not "install" packages. Velo "commits" a new directory tree structure that happens to contain the files required by the package.

---

## 3. Architecture Stack

### Layer 1: The Body (Raw CAS Store)
**Physical Storage**: `Disk (NVMe)` or `S3`
*   **Content**: Raw, uncompressed binaries (`.so`, `.py`, `.dll`).
*   **Addressing**: BLAKE3 Hash.
*   **Format**: Flat structure (e.g., `/var/velo/cas/a1/b2/a1b2c3...`).
*   **Property**: **Strictly Immutable**. Once written, a blob never changes.

### Layer 2: The Brain (Git-Over-LMDB)
**Metadata Storage**: `Memory Mapped DB (LMDB)`
*   **Content**: Git Objects (Commits, Trees) and References (Branches/Tags).
*   **Addressing**: SHA-1 or BLAKE3 Hash of the object content.
*   **Innovation**: **"Pointer Blobs"**.
    *   Standard Git Blob: Stores file content.
    *   Velo Git Blob: Stores **only the pointer** to Layer 1.
    *   *Structure*: `blob <size>\0{"cas_ref": "blake3:a1b2..."}`.
*   **Performance**: Operating on LMDB allows us to commit, branch, and checkout in microseconds.

### Layer 3: The Interface (VeloVFS Shim)
**Runtime**: `LD_PRELOAD` / `FUSE`
*   **Role**: The illusionist.
*   **Mechanism**: Intercepts `open("/app/lib/foo.py")`.
*   **flow**:
    1.  Lookup current `HEAD` Commit in Layer 2.
    2.  Traverse Git Tree in Memory to find `foo.py` -> Get Pointer Blob.
    3.  Extract `cas_ref`.
    4.  `mmap` the physical file from Layer 1.
    5.  Return File Descriptor to user.

---

## 4. Workflows

### 4.1 Ingest (The "Compiler")
Velo integrates `uv` as a frontend "Compiler". `uv` resolves dependency graphs; Velo materializes them.

1.  **User**: `uv pip install numpy`
2.  **Intercept**: Velo captures the intent.
3.  **Resolve**: `uv` calculates the need for `numpy-1.26.0`.
4.  **Fetch**: Velo checks Layer 1. If missing, download and write to CAS.
5.  **Commit**: 
    *   Create Git Tree objects for `/site-packages/numpy/...`.
    *   Create a new Commit Object: `Parent: HEAD`, `Message: "Install numpy 1.26.0"`.
    *   Update `HEAD` ref.
6.  **Result**: The file system view updates instantly.

### 4.2 The "Snapshot Compass" (Time Travel)
Since the runtime state is a Git Reference:

*   **Undo**: `velo revert` -> Move `HEAD` to `HEAD^`. (Instant).
*   **Fork**: `velo branch test-v2` -> Create new Ref pointing to current Commit. (Zero Copy).
*   **Bisect**: Automated debugging by moving the pointer through commit history to find breaking changes.

---

## 5. Persistence & Recovery

To achieve "Instant Restart":

*   **Persistence**: Layer 2 (LMDB) is memory-mapped to disk.
    *   Reads are RAM-speed.
    *   Writes are asynchronously flushed to disk (OS Page Cache).
    *   **Crash Safety**: LMDB ensures database integrity ("No Partial Writes").
*   **Rebuild**: If LMDB is corrupted or deleted:
    1.  Velo scans valid `uv.lock` files from active tenants.
    2.  Velo verifies CAS Blobs in Layer 1.
    3.  Velo **re-computes** the Git Trees and Commits.
    4.  The "Brain" is regenerated from the "Body".

---

## 6. Structure Definitions

### 6.1 `velo.lock` (The Bytecode)
The bridge between `uv.lock` (Intent) and Velo Runtime (Execution).

```json
{
  "meta": {
    "generated_at": 1706448000,
    "uv_hash": "sha256:..."
  },
  "mounts": {
    "/usr/bin/python": "tree:d4e5f6...",
    "/site-packages": "tree:a1b2c3..."
  }
}
```

### 6.2 LMDB Schema (K-V)
*   **Objects DB**:
    *   Key: `ObjectHash`
    *   Value: `[TypeByte][Content]`
*   **Refs DB**:
    *   Key: `refs/tenants/{id}/HEAD`
    *   Value: `CommitHash`

---

## 7. Conclusion

RFC-0044 defines a runtime that is **Stateless yet Persistent**. By decoupling content (CAS) from structure (Git-Memory), we achieve:
1.  **Infinite Deduplication**: 1000 tenants share the same physical bytes.
2.  **Instant Snapshots**: O(1) branching and rollback.
3.  **Crash Resilience**: 0-second restart time via mmap.
