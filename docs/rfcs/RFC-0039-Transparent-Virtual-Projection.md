# RFC-0039: Transparent Virtual Projection & In-place Transmutation

## 1. Status
**Partial Implementation** (Updated: 2026-02-03)

### Implementation Status

| Feature | Section | Status | Notes |
|---------|---------|--------|-------|
| Solid Tier-1 (immutable) | §5.1.1 | ✅ Done | `hard_link` + symlink replacement |
| Solid Tier-2 (mutable) | §5.1.2 | ✅ Done | `hard_link`, keep original |
| Phantom Mode | §5.2 | ✅ Done | Atomic `rename()` to CAS |
| Ingest Lock (flock) | §5.3 | ✅ Done | `lock_with_retry()` |
| CAS Sharding | §6 | ✅ Done | `blake3/ab/cd/hash_size.ext` |
| Parallel Ingest | - | ✅ Done | Rayon, ~14,000 files/sec |
| Break-Before-Write | §5.1.2 | ✅ Done | COW to temp via shim (test verified) |
| Live Ingest on close() | §3.2 | ✅ Done | `sync_ipc_manifest_reingest` on close |
| Tier-1 chattr +i | §5.1.1 | ✅ Done | Native implementation (macOS/Linux) |
| Tier-1 chown | §5.1.1 | ⏸️ Deferred | Deferred: High friction (requires root) |

## 2. Context & Objectives
Velo Rift™ aims to eliminate the friction between "Project Content" and "Disk Storage." This RFC proposes a **Transparent Projection Model** where the VFS layer replaces heavy-duty physical directories (e.g., `node_modules`, `target`, `.venv`) with a dynamic virtual lens. The environment is intended to be **long-lived**, becoming the primary state of the workspace rather than a transient execution context.

## 3. Core Concepts

### 3.1 Inception Mode
- **Action**: `vrift inception` (Enter the "dream")
- **Function**: Transitions the workspace into a persistent **VFS Layer**. Velo Rift™ acts as a "Live Lens" over the project directory.
- **Dependency Replacement**: Folders like `node_modules` or `target` are mapped from the CAS. They appear physically present but are managed virtual assets.

### 3.2 Live Ingest
Velo Rift™ automates the existing `ingest` logic:
- **Trigger**: When a process finishes writing a file (`close()`), Velo performs a **Live Ingest**.
- **Efficiency**: The file is hashed and either moved or hardlinked into the CAS.
- **SSOT**: The Manifest is updated immediately, ensuring the virtual view is always in sync.

### 3.3 Dimensional Ingest (ABI Tags)
To handle multi-version binaries:
- The `ingest` process considers the **ABI_Context** for binary files (`.so`, `.dylib`).
- This prevents collisions between different versions at the same path.

### 3.4 Storage Layout

| Component | Location | Description |
|-----------|----------|-------------|
| **User View** | **Original project directory** | Users work normally; Velo transparently redirects |
| **CAS (The Source)** | `~/.vrift/the_source/` | Content-addressable storage for all files |
| **Manifest** | `.vrift/manifest.lmdb` (per-project) | Path → Hash mappings |
| **Session State** | `.vrift/session.json` | Active mode, ABI context |

**Projection Mechanism**:

| Tier | User Sees | Actual Storage |
|------|-----------|----------------|
| **Tier-1** | `/project/node_modules/...` | Symlink → `~/.vrift/the_source/...` |
| **Tier-2** | `/project/target/...` | Hardlink (shared inode with CAS) |

## 4. Operational Modes

Velo Rift™ provides two modes to balance safety and performance.

### 4.1 Solid Mode (Default)
- **Behavior**: Physical files remain in project directory.
- **Rollback**: Instant, zero-impact on file availability.
- **UX**: `Velo is active in [Solid] mode. Physical files are safe.`

### 4.2 Phantom Mode (Advanced)
- **Behavior**: Physical files moved to CAS, replaced by virtual mapping.
- **Rollback**: Requires inverse-ingest (Restoration).
- **UX**: `Velo is active in [Phantom] mode. Project is now purely virtual.`

