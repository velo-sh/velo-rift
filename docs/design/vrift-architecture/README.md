# vrift Architecture Design Documents

This directory contains detailed design documents for the vrift virtual filesystem architecture.

## Document Index

### 01. Operation Flows
**File**: `01-operation-flows.md`  
**Focus**: Step-by-step execution flow from compiler perspective

Covers:
- Complete stat/open/read/write/close scenarios
- Exact latency breakdown at each step
- Memory addresses and state transitions
- Performance comparison with real filesystem

### 02. VDir Synchronization
**File**: `02-vdir-synchronization.md`  
**Focus**: Real-time sync between server and clients via shared memory

Covers:
- Shared memory + atomic generation counter mechanism
- Server publish protocol (memory_order_release)
- Client synchronization protocol (memory_order_acquire)
- Memory ordering guarantees and performance impact

### 03. Multi-Project Support
**File**: `03-multi-project.md`  
**Focus**: Single-user multi-project isolation and resource sharing

Covers:
- Project identification (path-based hashing)
- VDir file isolation per project
- Server management of multiple VDirs
- Cross-project deduplication via shared CAS

### 04. LMDB Persistence
**File**: `04-lmdb-persistence.md`  
**Focus**: Two-tier storage model and write ordering strategies

Covers:
- Hot tier (mmap) vs cold tier (LMDB)
- Three write strategies (sync LMDB, WAL, async LMDB)
- Crash recovery and data consistency guarantees
- Performance analysis and recommendations

### 05. Isolation and Fault Tolerance
**File**: `05-isolation-fault-tolerance.md`  
**Focus**: Production-grade fault isolation and resilience

Covers:
- Four isolation dimensions (process, VDir, server, CAS)
- Four failure scenarios (client crash, server crash, corruption, exit)
- Concurrency control and resource limits
- Monitoring and health checks

## Recommended Reading Order

1. **Start here**: RFC-0054 (main architecture overview)
2. **Operation flows** (understand runtime behavior)
3. **VDir sync** (understand memory synchronization)
4. **Multi-project** (understand isolation model)
5. **LMDB persistence** (understand durability)
6. **Fault tolerance** (understand production stability)

## Related Documents

- `docs/rfcs/RFC-0054-vrift-Architecture.md` - Main architecture RFC
- `docs/rfcs/RFC-0044-PSFS-Stat-Acceleration.md` - PSFS constraints
- `docs/rfcs/RFC-0051-Acceleration-Strategy.md` - Overall strategy
