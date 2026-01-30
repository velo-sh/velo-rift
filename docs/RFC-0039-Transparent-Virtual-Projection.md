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

## 4. Operational Strategies: No-feeling Rollback
Velo Rift™ provides two specific modes to balance safety and performance.

### 4.1 Solid Mode (默认 / Default)
- **Concept**: The environment is "Solid." Physical files remain in the project directory.
- **UX Feedback**: `Velo is active in [Solid] mode. Physical files are safe.`
- **Mechanism**: `Link-to-CAS` (Atomic).
- **Implementation**: Instead of moving the file, Velo simply creates a hardlink in the CAS pointing to the existing project file. The inode remains identical.
- **Rollback Experience**: **Perfect**. Since physical inodes never moved, deactivating Velo has zero impact on file availability.

### 4.2 Phantom Mode (幻影 / Advanced)
- **Concept**: The environment is a "Phantom." Physical files are moved to CAS and replaced by the virtual projection.
- **UX Feedback**: `Velo is active in [Phantom] mode. Project is now purely virtual.`
- **Mechanism**: `Live Ingest` + `Move`.
- **Rollback Experience**: **Virtual-Only**. Deactivating the layer leaves the directory "empty" until Velo performs an inverse-ingest (Restoration) to bring physical files back from the CAS.

## 5. Implementation Notes
- **Persistent State**: `vrift active` creates a long-lived Session, maintaining the projection across multiple shell instances.
- **ABI Continuity**: The Session persists the **ABI_Context**, ensuring binary consistency for development.
- **Shim Performance**: Capture occurs on `close()`, ensuring native disk speeds during active write cycles.
- **SIP Compliance**: On macOS, `active` mode handles Entitlements and SIP-stripping automatically.