> For implementation details, see [Section 5](#5-atomic-implementation-strategy).

## 5. Atomic Implementation Strategy

To guarantee **absolute safety**, Velo Rift™ enforces two invariants (The "Iron Law"):

| Invariant | Semantic | Guarantee |
|-----------|----------|----------|
| **P0-a** | Hash-Content Match | `CAS[hash].content == hash(content)` ALWAYS. READ-ONLY. |
| **P0-b** | Projection-Link Match | `VFS[path]` always returns the correct version |

> [!CAUTION]
> **Iron Law (P0-a)**: Any modification to a CAS-managed file MUST break the hardlink before writing. Direct modification of an ingested inode is strictly prohibited as it corrupts the global CAS.

Velo adheres to strict atomic syscall sequences and locking protocols to uphold both invariants.

### 5.1 Solid Mode (Tiered Asset Model)

Assets are classified by write frequency for optimized protection strategies.

#### Asset Tiers

| Tier | Type | Write Frequency | Examples |
|------|------|-----------------|----------|
| **Tier-1** | Immutable | Never | Toolchains, registry deps (`rustc`, `node_modules/@*`) |
| **Tier-2** | Mutable | Rare | Build artifacts (`target/*.rlib`, `dist/*.js`) |

#### 5.1.1 Tier-1: Immutable Assets (Maximum Protection)

**Policy**: Deny all writes. Transfer ownership to Velo Rift.

```rust
fn ingest_immutable(source: &Path) -> Result<()> {
    let hash = blake3::hash_file(source)?;
    let cas_target = get_cas_path(hash);

    fs::hard_link(source, &cas_target)?;

    // Maximum protection: owner → vrift, immutable flag
    chown(&cas_target, VRIFT_UID, VRIFT_GID)?;
    chmod(&cas_target, 0o444)?;
    set_immutable(&cas_target)?;  // chattr +i / chflags uchg

    // Replace source with symlink projection
    fs::remove_file(source)?;
    symlink(&cas_target, source)?;

    manifest.insert(hash, source);
    Ok(())
}
```

**Guarantees**:
- **Owner transfer**: vrift user owns CAS, user cannot modify.
- **Immutable flag**: Even root needs `chattr -i` first.
- **Source = symlink**: Write attempts → `EACCES`, no VFS overhead.

#### 5.1.2 Tier-2: Mutable Assets (Break-Before-Write)

**Policy**: Allow writes via VFS interception.

```rust
fn ingest_mutable(source: &Path) -> Result<()> {
    let hash = blake3::hash_file(source)?;
    let cas_target = get_cas_path(hash);

    if !cas_target.exists() {
        fs::hard_link(source, &cas_target)?;
        chmod_readonly(&cas_target)?;
    }
    chmod_readonly(source)?;  // Soft protection
    manifest.insert(hash, source);
    Ok(())
}

fn vfs_open_write(path: &Path) -> Result<File> {
    if !is_tier2_ingested(path) {
        return File::open_write(path);
    }
    // Break hardlink before write
    let content = fs::read(path)?;
    fs::remove_file(path)?;
    fs::write(path, &content)?;
    chmod_writable(path)?;
    File::open_with_reingest_on_close(path)
}
```

**Guarantees**:
- CAS immutable (different inode after break).
- `close()` triggers re-ingest with new hash.

#### 5.1.3 Tier Classification

| Source | Tier | Detection |
|--------|------|-----------|
| Registry deps | Tier-1 | Manifest `source: "registry"` |
| Toolchains | Tier-1 | Path pattern `/toolchains/*` |
| Build outputs | Tier-2 | Path pattern `target/*`, `dist/*` |
| User config | Tier-2 | Default for unclassified |

#### 5.1.4 Comparison

| Dimension | Tier-1 (Immutable) | Tier-2 (Mutable) |
|-----------|-------------------|------------------|
| Security | Maximum (owner + immutable) | Medium (chmod 444 + VFS) |
| Performance | Highest (no VFS intercept) | High (VFS only on write) |
| Write | Denied | Allowed (break link) |
| Rollback | Restore symlink | `chmod +w` |

#### 5.1.5 Performance Optimizations

##### Read Path

**Tier-1 (Zero Overhead)**:
```text
read(node_modules/@types/node/index.d.ts)
  → kernel follows symlink → CAS/xxxx
  → direct read, no VFS intercept
```

**Tier-2 (Prefetch)**:
```rust
fn prefetch_tier2(manifest: &Manifest) {
    for (_, hash) in manifest.tier2_entries() {
        posix_fadvise(get_cas_path(hash), POSIX_FADV_WILLNEED);
    }
}
```

##### Write Path (Truncate-Write Pattern)

Build tools typically **truncate + overwrite** (not append). Optimization:

```rust
fn vfs_open_write_optimized(path: &Path, flags: OpenFlags) -> Result<File> {
    if !is_tier2_ingested(path) {
        return File::open(path, flags);
    }

    if flags.contains(O_TRUNC) {
        // Fast path: truncate-write, no need to copy old content
        fs::remove_file(path)?;           // O(1) unlink
        manifest.mark_stale(path);
        return File::create_with_reingest_on_close(path);
    }

    // Slow path: append/update, must preserve old content
    let content = fs::read(path)?;
    fs::remove_file(path)?;
    fs::write(path, &content)?;
    chmod_writable(path)?;
    File::open_with_reingest_on_close(path, flags)
}
```

**Write Pattern Analysis**:

| Pattern | Operation | Optimization |
|---------|-----------|--------------|
| Truncate + Write | `cargo build` → `.rlib` | Skip content copy ✅ |
| Append | Log files | Copy old content |
| In-place Update | Binary patch | Copy old content |

### 5.2 Phantom Mode (Atomic Replacement)
The goal is to replace the physical file with a virtual entry atomically.

```rust
fn ingest_phantom(source: Path) -> Result<()> {
    let hash = blake3(&source)?;
    let cas_target = get_cas_path(hash);

    // 1. Rename (Atomic Move)
    // rename() is atomic on POSIX for same-filesystem paths.
    // The file exists either at 'source' OR 'cas_target', never neither.
    std::fs::rename(&source, &cas_target)?;

    // 2. Update Manifest
    manifest.insert(hash, source);
    Ok(())
}
```

### 5.3 Projection Consistency Protocol (P0-b Enforcement)

To ensure `VFS[path]` always returns the correct version, even during concurrent ingest:

**Ingest Lock Mechanism:**

> [!IMPORTANT]
> **Ingest Lock = `flock(LOCK_SH)`** (shared read lock)
>
> **Purpose**: Prevent external writes during read, ensuring `hash(content) == content`.
>
> Without this lock, external programs could modify the file while we are reading,
> causing the computed hash to not match the actual content (violates P0-a).

- Each `ingest_solid()` call holds a shared lock (`flock(LOCK_SH)`) on the source file.
- Lock is acquired BEFORE snapshot, released AFTER Manifest update.
- External writers are blocked while lock is held.
- This ensures: _"If Manifest says H, then CAS[H] is already committed and H == hash(content)."_

**VFS Read Priority:**
```rust
fn vfs_read(path: &Path) -> Result<Bytes> {
    if ingest_locks.is_held(path) {
        // Ingest in progress: read physical file (source of truth)
        return fs::read(path);
    }
    // Normal: read from CAS via Manifest
    let hash = manifest.get(path)?;
    fs::read(get_cas_path(hash))
}
```

**Guarantees:**
| State | VFS Returns | Correctness |
|-------|-------------|-------------|
| Ingest lock held | Physical file | ✅ Live content |
| No lock, Manifest=H | CAS[H] | ✅ Committed snapshot |

This eliminates the race window between CAS write and Manifest update.

## 6. CAS Directory Structure

Velo Rift™ uses a sharded CAS structure at `~/.vrift/the_source`:

```text
the_source/blake3/ab/cd/efgh..._[Size].[Ext]
```

**Key Properties**:
- 2-level sharding prevents inode exhaustion
- Size in filename enables O(1) integrity check
- Extension enables direct file inspection

> For detailed path logic and naming conventions, see [ARCHITECTURE.md §1.2](./ARCHITECTURE.md#12-structure-hierarchy).

## 7. Persistence & Crash Recovery

Velo Rift must survive restarts without losing file mappings.

### 7.1 Manifest Architecture

**Dual-Layer Structure**:

| Layer | Content | Storage | Properties |
|-------|---------|---------|------------|
| **Base Layer** | System libs, registry deps | LMDB (mmap) | Immutable, O(1) |
| **Delta Layer** | Project modifications | DashMap | Mutable, COW |

**Why LMDB**:
- O(1) mmap reads (zero-copy)
- MVCC (readers never block)
- ACID transactions

**Storage**: `.vrift/manifest.lmdb` (per-project)

### 7.2 Recovery Strategy

| Scenario | Recovery |
|----------|----------|
| Clean shutdown | Normal load |
| Manifest missing | Scan CAS, rebuild |
| Manifest corrupted | Restore from backup |
| CAS entry missing | Remove entry, warn user |

### 7.3 Durability Guarantees

| State | Durability |
|-------|------------|
| Manifest | LMDB ACID |
| Tier-1 symlinks | Filesystem |
| Tier-2 hardlinks | Filesystem |
| CAS entries | Content-addressable |

> For implementation details (LMDB API, startup recovery code, WAL), see [ARCHITECTURE.md §9.8](./ARCHITECTURE.md#98-persistence--crash-recovery-rfc-0039).

## 8. Implementation Notes
- **Persistent State**: `vrift inception` creates a long-lived Session.
- **ABI Continuity**: The Session persists the **ABI_Context**, ensuring that a long-running development environment remains binary-consistent.
- **Shim Performance**: Shadow capturing avoids the latency of synchronous hashing during small `write()` calls by deferring the ingest until `close()`.
- **SIP Compliance**: On macOS, `inception` mode handles Entitlements and SIP-stripping for children automatically.

## 9. Implementation References

For internal data structures and performance optimizations, see [ARCHITECTURE.md](./ARCHITECTURE.md):

| Topic | Section | Description |
|-------|---------|-------------|
| Hash & ID Optimization | [§13](./ARCHITECTURE.md#13-hash--id-optimization-strategy) | Interning, VeloId bit-packing, storage vs runtime sizes |
| Packfile / Blob Packing | [§12.1](./ARCHITECTURE.md#121-packfile--blob-packing-hotspot-consolidation) | Profile-guided packing, hotspot consolidation |
| VeloVFS Runtime | [§9](./ARCHITECTURE.md#9-velovfs-runtime-architecture) | LD_PRELOAD shim, Manifest lookup, Vnode structure |
| Multi-Tenant Isolation | [§8](./ARCHITECTURE.md#8-multi-tenant-isolation-architecture) | Namespace isolation, OverlayFS mechanics |
| Python Optimizations | [§10](./ARCHITECTURE.md#10-python-specific-optimizations) | PEP 683, import hooks, bytecode caching |

## 10. Future Work

| Topic | Priority | Description |
|-------|----------|-------------|
| **GC Strategy** | P2 | Orphan CAS entry cleanup via reference counting or mark-sweep |
| **Bloom Filter** | P1 | High-speed hash existence checks for Ingest/GC |
| **Multi-Project Sharing** | P2 | Shared Base Layer across projects, per-project Delta Layer |
| **Privileged Isolation** | P4 | Optional `chown` to system user (requires root setup) |
| **Remote CAS** | P3 | Network-backed CAS for distributed teams |
