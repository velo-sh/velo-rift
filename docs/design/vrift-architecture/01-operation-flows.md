# vrift: Operation Flows

This document details the step-by-step execution flow of key operations in vrift, from the perspective of the client (e.g., rustc) and the system.

---

## 1. Metadata Operations (`stat`)

**Goal**: Sub-200ns latency.

### Layered Consistency Model

vrift uses a **3-Layer Read Check** to maintain correctness:

| Layer | Check | Latency | Condition |
|-------|-------|---------|-----------|
| L1 | Dirty Bit / Staging | Native | File being written |
| L2 | VDir Index (SHM) | ~50ns | Clean, indexed file |
| L3 | Real FS Passthrough | ~2µs | Not managed by vrift |

### Detailed Flow

1. **Client**: `stat("src/main.rs")`
2. **InceptionLayer**: Intercepts syscall via shim.
3. **L1 Check (Dirty/Staging)**:
   - If file is `DIRTY` or exists in `.vrift/staging/`:
   - → Returns `stat` of **Real Staging Path**.
4. **L2 Check (VDir Index)**:
   - Queries Shared Memory Index.
   - **Hit**: Returns `struct stat` from memory. (Latency: ~50ns)
5. **L3 Fallback (Real FS)**:
   - **Miss**: Executes real syscall on underlying FS. (Latency: ~2µs)

**Total**: ~170ns (intercept + L2 lookup + return) for clean indexed files.

---

## 2. Read Operations (`open` + `read`)

**Goal**: Zero-Copy access to content.

### Detailed Flow

1. **Client**: `open("src/main.rs", O_RDONLY)`
2. **InceptionLayer**: Intercepts.
3. **VDir Lookup**: Gets CAS Hash `abcd...`.
4. **CAS Pool Check**:
   - **L2 Hit (SHM)**: Returns pointer to shared memory blob.
   - **L3 Miss**: Maps CAS File from `~/.vrift/cas/abcd...`.
5. **Client**: `read(fd, buf, len)`
6. **InceptionLayer**: `memcpy` from CAS ptr to `buf`.

**Total**: ~300ns for 4KB read from L2.

---

## 3. Write Operations (Staging Area Model)

**Goal**: Native-speed writes with atomic ingestion.

### Architecture Note

Write path uses **Staging Area** model (not RingBuffer):
- **Data Plane**: Local Staging File (Native FS speed).
- **Control Plane**: UDS Commit on `close()`.

### 3.1 Open & Mark Dirty

1. **Client**: `open("main.o", O_WRONLY)`
2. **InceptionLayer**:
   - **Mark Dirty**: Updates shared state to mark "main.o" as `DIRTY`.
   - **Redirect**: Returns FD to `.vrift/staging/<pid>/<fd>_<ts>.tmp`.
3. **Return**: Real FD to temp file. **Zero IPC**.

**Latency**: ~3µs (local syscall)

### 3.2 Native Write

1. **Client**: `write(fd, buf, size)`
2. **Kernel**: Data goes directly to OS Page Cache.
3. **No IPC**: InceptionLayer is not involved.

**Latency**: ~10ns per write (Page Cache throughput)

### 3.3 Close & Commit

1. **Client**: `close(fd)`
2. **InceptionLayer**: Sends UDS Command:
   ```
   CMD_COMMIT { virtual_path: "main.o", staging_path: "..." }
   ```
3. **vdir_d**:
   - **Ingest**: `ioctl(FICLONERANGE)` or `link()` to CAS.
   - **Update**: Updates VDir Index, clears `DIRTY` flag.
   - **Ack**: Returns status.
4. **InceptionLayer**: Unlinks staging file (cleanup).
5. **Return**: Returns `0` to client.

**Latency**: ~20µs (UDS round-trip)

---

## 4. Latency Summary

| Operation | Component | Latency |
|-----------|-----------|---------|
| `stat()` | Full path (intercept + L2 + return) | ~170ns |
| `stat()` | VDir lookup only | ~50ns |
| `read(4KB)` | From L2 CAS Pool | ~300ns |
| `write()` | To Page Cache | ~10ns |
| `open()` | Staging redirect | ~3µs |
| `close()` | UDS Commit | ~20µs |

> **Note**: All write-path latencies are amortized by Native FS speed.
> Compile artifacts (average 4-64KB) complete in ~30µs total.

---

## 5. Consistency Guarantee

The **Dirty Bit** ensures strong consistency:

```
Writer Process A              Reader Process B
─────────────────             ─────────────────
open() → Mark DIRTY
write() → Staging File
                              stat() → Sees DIRTY
                              → Reads from Staging (Real Path)
close() → Commit
         Clear DIRTY
                              stat() → Sees CLEAN
                              → Reads from VDir Index (Fast)
```

**Guarantee**: Modifications are visible immediately to all processes.

---

[End of Document]
