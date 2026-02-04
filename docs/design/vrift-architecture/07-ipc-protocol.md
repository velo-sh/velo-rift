# vrift IPC Protocol Specification

This document defines the Inter-Process Communication protocol between InceptionLayer clients and `vdir_d` project daemons.

---

## 1. Transport Layer

### Unix Domain Socket (UDS)

**Socket Path**: `~/.vrift/sockets/<project_id>.sock`

**Properties**:
- Stream-oriented (SOCK_STREAM)
- Per-project isolation
- Auto-created by `vdir_d` on startup
- Cleaned up on graceful shutdown

**Connection Lifecycle**:
```
1. vdir_d creates socket at startup
2. InceptionLayer connects on first write operation
3. Connection persists for process lifetime
4. Socket HUP signals client disconnect
```

---

## 2. Message Frame Format

All messages use a simple length-prefixed binary format:

```c
struct MessageFrame {
    uint32_t magic;        // 0x56524946 ("VRIF")
    uint32_t version;      // Protocol version (currently 1)
    uint32_t msg_type;     // See MessageType enum
    uint32_t payload_len;  // Length of payload in bytes
    uint8_t  payload[];    // Variable-length payload
} __attribute__((packed));
```

**Wire Format** (Little Endian):
```
[0:4]   Magic (0x56524946)
[4:8]   Version (1)
[8:12]  MessageType
[12:16] PayloadLength
[16:N]  Payload (N = PayloadLength)
```

---

## 3. Message Types

```c
enum MessageType {
    // Client → Server
    MSG_CONNECT         = 0x0001,  // Initial handshake
    MSG_COMMIT          = 0x0002,  // File write complete
    MSG_ABORT           = 0x0003,  // Cancel in-progress write
    MSG_PING            = 0x0004,  // Keepalive
    
    // Server → Client
    MSG_CONNECT_ACK     = 0x1001,  // Handshake response
    MSG_COMMIT_ACK      = 0x1002,  // Commit success
    MSG_COMMIT_NACK     = 0x1003,  // Commit failure
    MSG_PONG            = 0x1004,  // Keepalive response
    MSG_ERROR           = 0x1FFF,  // Generic error
};
```

---

## 4. Message Payloads

### 4.1 MSG_CONNECT (Client → Server)

**Purpose**: Register client with daemon, receive VDir mmap path.

```c
struct ConnectPayload {
    uint32_t client_pid;           // Client process ID
    uint32_t flags;                // Reserved (0)
    char     project_root[256];    // Absolute path to project
} __attribute__((packed));
```

### 4.2 MSG_CONNECT_ACK (Server → Client)

```c
struct ConnectAckPayload {
    uint32_t status;               // 0 = success
    uint32_t flags;                // Capability flags
    char     vdir_path[256];       // Path to VDir mmap file
    char     staging_base[256];    // Base path for staging files
} __attribute__((packed));

// Capability flags
#define CAP_REFLINK_SUPPORTED   (1 << 0)
#define CAP_HARDLINK_SUPPORTED  (1 << 1)
```

### 4.3 MSG_COMMIT (Client → Server)

**Purpose**: Request atomic ingestion of staged file.

```c
struct CommitPayload {
    uint32_t flags;                // See commit flags
    uint64_t file_size;            // Size in bytes
    int64_t  mtime_sec;            // Modification time (seconds)
    uint32_t mtime_nsec;           // Modification time (nanoseconds)
    uint32_t mode;                 // File mode (permissions)
    uint16_t virtual_path_len;     // Length of virtual path
    uint16_t staging_path_len;     // Length of staging path
    char     paths[];              // virtual_path + staging_path (concatenated)
} __attribute__((packed));

// Commit flags
#define COMMIT_FLAG_SYNC        (1 << 0)  // Wait for fsync
#define COMMIT_FLAG_REPLACE     (1 << 1)  // Replace existing file
#define COMMIT_FLAG_NEW         (1 << 2)  // Fail if exists
```

**Path Encoding**:
```
paths = virtual_path + '\0' + staging_path + '\0'
```

