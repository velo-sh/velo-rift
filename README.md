# Velo Rift

> **Modern runtimes are fast. Disks are not.**  
> Velo Rift removes the filesystem from the critical path of computation.

---

## What is Velo Rift?

Velo Rift is a **virtual file system layer** that solves two problems:

1. **Read-only file access is too slow** → mmap from content-addressable storage
2. **Duplicate files waste storage** → global deduplication

```text
Traditional:  open("/node_modules/...") → disk seek → read → copy
Velo Rift:    open("/node_modules/...") → mmap pointer → done
```

**Result**: Cold start in milliseconds, not minutes.

---

## What Velo Rift IS

- ✅ A **virtual file system** for read-only content
- ✅ A **content-addressable store** with global deduplication  
- ✅ An **I/O accelerator** for Python, Node.js, Rust, and more

## What Velo Rift is NOT

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

## Quick Start

```bash
# Install (coming soon)
curl -fsSL https://velo.dev/install.sh | sh

# Accelerate your project
cd my-project
velo init
velo run npm start
```

---

## Documentation

| Document | Description |
|----------|-------------|
| [Comparison](docs/COMPARISON.md) | How we compare to other tools |
| [Architecture](docs/ARCHITECTURE.md) | Implementation specification |

---

## Who Should Use Velo Rift

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

🚧 **Early Development** — Architecture defined, implementation in progress.

## License

Apache 2.0
