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
- **Safety Guarantee**: Even if the CAS (TheSource) is completely wiped or corrupted, project files remain 100% intact. They are independent references to the same data blocks.

### 4.2 Phantom Mode (幻影 / Advanced)
- **Concept**: The environment is a "Phantom." Physical files are moved to CAS and replaced by the virtual projection.
- **UX Feedback**: `Velo is active in [Phantom] mode. Project is now purely virtual.`
- **Mechanism**: `Live Ingest` + `Move`.
- **Rollback Experience**: **Virtual-Only**. Deactivating the layer leaves the directory "empty" until Velo performs an inverse-ingest (Restoration) to bring physical files back from the CAS.

## 5. Atomic Implementation Strategy

To guarantee **absolute safety**, Velo Rift™ enforces two invariants:

| Invariant | Semantic | Guarantee |
|-----------|----------|----------|
| **P0-a** | Hash-Content Match | `CAS[hash].content == hash(content)` always |
| **P0-b** | Projection-Link Match | `VFS[path]` always returns the correct version |

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
- Each `ingest_solid()` call holds an **Ingest Lock** on the source path.
- Lock is acquired BEFORE snapshot, released AFTER Manifest update.
- This ensures: _"If Manifest says H, then CAS[H] is already committed."_

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

## 6. CAS Directory Structure (The Source)
Velo Rift™ uses a sharded directory structure to optimize for filesystem performance and debuggability.

### 6.1 Path Logic
The CAS root resides at `VR_THE_SOURCE` (default: `~/.vrift/the_source`).

**Structure:**
```text
the_source/
  └── blake3/              <-- Algorithm Namespace
       ├── ab/             <-- Shard L1 (Hash 0-1)
       │   └── cd/         <-- Shard L2 (Hash 2-3)
       │       └── efgh..._[Size].[Ext]  <-- Artifact (Remaining Hash)
```

### 6.2 Naming Convention
Artifacts utilize a "Self-Describing" filename format:
`[Remaining_Hash]_[Size_in_Bytes].[Original_Extension]`

**Reconstruction:**
`Full_Hash = Shard_L1 + Shard_L2 + Remaining_Hash`
Example: `abcdef12345...` -> `ab/cd/ef12345...`

**Benefits:**
- **Sharding**: Prevents directory inode exhaustion (billions of files).
- **Integrity Check**: `stat()` size matches filename size (O(1) corrupt check).
- **Debuggability**: Extensions allow direct inspection (`cat`, `open`, `objdump`) without metadata lookup.
- **Example**: `ab/cd/ef12345..._1024.rs`

## 7. Persistence & Crash Recovery

Velo Rift must survive restarts without losing file mappings or corrupting project state.

### 7.1 Manifest Architecture (Dual-Layer)

The Manifest is the **Single Source of Truth** for path → hash mappings. It uses a two-layer structure for optimal performance.

#### Layer Structure

| Layer | Content | Storage | Properties |
|-------|---------|---------|------------|
| **Base Layer** | System libs, registry deps | LMDB (shared mmap) | Immutable, O(1) lookup |
| **Delta Layer** | Tenant modifications | DashMap (per-project) | Mutable, Copy-on-Write |

```rust
struct ManifestLookup {
    base: LmdbManifest,                           // Global, shared
    delta: DashMap<PathBuf, DeltaEntry>,          // Per-project
}

enum DeltaEntry {
    Modified(ManifestEntry),   // Points to new hash
    Deleted,                   // Whiteout marker
}

struct ManifestEntry {
    hash: Hash,
    tier: Tier,
    original_mode: u32,
    ingest_time: u64,
}
```

#### Lookup Algorithm

```rust
fn lookup(&self, path: &Path) -> Option<ManifestEntry> {
    // 1. Check Delta Layer (project modifications)
    if let Some(entry) = self.delta.get(path) {
        return match entry.value() {
            DeltaEntry::Modified(e) => Some(e.clone()),
            DeltaEntry::Deleted => None,  // Whiteout
        };
    }
    // 2. Check Base Layer (shared packages)
    self.base.get(path)
}
```

### 7.1.1 Storage Backend: LMDB

**Why LMDB over JSON**:

| Dimension | JSON | LMDB |
|-----------|------|------|
| Read | O(n) parse | **O(1) mmap** |
| Write | O(n) serialize | **O(1) incremental** |
| Concurrency | Exclusive | **MVCC (readers never block)** |
| Crash Safety | Atomic rename | **ACID transactions** |
| Memory | Full load | **Lazy mmap** |

**Implementation**:

```rust
pub struct LmdbManifest {
    env: heed::Env,
    entries: Database<Str, SerdeBincode<ManifestEntry>>,
}

impl LmdbManifest {
    fn open(path: &Path) -> Result<Self> {
        let env = heed::EnvOpenOptions::new()
            .map_size(1 << 30)  // 1GB max
            .open(path)?;
        let entries = env.create_database(Some("manifest"))?;
        Ok(Self { env, entries })
    }

    fn get(&self, path: &Path) -> Option<ManifestEntry> {
        let rtxn = self.env.read_txn().ok()?;
        self.entries.get(&rtxn, path.to_str()?).ok().flatten()
    }

    fn put(&self, path: &Path, entry: &ManifestEntry) -> Result<()> {
        let mut wtxn = self.env.write_txn()?;
        self.entries.put(&mut wtxn, path.to_str()?, entry)?;
        wtxn.commit()
    }
}
```

