# Deep Dive: InceptionLayer <-> vdir_d Interaction Protocol

## 1. Design Goal
**Maximize Throughput, Minimize Latency.**
*   **Throughput**: Limited only by memory bandwidth (`memcpy`).
*   **Latency**: Non-blocking for Producer (InceptionLayer) as long as buffer isn't full.
*   **Safety**: Crash-safe. Consumer (`vdir_d`) failure must not crash Producer.

---

## 2. Shared Memory RingBuffer Layout

Each active file write session (`open` -> `write`... -> `close`) allocates a dedicated **SPSC (Single Producer Single Consumer) RingBuffer** in Shared Memory.

### Memory Structure
```c
struct RingBuffer {
    // Cache Line 1: Producer Write State (Written by InceptionLayer)
    alignas(64) atomic_uint64_t write_head; 
    
    // Cache Line 2: Consumer Read State  (Written by vdir_d)
    alignas(64) atomic_uint64_t read_tail;
    
    // Cache Line 3: Control Flags
    alignas(64) atomic_uint32_t flags; // EOF, ERROR, PAUSE
    uint32_t capacity;                 // Power of 2 (e.g., 4MB)
    
    // Data Area (Aligned to Page Size)
    alignas(4096) uint8_t data[];      
};
```
**Why alignment?** To prevent **False Sharing**. Producer only updates `write_head`, Consumer only updates `read_tail`. They sit on different CPU Cache Lines.

---

## 3. Interaction Flow (The "Fast Loop")

### Phase 1: Handshake (Connection)
1.  **InceptionLayer**: Connects to `vdir_d` Unix Socket.
    *   `SEND: { cmd: OPEN, path: "target/main.o" }`
2.  **vdir_d**: 
    *   Allocates a 4MB RingBuffer (Memfd).
    *   `SEND: { status: OK, memfd: 123, capacity: 4MB }` (Passes FD via SCM_RIGHTS).
3.  **InceptionLayer**: `mmap` the RingBuffer.
    *   *Result*: Both processes have mapped the *same* physical RAM.

### Phase 2: Streaming Write (The Hot Path)
**Producer (InceptionLayer) Algorithm**:
```rust
fn write(buf: &[u8]) {
    while buf.len() > 0 {
        // 1. Calculate available space
        head = ring.write_head.load(Relaxed);
        tail = ring.read_tail.load(Acquire); // Sync with consumer
        available = capacity - (head - tail);
        
        if available == 0 {
            // Buffer Full! Must wait (Backpressure)
            futex_wait(&ring.read_tail, tail);
            continue;
        }

        // 2. Write Data (memcpy)
        chunk = min(buf.len(), available);
        memcpy(&ring.data[head % capacity], buf, chunk);
        
        // 3. Commit Write (Release semantics)
        // Consumer can NOT see data until this store completes
        ring.write_head.store(head + chunk, Release);
        
        // 4. Signal Consumer (Lazy)
        // Only wake if Consumer is sleeping or we crossed a threshold (e.g. 16KB)
        if chunk > WAKE_THRESHOLD || waiting_flag {
            futex_wake(&ring.write_head);
        }
        
        buf = buf[chunk..];
    }
}
```

**Consumer (vdir_d) Algorithm**:
```rust
fn ingest_loop() {
    loop {
        // 1. Check for data
        tail = ring.read_tail.load(Relaxed);
        head = ring.write_head.load(Acquire); // Sync with Producer
        
        if head == tail {
            if flag == EOF { break; }
            // Empty! Sleep.
            futex_wait(&ring.write_head, head);
            continue;
        }
        
        // 2. Process Data (Zero Copy Read)
        data_slice = &ring.data[tail % capacity .. head % capacity];
        
        // PARALLEL JOB: Hashing + Compression
        hasher.update(data_slice);
        
        // 3. Release Space
        ring.read_tail.store(head, Release);
        
        // 4. Notify Producer (if it was blocked)
        futex_wake(&ring.read_tail);
    }
}
```

### Phase 3: Completion (In-Band EOF)
1.  **InceptionLayer**: Atomic Store `ring.flags = EOF` (Release).
2.  **InceptionLayer**: `futex_wake(&ring.write_head)` (Only if Consumer sleeping).
3.  **InceptionLayer**: Returns `0` instantly. **Zero Syscall** close.
    *   *Note*: No Unix Socket `CLOSE` command is needed. The ring buffer state handles termination.

---

## 4. Critical Design Choices

### A. Backpressure Strategy
*   **What if Consumer is slow?** (CPU load high)
*   **Behavior**: Producer (Compiler) **BLOCKS**.
*   **Rationale**: This is standard Kernel behavior (Pipe/Socket buffer full). Prevents OOM. Compiler pauses naturally, giving `vdir_d` CPU time to catch up. Self-regulating system.

### B. Why RingBuffer > Pipe?
*   **Pipe**: Needs Syscall (`write` + `read`) and usually involves kernel-internal copy (or page flipping cost).
*   **Shared RingBuffer**: 
    *   **Zero Syscall** in non-full/non-empty case.
    *   **User-Space Memcpy**.
    *   Modern CPUs optimized for memcpy.

### C. Signaling (EventFD vs Futex)
*   **Futex**: Fastest. User-space check first, syscall only if contention.
*   **Decision**: Use **Futex** on the `head`/`tail` atomic variables directly.

---

## 5. Performance Envelop
*   **Throughput**: 10GB/s+ (Memory Bandwidth saturation).
*   **Latency**: < 1µs (Cache coherency latency).
*   **IPC Overhead**: Only when buffer fills/empties. For a 100MB file and 4MB buffer, we signal ~25 times. Negligible.
