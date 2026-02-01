# RFC-0046: Deterministic Metadata Isolation & Write-Shadowing

## Status
**Draft**

## Summary
Formalizes the separation of control plane (metadata) and data plane (VFS/CAS) to support real-time ingestion of build artifacts while preventing architectural recursion/deadlocks.

## Problem Statement
Current Velo Rift implementations face a potential "Metadata Recursion" hazard:
1. If metadata (e.g., manifests) is located inside the project directory, a Shim-monitored process (like `ls` or `cargo`) might attempt to virtualize the metadata files themselves.
2. Real-time ingestion of build artifacts into CAS requires a safe "buffer" area to avoid deadlocking the filesystem during hash calculation and IPC.

## Proposed Solution

### 1. The `.vrift/` Control Plane
Every Velo Rift project MUST utilize a hidden directory `.vrift/` at the project root for coordination.

**Deterministic Rule**: The `.vrift/` directory and all its contents are **EXCLUDED** from VFS virtualization.

- **Shim Enforcement**: The `psfs_applicable` function must explicitly return `false` for any path containing `/.vrift/`.
- **Contents**:
    - `manifest.lmdb`: The primary project metadata.
    - `hot_cache.bin`: Memory-mapped acceleration data.
    - `shadow/`: A local staging area for newly written files.

### 2. Global CAS Root Exclusion
The global CAS directory (defined by `VRIFT_CAS_ROOT`) must be treated as a "System-Level Forbidden Zone" for the Shim.

**Safety Invariant**: The Shim MUST NOT intercept any syscall where the resolved path is within the `VRIFT_CAS_ROOT`.

- **Rationale**: When `vriftd` or `vrift ingest` writes to the CAS, these operations must be direct. If intercepted, they would trigger another IPC to the daemon, creating an infinite loop.
- **Enforcement**: Upon initialization, the Shim reads `VRIFT_CAS_ROOT` and adds it to its internal path-exclusion list.

### 3. Write-Shadowing Mechanism
To support multi-project shared build artifacts (e.g., shared `target/` in CAS):

1. **Intercepted Write**: A process writes to a virtual path `/vrift/bin/app`.
2. **Shadow Redirection**: The Shim redirects the syscall to the host path `.vrift/shadow/bin/app`.
3. **Async Promotion**:
    - Upon `close()`, the Shim notifies `vriftd`.
    - `vriftd` (running with `VRIFT_ENABLED=0`) hashes the shadow file.
    - `vriftd` moves the file to the global `VRIFT_CAS_ROOT`.
    - `vriftd` updates the LMDB manifest to point the virtual path to the new hash.

### 3. Recursion Prevention (The Interlock)
To ensure system stability, the following "Interlock" rules are established:

| Component | Responsibility |
| :--- | :--- |
| **Shim** | Detects virtual paths. Hard-excludes `.vrift/` and `VRIFT_CAS_ROOT`. |
| **Daemon** | Runs with Velo Rift disabled (`VRIFT_ENABLED=0`) to avoid self-interception. |
| **IPC** | Uses explicit file descriptors (sockets) rather than path-based coordination to avoid interception loops. |

## Implementation Roadmap

### Phase 1: Metadata & CAS Sanitization
- Relocate all manifests and logs into `.vrift/`.
- Update `lib.rs` to include the `/.vrift/` and `VRIFT_CAS_ROOT` exclusion rules in `psfs_applicable`.

### Phase 2: Write-Shadowing (Target: Phase 7/9)
- Implement `Redirected-O_WRONLY` support.
- Implement background "Shadow to CAS" promotion in the daemon.

## Security & Performance
- **Isolation**: Metadata is physically separated from the projected view, preventing accidental deletion via `rm -rf /vrift/*`.
- **Zero-Latency Writes**: Redirection to a local shadow directory ensures no IPC overhead during critical `write()` loops.
