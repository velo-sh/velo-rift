# vrift IPC Protocol Specification

This document defines the Inter-Process Communication protocol between InceptionLayer clients and `vdir_d` (VDir Daemon).

---

## 1. Transport Layer

### Unix Domain Socket (UDS)

**Default Socket Path**: `~/.vrift/sockets/<project_name>.sock`

**Properties**:
- Stream-oriented (SOCK_STREAM)
- Per-project daemon architecture
- Persistent connections with frame-based protocol
- Zero-copy deserialization where applicable (planned)

---

## 2. Wire Format (Version 4)

All messages utilize a 12-byte fixed header followed by a variable-length **rkyv-serialized** payload.

### 2.1 IpcHeader (12 Bytes)

| Field | Type | Offset | Description |
| :--- | :--- | :--- | :--- |
| `magic` | `[u8; 2]` | 0 | "VR" (Vrift) |
| `type_ver` | `u8` | 2 | hi4=type, lo4=version (4) |
| `flags` | `u8` | 3 | Reserved |
| `length` | `u32` (LE) | 4 | Payload length (Max 32MB safety cap) |
| `seq_id` | `u32` (LE) | 8 | Sequence ID for request-response matching |

### 2.2 Frame Types

- `Request` (0): Client to Server
- `Response` (1): Server to Client
- `Heartbeat` (2): Bidirectional keep-alive (RFC-0053)

### 2.3 Implementation Details

```rust
// IpcHeader implementation in crates/vrift-ipc/src/lib.rs
pub struct IpcHeader {
    pub magic: [u8; 2],
    pub type_ver: u8,
    pub flags: u8,
    pub length: u32,
    pub seq_id: u32,
}
```

**Protocol Flow**:
1. Client sends 12-byte `IpcHeader` (type=Request).
2. Client sends `rkyv` payload.
3. Server reads 12-byte `IpcHeader`.
4. Server reads `length` bytes of payload.
5. Server sends 12-byte `IpcHeader` (type=Response, same `seq_id`).
6. Server sends `rkyv` response payload.

---

## 3. Communication Patterns

### 3.1 Synchronous vs Async
- **Inception Layer (Shim)**: Uses synchronous blocking I/O (via `crates/vrift-ipc/src/lib.rs::frame_sync`).
- **Daemon (`vdir_d`)**: Uses asynchronous Tokio-based I/O (via `crates/vrift-ipc/src/lib.rs::frame_async`).

### 3.2 Heartbeats (RFC-0053)
Heartbeats are zero-length payload frames (`length = 0`) with `FrameType::Heartbeat`. They are used to prevent socket timeouts and verify connection liveness. Both sides should skip heartbeats during normal request processing.

---

## 4. Request Types (VeloRequest)

```rust
pub enum VeloRequest {
    Handshake { client_version: String, protocol_version: u32 },
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
    
    // Process/Safety
    Spawn { command: Vec<String>, env: Vec<(String, String)>, cwd: String },
    Protect { path: String, immutable: bool, owner: Option<String> },
    
    // RFC-0049: File Locking
    FlockAcquire { path: String, operation: i32 },
    FlockRelease { path: String },
}
```

---

## 5. Response Types (VeloResponse)

Responses are wrapped in `VeloResponse` and may contain structured errors (`VeloError`).

```rust
pub enum VeloResponse {
    HandshakeAck { server_version: String },
    StatusAck { status: String },
    RegisterAck { workspace_id: String },
    CasAck,
    ManifestAck { entry: Option<VnodeEntry> },
    ManifestListAck { entries: Vec<DirEntry> },
    CasFound { size: u64 },
    CasNotFound,
    SpawnAck { pid: u32 },
    Error(VeloError), // Phase 3: Structured Errors
}
```

---

## 6. Error Handling

Structured errors use `VeloErrorKind` to allow the client to map IPC errors back to standard `errno` (e.g., `NotFound` -> `ENOENT`).

---

[End of Document]
