# Velo Rift

> **Modern runtimes are fast. Disks are not.**  
> Velo Rift removes the filesystem from the critical path of computation.

---

## What is Velo Rift?

Velo Rift is a **virtual file system layer** that makes dependency-heavy applications start instantly by eliminating disk I/O bottlenecks.

```text
Traditional:  open("/node_modules/...") → disk seek → read → parse
Velo Rift:    open("/node_modules/...") → mmap pointer → done
```

**Result**: Cold start in milliseconds, not minutes.

---

## Core Principles

| Principle | Meaning |
|-----------|---------|
| **Path = Content** | Same path → same bytes (content-addressed) |
| **Immutable Snapshots** | World state is frozen, verifiable, replayable |
| **Zero-Copy I/O** | mmap directly from CAS, no file extraction |

---

## What Velo Rift IS

- ✅ A **virtual file system** optimized for immutable content
- ✅ A **content-addressable store** with global deduplication  
- ✅ An **I/O accelerator** for Python, Node.js, Rust, and more
- ✅ A **state distribution layer** for reproducible execution

## What Velo Rift is NOT

- ❌ A runtime replacement (we accelerate Node.js, Python, Bun — not replace them)
- ❌ A package manager (we wrap uv, npm, cargo — not replace them)
- ❌ A build system (that's Bazel's job)
- ❌ A container runtime (that's Docker's job)
- ❌ A general-purpose filesystem for mutable data

**We do one thing well: make file access instant.**

---

## Use Cases

| Scenario | Without Velo | With Velo |
|----------|-------------|-----------|
| `npm install` (1000 packages) | 2 minutes | < 1 second |
| Python cold start | 500ms | 50ms |
| CI dependency restore | Pull from cache | Already there |
| Multi-tenant isolation | Copy per tenant | CoW overlay |

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
| [World Model Whitepaper](docs/velo-world-model-whitepaper.md) | Why Velo exists (philosophy) |
| [Technical Positioning](docs/architecture/velo-technical-positioning.md) | How we compare to other tools |
| [Technical Deep Dive](docs/architecture/velo-technical-deep-dive.md) | Implementation specification |

---

## Who Should Use Velo Rift

**Yes, if you:**
- Run large dependency trees (1000+ packages)
- Care about cold start latency (serverless, CI/CD)
- Want reproducible execution (auditing, compliance)
- Build AI agent infrastructure (deterministic replay)

**No, if you:**
- Need POSIX-perfect semantics
- Have write-heavy mutable workloads
- Your bottleneck is CPU, not I/O
- Have < 100 dependencies (overhead not worth it)

---

## Project Status

🚧 **Early Development** — Architecture defined, implementation in progress.

---

## License

Apache 2.0

---

> *"If software is becoming an autonomous, living entity,  
> it requires a world that does not shift beneath its feet."*
>
> **Velo Rift is that world.**
