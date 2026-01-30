# Velo Rift: Comprehensive Usage Guide

Velo Rift is a high-performance **data virtualization layer** designed for the AI-native era. Unlike traditional filesystems or containers, Velo decouples "where a file lives" from "what a file contains."

---

## 1. Core Principles & Why Velo?

### 🧠 The Core Equation: `Hash(Content) = Identity`
In a traditional filesystem, path identity is tied to location. In Velo, identity is tied to **Content**.

*   **Global Deduplication (CAS)**: If 100 projects use the same `libpython.so`, Velo stores only **one** physical copy on disk.
*   **Zero-Copy Virtualization**: We don't "copy" environments; we "project" them. Whether using links or syscall interception, we provide a virtual view of data that is physically shared.
*   **Absolute Determinism**: A Velo Manifest (`.velo`) uniquely defines an entire environment. If the manifest hash is the same, the execution outcome is guaranteed to be reproducible across any machine.

---

## 2. Two Ways Velo Runs Your Code

Velo provides two distinct mechanisms to "project" your virtual environment into the real world.

### A. Link Farm (The Physical Mirage) —— **Recommended for Isolation**
Velo creates a temporary directory structure filled with **Hard Links** back to the global CAS store.
-   **When to use**: Rootless Linux containers, sandboxed execution, and when running statically linked binaries (Go, Rust).
-   **Pros**: 100% Native performance (Kernel handles it), compatible with all languages, enables OverlayFS layering.
-   **Cons**: Minor overhead of creating file inodes in a temporary path.

### B. The Shim Interception (The Invisible Mirage) —— **Recommended for Local Dev**
Velo uses `LD_PRELOAD` to inject a small library (`velo-shim`) into your process, intercepting filesystem calls like `open()` and `stat()`.
-   **When to use**: MacOS development, rapid local testing, and when you want **zero** physical traces on the disk.
-   **Pros**: Instantaneous (no directory to create), no disk footprint.
-   **Cons**: Only works for dynamically linked programs; slight overhead on every syscall.

---

## 3. Usage Modes & Scenarios

### Mode 1: Local Development Acceleration
**Scenario**: You have 10 separate AI projects sharing 90% of their dependencies (PyTorch, Transformers).
-   **Action**: Ingest shared dependencies once.
-   **Benefit**: Save 10x disk space; environments start in milliseconds.
-   **Command**:
    ```bash
    # Run locally with Shim interception
    velo run --manifest project.velo -- python main.py
    ```

### Mode 2: Secure Sandbox (Multi-Tenant)
**Scenario**: Running untrusted code or a multi-user build platform.
-   **Action**: Use the Linux Namespace sandbox and project an isolated rootfs.
-   **Benefit**: Strong security (Rootless/Unprivileged), private modifiable layers (OverlayFS).
-   **Command**:
    ```bash
    # Prepare base tools (one time)
    ./scripts/setup_busybox.sh
    # Run in isolated sandbox (Mode A)
    velo run --isolate --base busybox.manifest --manifest app.velo -- /bin/sh -c "id -u"
    ```

### Mode 3: Reproducible CI/CD
**Scenario**: You need to ensure the test environment on the CI runner is *exactly* the same as your local machine.
-   **Action**: Pass the Manifest file between nodes.
-   **Benefit**: "Hash-verified" consistency. No more "it works on my machine" bugs.
-   **Command**:
    ```bash
    velo run --manifest release_v1.0.velo -- ./test_suite.sh
    ```

---

## 4. Comparison Summary

| Feature | **Link Farm (Mode A)** | **Shim Interception (Mode B)** |
| :--- | :--- | :--- |
| **Sandbox Support** | Full (OverlayFS Compatible) | Minimal |
| **Binary Compatibility** | **All binaries** (Static & Dynamic) | Dynamic only (No Go/Static Rust) |
| **Performance** | Native | ~1-2% Syscall Overhead |
| **Setup Cost** | Low (O(N) Link Creation) | **Zero (O(1))** |
| **Primary Platform** | Linux (for isolation) | macOS / Linux |

---

## 5. Standard Workflow

### 1. Ingestion
Take any folder and make it "Velo-native."
```bash
velo ingest ./my_project --output my.velo
```

### 2. Execution
Run it using your preferred mode as described above.

### 3. Maintenance
Check your savings and clean up old versions.
```bash
velo status    # Monitor CAS health and dedup ratio
velo gc --run  # Cleanup orphaned blobs
```