### 4.4 MSG_COMMIT_ACK (Server → Client)

```c
struct CommitAckPayload {
    uint32_t status;               // 0 = success
    uint8_t  cas_hash[32];         // BLAKE3 hash of content
    uint64_t generation;           // New VDir generation
} __attribute__((packed));
```

### 4.5 MSG_COMMIT_NACK (Server → Client)

```c
struct CommitNackPayload {
    uint32_t error_code;           // See error codes
    char     error_msg[256];       // Human-readable message
} __attribute__((packed));

// Error codes
#define ERR_FILE_NOT_FOUND      1
#define ERR_PERMISSION_DENIED   2
#define ERR_DISK_FULL           3
#define ERR_HASH_MISMATCH       4
#define ERR_INTERNAL            255
```

### 4.6 MSG_ABORT (Client → Server)

**Purpose**: Cancel in-progress write, cleanup staging file.

```c
struct AbortPayload {
    uint16_t virtual_path_len;
    uint16_t staging_path_len;
    char     paths[];              // virtual_path + staging_path
} __attribute__((packed));
```

### 4.7 MSG_PING / MSG_PONG

**Purpose**: Keepalive and connection health check.

```c
struct PingPayload {
    uint64_t timestamp_ns;         // Client timestamp (monotonic)
} __attribute__((packed));

struct PongPayload {
    uint64_t client_timestamp_ns;  // Echo back client timestamp
    uint64_t server_timestamp_ns;  // Server timestamp
    uint64_t generation;           // Current VDir generation
} __attribute__((packed));
```

---

## 5. Protocol Flow Examples

### 5.1 Successful Write Flow

```
InceptionLayer                          vdir_d
     |                                     |
     |--- MSG_CONNECT ------------------->|
     |<-- MSG_CONNECT_ACK ----------------|
     |                                     |
     |    [open() -> staging file]         |
     |    [write() -> staging file]        |
     |    [close() triggers commit]        |
     |                                     |
     |--- MSG_COMMIT -------------------->|
     |                                     |  [reflink staging → CAS]
     |                                     |  [update VDir]
     |                                     |  [clear dirty bit]
     |<-- MSG_COMMIT_ACK -----------------|
     |                                     |
     |    [unlink staging file]            |
     |                                     |
```

### 5.2 Failed Write Flow

```
InceptionLayer                          vdir_d
     |                                     |
     |--- MSG_COMMIT -------------------->|
     |                                     |  [staging file missing!]
     |<-- MSG_COMMIT_NACK ----------------|
     |    (ERR_FILE_NOT_FOUND)             |
     |                                     |
     |    [log error, return -EIO]         |
     |                                     |
```

### 5.3 Client Crash Detection

```
InceptionLayer                          vdir_d
     |                                     |
     |--- MSG_CONNECT ------------------->|
     |<-- MSG_CONNECT_ACK ----------------|
     |                                     |
     |    [CRASH! - socket closes]         |
     |                                     |
     X                                     |  [detect socket HUP]
                                           |  [find dirty files for PID]
                                           |  [clear dirty bits]
                                           |  [cleanup staging files]
```

---

## 6. Timeout and Retry Policy

| Operation | Timeout | Retry |
|-----------|---------|-------|
| Connect | 5s | 3 times |
| Commit | 30s | No retry (caller handles) |
| Ping | 10s | 3 times before disconnect |

**Reconnection**:
- If connection lost, InceptionLayer enters fallback mode
- Retries connection on next write operation
- Existing dirty files remain dirty until commit succeeds

---

## 7. Versioning

**Protocol Version**: 1

**Compatibility**:
- Server rejects clients with higher version (MSG_ERROR)
- Server accepts clients with lower version (backward compatible)
- Version bump required for breaking payload changes

---

## 8. Security Considerations

- Socket permissions: `0600` (owner-only)
- PID verification via `getsockopt(SO_PEERCRED)`
- No authentication beyond Unix permissions (single-user scope)

---

[End of Document]
