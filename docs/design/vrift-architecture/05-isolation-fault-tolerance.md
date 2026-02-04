# vrift Isolation and Fault Tolerance

## Design Principle

### Isolation Level: Process (The Gold Standard)

Our architecture uses **Per-Project Process Isolation**:

1.  **Project A**: Has its own `vdir_d` process (PID 100).
2.  **Project B**: Has its own `vdir_d` process (PID 101).
3.  **InceptionLayer A**: Communicates ONLY with PID 100.

**Scenario**: Protocol Error in Project A **InceptionLayer** forces `vdir_d` (PID 100) to panic.
*   **Result**: Project A builds stop.
*   **Project B**: `vdir_d` (PID 101) continues running unscathed.
*   **Central**: `vriftd` continues running.

This is superior to shared-server isolation because memory corruption in one project's handler cannot bleed into another.

---

## Isolation Dimensions

### 1. Process Isolation (Natural)
(Unchanged)

### 2. VDir Isolation (Architecture)
(Unchanged)

### 3. Data Plane Isolation: The "Infinite" Per-Project Pool

**The Architecture**:
- Each Project (ID) gets a dedicated Shared Memory File (`/dev/shm/vrift_data_<id>`).
- **Size**: Functionally Infinite (e.g., 1TB Sparse Mapping).
- **Isolation**: 
    - Client of Project A **ONLY** mmaps `vrift_data_A`.
    - It physically **CANNOT** address or corrupt Project B's memory.
    - OS enforces this boundary via Page Tables.

**Why it works**:
- **Sparse Files**: On Linux/macOS, `ftruncate(fd, 1TB)` creates a sparse file. It consumes 0 physical RAM.
- **On-Demand Paging**: Physical RAM is only allocated when a Client touches a page.
- **Server Role**:
    - Server maps ALL project pools (A, B, C...).
    - Server acts as the **Hypervisor**, managing lifecycle and checking quotas.

#### Safety Mechanisms

**A. Crash Containment**
- If Client A crashes mid-write:
    - It corrupts a page in `vrift_data_A`.
    - **Project B is 100% safe**.
    - Server detects crash -> Punches hole (FALLOC_FL_PUNCH_HOLE) in `vrift_data_A` to release physical RAM -> Reports build failure for A.

**B. Quota Enforcement**
- Although Virtual Space is infinite, Physical RAM is finite.
- **Server-Side Watchdog**: Monitors RSS (Resident Set Size) of each pool file.
- **Limit**: If Project A uses > 4GB RAM, Server kills Project A's build (or forces spill to disk).

**C. No Global Locks**
- Each Pool has its own independent Free List / Slab Metadata.
- Lock contention is strictly intra-project. Project A's locking never stalls Project B.

---

## Conclusion

By giving each project its own "Infinite Virtual Playground", we delegate isolation back to the **Hardware (MMU)** and **OS Kernel**.

- **Isolation Level**: Process/VM Level (Highest).
- **Complexity**: Low (OS manages mapping).
- **Safety**: Robust.

[Rest of fault scenarios apply unchanged]
