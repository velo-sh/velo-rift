# Velo Rift™: Comprehensive Usage Guide

Velo Rift is a high-performance **data virtualization layer** designed for the AI-native era. It decouples "where a file lives" from "what a file contains," allowing you to run applications in virtualized environments with zero overhead.

---

## 🚀 Quick Start (Zero-Config)

The fastest way to experience Velo Rift™ is to just run your code. No manual ingestion or manifest setup required.

In any project directory (Python, Node.js, or Rust):
```bash
# Just run your command. Velo Rift™ will auto-detect your project.
vrift run -- python3 main.py
```
Velo Rift™ will perform a **Transient Ingest** on the fly, creating a temporary virtual view of your project and executing it immediately.

---

## 🛠 Step 1: Project Initialization

For professional projects, you may want a persistent configuration with custom filters (e.g., ignoring `node_modules` or `target/`).

```bash
# Run in your project root
vrift init
```
*   **What it does**: Detects your project type (Cargo, npm, Pip) and creates a `vrift.manifest`.
*   **Why use it**: It applies smart **LifeCode™ filters** to ensure only source code is virtualized, keeping your environment lean.

---

## 🏃 Step 2: Virtual Execution

Once you have a manifest (or even if you don't), use `vrift run` to execute code inside the **VeloVFS** layer.

### Basic Run
```bash
vrift run -- <command>
```

### Manual Manifest Selection
If you have multiple manifests (e.g., for different environment versions):
```bash
vrift run --manifest environments/stable.manifest -- ./deploy.sh
```

---

## 🛡 Step 3: Advanced Isolation (Linux Only)

For multi-tenant environments or security-critical tasks, Velo Rift™ supports **Rootless Isolation** using Linux Namespaces.

### Isolated Sandbox
```bash
vrift run --isolate -- python3 malicious_script.py
```

### Layered Manifests (Base Images)
You can stack manifests to create a layered environment (similar to Docker layers but without the performance penalty):
```bash
# Run app.manifest on top of a static busybox toolchain
vrift run --isolate --base busybox.manifest --manifest app.manifest -- /bin/sh
```

---

## 📊 Step 4: Maintenance & Optimization

### CAS Status & Monitoring

See global deduplication savings and project breakdown:

```bash
vrift status
```

**Example Output**:
```
VRift CAS Status:

  CAS Location: ~/.vrift/the_source
  Total Size:   1.48 GB
  Total Blobs:  115,363

  Registered Projects:
  ┌─────────────────────────────────────────────────────────────────┐
  │ Project        │ Files    │ Unique Blobs │ Shared │ Size       │
  ├─────────────────────────────────────────────────────────────────┤
  │ project1       │ 16,647   │ 13,783       │ 0      │ 222 MB     │
  │ project2       │ 23,948   │ 6,816        │ 6,967  │ +122 MB    │
  │ project3       │ 61,703   │ 30,947       │ 13,829 │ +365 MB    │
  └─────────────────────────────────────────────────────────────────┘

  Orphaned Blobs: 0 (run `vrift gc` to check)
```

### Garbage Collection

Clean up orphaned blobs that are no longer referenced by any manifest:

```bash
# Dry run (default): show what would be deleted
vrift gc

# Actually delete orphaned blobs (interactive confirmation)
vrift gc --delete

# Skip confirmation prompt (for scripts and CI)
vrift gc --delete --yes
# or short form:
vrift gc --delete -y

# Delete only orphans older than 2 hours (safest)
vrift gc --delete --older-than 2h

# Prune stale manifests (projects that were deleted)
vrift gc --prune-stale

# Full cleanup: prune stale + delete orphans
vrift gc --prune-stale --delete --yes
```

#### GC Options

| Option | Description |
|--------|-------------|
| `--delete` | Actually delete orphaned blobs (default is dry-run) |
| `--yes`, `-y` | Skip confirmation prompt (for automation) |
| `--prune-stale` | Remove stale manifest entries (source paths deleted) |
| `--older-than <DURATION>` | Only delete orphans older than this (e.g., "1h", "24h") |
| `--immediate` | Skip grace period and delete immediately |

#### GC Output Example

```
🗑️  VRift Garbage Collection
   CAS:     /tmp/demo/cas

  Registry Status:
    📁 Registered manifests: 2 (1 active, 1 stale)

  ⚠️  Stale Manifests (source path deleted):
      abc12345 - "/path/to/deleted/project"

  CAS Statistics:
    📦 Total blobs:   16,180 (245.44 MB)
    ✅ Referenced:    3,819
    🗑️  Orphaned:      12,362 (217.03 MB)
    💾 Reclaimable:   88.4% of CAS

╔════════════════════════════════════════╗
║  ✅ GC Complete in 1.22s              ║
╚════════════════════════════════════════╝

   🗑️  12,362 orphaned blobs deleted
   💾 217.03 MB reclaimed
   📉 CAS reduced by 88.4%
   ⚡ 10,117 blobs/sec
```

#### GC Demo

Experience the full GC lifecycle with a one-click demo:

```bash
./scripts/demo_gc.sh          # Full demo with npm install
./scripts/demo_gc.sh --skip-npm  # Skip npm install (faster)
```

The demo shows:
1. Ingest two projects (frontend + backend stacks)
2. Cross-project deduplication  
3. Stale manifest detection after project deletion
4. Orphan blob identification with size stats
5. Safe cleanup with progress bar
6. **Safety verification** - re-ingest proves no false deletions

### Health Check

Diagnose potential issues with the CAS and registry:

```bash
vrift doctor
```

**Example Output**:
```
VRift Doctor Report:

  ✅ Registry: valid (3 manifests)
  ⚠️  Stale manifests: 1 (run gc --prune-stale)
  ✅ CAS permissions: OK
  ✅ Disk space: 45 GB available
  ✅ Lock: not held

  Recommendations:
  - Run `vrift gc --prune-stale` to clean stale manifests
```

### Registry Management

Rebuild registry if corrupted or manifests lost:

```bash
# Rebuild registry from cached manifests
vrift registry --rebuild
```

### Full CAS Reset (Destructive)

For complete cleanup (e.g., fresh testing environment):

```bash
# Interactive confirmation required
vrift clean --all --force

# With permission fix (for hard-linked files)
vrift clean --all --force --fix-perms

# Non-interactive (for CI/CD)
vrift clean --all --force --yes
```

> ⚠️ **Warning**: `vrift clean --all` deletes the entire CAS. This is irreversible.

## 🧠 Under the Hood: Principles

### 1. Hash(Content) = Identity
In Velo, identity is tied to **Content**, not path. If 100 projects use the same `libpython.so`, Velo Rift stores only **one** copy in **TheSource** (CAS).

### 2. Two Inception Modes
Velo Rift offers two ways to enter the VFS "dream":
*   **The Shim (macOS/Linux)**: Uses `LD_PRELOAD` to intercept syscalls. Zero disk footprint. Best for local development.
*   **Link Farm (Linux Isolation)**: Creates a temporary directory of hardlinks. Best for containers and static binaries.

### 3. Inception Commands (macOS SIP Bypass)
On macOS, SIP restricts shim injection for system binaries. Use Inception Mode for full coverage:
```bash
# Enter the dream (sets up PATH + shim)
eval "$(vrift inception)"

# Run your build
cargo build

# Exit the dream
vrift wake
```

### 4. Absolute Determinism
A `vrift.manifest` uniquely defines an entire environment. If the manifest hash is the same, the execution outcome is guaranteed to be reproducible.

---

## ⚙️ Configuration Management

Velo Rift™ uses a layered configuration system with TOML files and environment variable overrides.

### Config Commands

```bash
# Initialize a local config file (.vrift/config.toml)
vrift config init

# Initialize global config (~/.vrift/config.toml)
vrift config init --global

# Show current effective configuration
vrift config show

# Show config file locations
vrift config path

# Validate config file syntax
vrift config validate
vrift config validate /path/to/config.toml  # validate specific file
```

### Configuration Hierarchy

Configuration is loaded in order (later overrides earlier):

1. **Built-in defaults** → Sensible defaults for all settings
2. **Global config** → `~/.vrift/config.toml`
3. **Project config** → `.vrift/config.toml` (in current directory)
4. **Environment variables** → `VR_*` and `VRIFT_*` prefixes

### Config File Locations

| Location | Path | Scope |
|----------|------|-------|
| Global | `~/.vrift/config.toml` | User-wide defaults |
| Project | `.vrift/config.toml` | Per-project overrides |

### Environment Variable Overrides

| Variable | Config Key | Example |
|----------|------------|---------|
| `VR_THE_SOURCE` | `storage.the_source` | `/data/shared-cas` |
| `VRIFT_THREADS` | `ingest.threads` | `8` |

### Example Config File

```toml
[storage]
the_source = "~/.vrift/the_source"
default_mode = "solid"  # or "phantom"

[ingest]
threads = 4
default_tier = "tier2"  # or "tier1"

[tiers]
tier1_patterns = ["node_modules", ".cargo/registry", "target/release"]
tier2_patterns = ["src", "lib", "tests"]

[security]
enabled = true
exclude_patterns = [".env", "*.key", ".aws"]

[daemon]
enabled = false
socket_path = "/run/vrift/daemon.sock"
```

---

## 📦 TheSource™ (CAS) Configuration

Velo Rift stores all deduplicated content in a **Content-Addressable Store (CAS)** called **TheSource™**.

### Global Parameters

| Parameter | Default | Description |
|-----------|---------|-------------|
| `--the-source-root` | `~/.vrift/the_source` | Global CAS directory |
| `--mode` | `solid` | Ingest mode: `solid` (hard_link) or `phantom` (rename) |
| `--tier` | `tier2` | Asset tier: `tier1` (immutable, symlink) or `tier2` (mutable, keep original) |
| `VR_THE_SOURCE` | (env var) | Override CAS path via environment variable |

### Default Behavior

By default, all projects share a **global CAS** for maximum deduplication:

```bash
# All projects use the same CAS
vrift ingest node_modules -o manifest.bin
# → CAS stored in: ~/.vrift/the_source/blake3/

# Second project with shared dependencies
cd ../another-project
vrift ingest node_modules -o manifest.bin
# → Shared files are deduplicated automatically!
```

### Custom CAS Location

Override the CAS location for isolated testing or CI/CD:

```bash
# Specify custom CAS root
vrift --the-source-root /tmp/test-cas ingest node_modules -o manifest.bin

# Or use environment variable
export VR_THE_SOURCE=/data/shared-cas
vrift ingest node_modules -o manifest.bin
```

### Recommended Usage by Scenario

| Scenario | CAS Location | Purpose |
|----------|--------------|---------|
| **Development** | `~/.vrift/the_source` (default) | Global dedup across all local projects |
| **CI/CD Pipeline** | `--the-source-root $CI_CACHE` | Ephemeral per-job, or shared cache for speed |
| **E2E Testing** | `mktemp -d` | Isolated test environment, avoid pollution |
| **Multi-tenant** | Per-user/team directory | Isolation between users/teams |

### CAS Directory Structure

```
~/.vrift/the_source/
└── blake3/                    # Hash algorithm directory
    ├── ab/                    # First 2 chars of hash (sharding)
    │   └── cd/                # Next 2 chars of hash
    │       ├── abcd1234...efgh_1024.bin    # blob: hash_size.bin
    │       └── abcd5678...ijkl_2048.bin
    └── ef/
        └── 12/
            └── ef123456...mnop_512.bin
```

Each blob is named with its full BLAKE3 hash and file size, ensuring content-addressable integrity.

---

## 🎯 Demo: Cross-Project Deduplication

Experience VRift's deduplication superpowers with a one-click demo:

```bash
# Full demo (fresh start + re-run)
./scripts/demo_dedup.sh

# Quick demo (xsmall + small only)
./scripts/demo_dedup.sh --quick

# Fresh start only (delete CAS first)
./scripts/demo_dedup.sh --fresh-only

# Re-run only (test warm CAS performance)
./scripts/demo_dedup.sh --rerun-only
```

### Expected Results

| Scenario | Description | Dedup Rate |
|----------|-------------|------------|
| **Fresh Start** | Small → Large order | 50-70% |
| **Re-Run** | Warm CAS | **100%** |

### Key Metrics

- **Speed**: 10,000+ files/sec
- **Dedup**: Up to 100% on re-run
- **Savings**: 50%+ on cross-project dependencies
