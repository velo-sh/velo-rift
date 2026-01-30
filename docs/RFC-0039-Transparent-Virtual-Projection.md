# RFC-0039: Transparent Virtual Projection & In-place Transmutation

## 1. Status
**Draft**

## 2. Context & Objectives
Velo Rift™ aims to eliminate the friction between "Project Content" and "Disk Storage." This RFC proposes a **Transparent Projection Model** where the VFS layer replaces heavy-duty physical directories (e.g., `node_modules`, `target`, `.venv`) with a dynamic virtual lens. The environment is intended to be **long-lived**, becoming the primary state of the workspace rather than a transient execution context.

## 3. Core Concepts

### 3.1 Active Projection
- **Action**: `vrift active`
- **Function**: Transitions the workspace into a persistent **Projected State**. Velo Rift™ acts as a "Live Lens" over the project directory.
- **Dependency Replacement**: Folders like `node_modules` or `target` are projected from the CAS. They appear physically present but are managed virtual assets.

### 3.2 Live Ingest
Velo Rift™ automates the existing `ingest` logic:
- **Trigger**: When a process finishes writing a file (`close()`), Velo performs a **Live Ingest**.
- **Efficiency**: The file is hashed and either moved or hardlinked into the CAS.
- **SSOT**: The Manifest is updated immediately, ensuring the virtual view is always in sync.

### 3.3 Dimensional Ingest (ABI Tags)
To handle multi-version binaries:
- The `ingest` process considers the **ABI_Context** for binary files (`.so`, `.dylib`).
- This prevents collisions between different versions at the same path.

## 4. Operational Strategies
Velo Rift™ provides two levels of transparency to ensure "no-feeling rollback" (无感回退):

### 4.1 Level 1: Hardlinked Shadowing (Default)
- **Mechanism**: `Live Ingest` + `Hardlink`. 
- **User Experience**: Files are moved to CAS but a hardlink is immediately created at the original path. 
- **Rollback**: Instant. Since the physical inode remains at the path, deactivating the virtual layer has zero effect on file availability.

### 4.2 Level 2: Virtual Projection (Aggressive)
- **Mechanism**: `Live Ingest` + `Move`.
- **User Experience**: The physical file is moved to CAS and removed from the project directory. It remains visible only through the Velo lens.
- **Rollback**: Requires Restoration. Deactivating the layer requires Velo to restore physical files from the CAS.

## 5. Implementation Notes
- **Persistent State**: `vrift active` creates a long-lived Session.
- **ABI Continuity**: The Session persists the **ABI_Context**, ensuring that a long-running development environment remains binary-consistent.
- **Shim Performance**: Shadow capturing avoids the latency of synchronous hashing during small `write()` calls by deferring the ingest until `close()`.
- **SIP Compliance**: On macOS, `active` mode handles Entitlements and SIP-stripping for children automatically.
