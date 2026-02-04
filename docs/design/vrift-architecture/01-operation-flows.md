# vrift: Operation Flows

This document details the step-by-step execution flow of key operations in vrift, from the perspective of the client (e.g., rustc) and the system.

---

## 1. Metadata Operations (`stat`)

**Goal**: Sub-100ns latency.

### Flow

1.  **Client**: `stat("src/main.rs")`
2.  **InceptionLayer**: Intercepts syscall.
3.  **InceptionLayer**: Queries VDir (Shared Memory Index).
    *   **Hit**: Returns `struct stat` from memory. (Latency: ~50ns)
    *   **Miss**: Falls back to real filesystem. (Latency: ~2µs)

**Architecture Note**: No IPC. No Lock (Wait-free read).

---

## 2. Read Operations (`open` + `read`)

**Goal**: Zero-Copy access to content.

### Flow

1.  **Client**: `open("src/main.rs", O_RDONLY)`
2.  **InceptionLayer**: Intercepts.
3.  **InceptionLayer**: Lookups VDir.
    *   **Metadata**: Gets CAS Hash `abcd...`.
4.  **InceptionLayer**: Checks L2 CAS Pool (Shared Memory).
    *   **Hit**: Returns pointer to shared memory blob.
    *   **Miss**: Maps L3 CAS File (`~/.vrift/cas/abcd...`).
5.  **Client**: `read(fd, buf, len)`
6.  **InceptionLayer**: `memcpy` from CAS ptr to `buf`.

---

## 3. Write Operations (`write`)

**Goal**: Non-blocking ingestion.

### Flow

1.  **Client**: `open("target/main.o", O_WRONLY)`
    *   InceptionLayer buffers metadata.
2.  **Client**: `write(fd, buf)`
    *   InceptionLayer buffers data in process memory.
### 2.3 `close(fd)`
1.  **InceptionLayer**: Atomic Store `ring.flags = EOF` (Release).
2.  **InceptionLayer**: `futex_wake` (Conditional).
3.  **Return**: Returns `0` instantly. **Zero Syscall**.
    (Optimistic).

### Server Background Flow

1.  **Server**: Receives IPC.
2.  **Server**: Computes Hash (BLAKE3).
3.  **Server**: Writes to CAS (Dedup).
4.  **Server**: Updates VDir.

---

[End of original document structure]
