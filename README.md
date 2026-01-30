# Velo Rift™

> **Modern runtimes are fast. Disks are not.**  
> Velo Rift™ removes the filesystem from the critical path of computation.

---

## What is Velo Rift™?

Velo Rift™ is a **virtual file system layer** (powered by VeloVFS) that solves two problems:

1. **Read-only file access is too slow** → mmap from content-addressable storage
2. **Duplicate files waste storage** → global deduplication

```text
Traditional:  open("/node_modules/...") → disk seek → read → copy
Velo Rift™:   open("/node_modules/...") → mmap pointer → done
```

**Result**: Cold start in milliseconds, not minutes.

---

## What Velo Rift™ IS

- ✅ A **virtual file system** for read-only content
- ✅ A **content-addressable store** with global deduplication  
- ✅ An **I/O accelerator** for Python, Node.js, Rust, and more

## What Velo Rift™ is NOT

- ❌ A runtime replacement (we accelerate existing runtimes)
- ❌ A package manager (we wrap uv, npm, cargo)
- ❌ A build system (that's Bazel's job)
- ❌ A container runtime (that's Docker's job)
- ❌ A general-purpose filesystem for mutable data

---

## Results

| Metric | Before | After |
|--------|--------|-------|
| `npm install` (1000 packages) | 2 minutes | < 1 second |
| Python cold start | 500ms | 50ms |
| Disk usage (10 projects) | 10 GB | 1 GB |

---

## 🚀 Quick Start (Local)

1. **Build**: `cargo build --release`
2. **Ingest**: `vrift™ ingest ./path/to/folder --output app.velo`
3. **Run**: `vrift™ run --manifest app.velo -- ls -R`

For more advanced scenarios, see the [Full Usage Guide](docs/USAGE.md).

---

## 🛠 Usage Modes

Velo Rift™ supports three primary execution modes depending on your needs:

### 1. Local Development (Mode B: Library Interception)
Uses `LD_PRELOAD` to transparently virtualize files without creating physical links.
```bash
vrift™ run --manifest app.velo -- python main.py
```

### 2. High-Performance Sharing (Mode A: Link Farm)
Instantly creates a directory of hard links back to the global CAS. The default for non-SANDBOX Linux tasks.
```bash
# Default behavior for standard runs
vrift™ run --manifest app.velo -- ./my_binary
```

### 3. Secure Isolation (Mode A + Sandboxing)
Creates a rootless Linux Namespace container with a layered rootfs (Multi-manifest support).
```bash
./scripts/setup_busybox.sh
vrift™ run --isolate --base busybox.manifest --manifest app.velo -- /bin/sh
```

## ⚡️ Performance & Benchmarking

We use **Criterion** for high-precision benchmarking.

Run CAS micro-benchmarks:

```bash
cargo bench -p velo-cas
```

This measures the nanosecond-latency of `store`, `get`, and `get_mmap` operations.

---

## Documentation

| Document | Description |
|----------|-------------|
| [Usage Guide](docs/USAGE.md) | **Start Here!** Multi-mode execution guide |
| [Comparison](docs/COMPARISON.md) | How we compare to other tools |
| [Architecture](docs/ARCHITECTURE.md) | Implementation specification |

---

## Who Should Use Velo Rift™

**Yes:**
- Large dependency trees (1000+ packages)
- Cold start latency matters (serverless, CI/CD)
- Multi-tenant workloads

**No:**
- POSIX-perfect semantics required
- Write-heavy mutable workloads
- Bottleneck is CPU, not I/O

---

## Status

� **Active Development** — Core architecture implemented. Stable CAS, VFS Shim, and Rootless Isolation (Linux) are ready for use.

## License

Apache 2.0
