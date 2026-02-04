# vrift IPC Protocol Specification

This document defines the Inter-Process Communication protocol between InceptionLayer clients and `vriftd` daemon.

---

## 1. Transport Layer

### Unix Domain Socket (UDS)

**Default Socket Path**: `/tmp/vrift.sock`

**Properties**:
- Stream-oriented (SOCK_STREAM)
- Single daemon serves all projects (current implementation)
- Persistent connections with length-prefixed messages

---

## 2. Wire Format

All messages use **bincode serialization** with length prefix:

```
┌─────────────┬───────────────────────┐
│ Length (u32)│ bincode(VeloRequest)  │
│  LE bytes   │     or VeloResponse   │
└─────────────┴───────────────────────┘
```

**Protocol Flow**:
```rust
// Send: length prefix + bincode payload
let bytes = bincode::serialize(&request)?;
stream.write_all(&(bytes.len() as u32).to_le_bytes())?;
stream.write_all(&bytes)?;

// Receive: read length, then payload
let mut len_buf = [0u8; 4];
stream.read_exact(&mut len_buf)?;
let len = u32::from_le_bytes(len_buf) as usize;
let mut payload = vec![0u8; len];
stream.read_exact(&mut payload)?;
let response: VeloResponse = bincode::deserialize(&payload)?;
```

---

## 3. Request Types (VeloRequest)

```rust
#[derive(Debug, Serialize, Deserialize)]
pub enum VeloRequest {
    // Connection
    Handshake { client_version: String },
    Status,
    RegisterWorkspace { project_root: String },
    
    // Manifest Operations (VFS metadata)
    ManifestGet { path: String },
    ManifestUpsert { path: String, entry: VnodeEntry },
    ManifestRemove { path: String },
    ManifestRename { old_path: String, new_path: String },
    ManifestUpdateMtime { path: String, mtime_ns: u64 },
    ManifestReingest { vpath: String, temp_path: String },
    ManifestListDir { path: String },
    
    // CAS Operations (content storage)
    CasInsert { hash: [u8; 32], size: u64 },
    CasGet { hash: [u8; 32] },
    CasSweep { bloom_filter: Vec<u8> },
    
    // File Locking (RFC-0049)
    FlockAcquire { path: String, operation: i32 },
    FlockRelease { path: String },
    
    // Process Management
    Spawn { command: Vec<String>, env: Vec<(String, String)>, cwd: String },
    Protect { path: String, immutable: bool, owner: Option<String> },
}
```

---

## 4. Response Types (VeloResponse)

```rust
#[derive(Debug, Serialize, Deserialize)]
pub enum VeloResponse {
    // Acknowledgements
    HandshakeAck { server_version: String },
    StatusAck { status: String },
    RegisterAck { workspace_id: String },
    CasAck,
    ProtectAck,
    FlockAck,
    
    // Manifest Responses
    ManifestAck { entry: Option<VnodeEntry> },
    ManifestListAck { entries: Vec<DirEntry> },
    
    // CAS Responses
    CasFound { size: u64 },
    CasNotFound,
    CasSweepAck { deleted_count: u32, reclaimed_bytes: u64 },
    
    // Process Responses
    SpawnAck { pid: u32 },
    
    // Error
    Error(String),
}
```

---

## 5. Data Structures

### VnodeEntry (Manifest Entry)

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VnodeEntry {
    pub content_hash: [u8; 32],  // BLAKE3 hash
    pub size: u64,
    pub mtime: u64,              // Unix timestamp (ns)
    pub mode: u32,               // File mode (permissions)
    pub flags: u16,              // 0x01 = dir, 0x02 = symlink
}
```

### DirEntry (Directory Listing)

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
}
```

---

## 6. Protocol Flow Examples

### 6.1 Handshake + Status

```
Client                              Server
   |                                   |
   |--- Handshake { "0.1.0" } -------->|
   |<-- HandshakeAck { "0.1.0" } ------|
   |                                   |
   |--- Status -------------------->|
   |<-- StatusAck { "ready" } ---------|
```

### 6.2 Workspace Registration + Manifest Query

```
Client                              Server
   |                                   |
   |--- RegisterWorkspace { "/proj" }->|
   |<-- RegisterAck { "a8f9..." } -----|
   |                                   |
   |--- ManifestGet { "src/main.rs" }->|
   |<-- ManifestAck { Some(entry) } ---|
```

### 6.3 CoW Write Flow (ManifestReingest)

```
Client                              Server
   |                                   |
   | [write to temp file locally]      |
   | [close triggers reingest]         |
   |                                   |
   |--- ManifestReingest { -------->|
   |      vpath: "src/lib.rs",         |
   |      temp_path: "/tmp/cow123"     |
   |    }                              |
   |                                   |
   |                                   | [hash temp file]
   |                                   | [insert to CAS]
   |                                   | [update manifest]
   |                                   |
   |<-- ManifestAck { entry } ---------|
```

---

## 7. Shared Memory Fast Path (RFC-0044)

For read-heavy operations, manifest data is also exported to shared memory:

**File**: `/tmp/vrift-manifest.mmap`

**Layout**:
```
┌──────────────────────────────────────────────────────────┐
│ ManifestMmapHeader (40 bytes)                            │
├──────────────────────────────────────────────────────────┤
│ Bloom Filter (32KB) - path existence check               │
├──────────────────────────────────────────────────────────┤
│ Stat Hash Table (MmapStatEntry[]) - path → metadata      │
├──────────────────────────────────────────────────────────┤
│ Dir Index Table (MmapDirIndexEntry[]) - parent → children│
├──────────────────────────────────────────────────────────┤
│ Children Pool (MmapDirChild[]) - directory entries       │
└──────────────────────────────────────────────────────────┘
```

**Header**:
```rust
pub struct ManifestMmapHeader {
    pub magic: u32,           // 0x504D4D56 ("VMMP")
    pub version: u32,         // Format version (1)
    pub entry_count: u32,
    pub bloom_offset: u32,
    pub table_offset: u32,
    pub table_capacity: u32,
    pub dir_index_offset: u32,
    pub dir_index_capacity: u32,
    pub children_offset: u32,
    pub children_count: u32,
}
```

**Stat Entry**:
```rust
pub struct MmapStatEntry {
    pub path_hash: u64,    // FNV-1a hash (0 = empty slot)
    pub size: u64,
    pub mtime: i64,
    pub mtime_nsec: i64,
    pub mode: u32,
    pub flags: u32,        // 0x01 = dir, 0x02 = symlink
}
```

---

## 8. Reconnection and Error Handling

**Client Behavior**:
- Connect on first VFS operation
- Reconnect on socket error
- Fall back to real FS if daemon unavailable

**Timeout**:
- Default connect timeout: 5 seconds
- No per-request timeout (blocking I/O)

---

## 9. Implementation Notes

### Current vs. Future Architecture

| Aspect | Current (v1) | Future (v3 Staging) |
|--------|--------------|---------------------|
| Daemon | Single `vriftd` | Per-project `vdir_d` |
| Socket | `/tmp/vrift.sock` | `~/.vrift/sockets/<project>.sock` |
| Write Model | ManifestReingest | Staging Area + Dirty Bit |

> **Note**: The Staging Area model with Dirty Bit (documented in 02-vdir-synchronization.md and 06-write-path-ingestion.md) is the target architecture for Sprint 1.

---

[End of Document]