**Storage Location**: `.vrift/manifest.lmdb` (per-project)

### 7.2 Startup Recovery

On `vrift active` or daemon restart:

```rust
fn startup_recovery() -> Result<()> {
    let manifest = Manifest::load()?;

    for (path, entry) in &manifest.entries {
        match entry.tier {
            Tier::Tier1 => validate_tier1(path, entry)?,
            Tier::Tier2 => validate_tier2(path, entry)?,
        }
    }
    Ok(())
}

fn validate_tier1(path: &Path, entry: &ManifestEntry) -> Result<()> {
    // Tier-1: Source should be symlink → CAS
    if !path.is_symlink() {
        // Restore symlink
        symlink(get_cas_path(&entry.hash), path)?;
    }
    Ok(())
}

fn validate_tier2(path: &Path, entry: &ManifestEntry) -> Result<()> {
    // Tier-2: Source should be hardlink to CAS (same inode)
    let source_ino = fs::metadata(path)?.ino();
    let cas_ino = fs::metadata(get_cas_path(&entry.hash))?.ino();
    if source_ino != cas_ino {
        // Hardlink broken, re-establish or warn
        warn!("Tier-2 link broken for {:?}, re-ingesting", path);
        ingest_mutable(path)?;
    }
    Ok(())
}
```

### 7.3 Phantom Mode Recovery

In Phantom Mode, source files are **moved** to CAS. On restart, paths may appear empty.

```rust
fn restore_phantom_projections(manifest: &Manifest) -> Result<()> {
    for (path, entry) in manifest.phantom_entries() {
        if !path.exists() {
            // Restore visibility via symlink (lightweight)
            // or FUSE mount (full fidelity)
            symlink(get_cas_path(&entry.hash), path)?;
        }
    }
    Ok(())
}
```

### 7.4 Crash Recovery Matrix

| Scenario | Detection | Recovery |
|----------|-----------|----------|
| **Clean shutdown** | Manifest valid | Normal startup |
| **Manifest missing** | File not found | Scan CAS, rebuild from symlinks |
| **Manifest corrupted** | Parse error | Restore from `.vrift/manifest.json.bak` |
| **CAS entry missing** | Hash lookup fails | Remove from Manifest, warn user |
| **Orphan CAS entries** | Not in any Manifest | GC candidates |

### 7.5 Durability Guarantees

| State | Durability | Recovery |
|-------|------------|----------|
| **Manifest** | Persisted atomically | Load from disk |
| **Tier-1 symlinks** | Filesystem durable | Self-describing |
| **Tier-2 hardlinks** | Filesystem durable | Verifiable via inode |
| **CAS entries** | Filesystem durable | Content-addressable |

### 7.6 WAL (Optional Enhancement)

For high-frequency ingest scenarios, a Write-Ahead Log reduces fsync overhead:

```rust
fn ingest_with_wal(source: &Path) -> Result<()> {
    let hash = blake3::hash_file(source)?;
    
    // Step 1: Append to WAL (fast, sequential write)
    wal.append(WalEntry::Ingest { path: source, hash })?;
    
    // Step 2: Perform ingest
    do_ingest(source, hash)?;
    
    // Step 3: Checkpoint WAL → Manifest periodically
    if wal.size() > CHECKPOINT_THRESHOLD {
        manifest.merge_wal(&wal)?;
        wal.truncate()?;
    }
    Ok(())
}
```

## 8. Implementation Notes
- **Persistent State**: `vrift active` creates a long-lived Session.
- **ABI Continuity**: The Session persists the **ABI_Context**, ensuring that a long-running development environment remains binary-consistent.
- **Shim Performance**: Shadow capturing avoids the latency of synchronous hashing during small `write()` calls by deferring the ingest until `close()`.
- **SIP Compliance**: On macOS, `active` mode handles Entitlements and SIP-stripping for children automatically.

## 9. Implementation References

For internal data structures and performance optimizations, see [ARCHITECTURE.md](./ARCHITECTURE.md):

| Topic | Section | Description |
|-------|---------|-------------|
| Hash & ID Optimization | [§13](./ARCHITECTURE.md#13-hash--id-optimization-strategy) | Interning, VeloId bit-packing, storage vs runtime sizes |
| Packfile / Blob Packing | [§12.1](./ARCHITECTURE.md#121-packfile--blob-packing-hotspot-consolidation) | Profile-guided packing, hotspot consolidation |
| VeloVFS Runtime | [§9](./ARCHITECTURE.md#9-velovfs-runtime-architecture) | LD_PRELOAD shim, Manifest lookup, Vnode structure |
| Multi-Tenant Isolation | [§8](./ARCHITECTURE.md#8-multi-tenant-isolation-architecture) | Namespace isolation, OverlayFS mechanics |
| Python Optimizations | [§10](./ARCHITECTURE.md#10-python-specific-optimizations) | PEP 683, import hooks, bytecode caching |
