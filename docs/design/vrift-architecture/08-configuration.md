# vrift Configuration Specification

This document defines all configurable parameters for vrift components.

---

## 1. Environment Variables

### 1.1 Client-Side (InceptionLayer)

| Variable | Type | Default | Description |
|----------|------|---------|-------------|
| `VRIFT_ENABLED` | bool | `1` | Enable/disable vrift interception |
| `VRIFT_PROJECT_ROOT` | path | auto-detect | Explicit project root path |
| `VRIFT_FALLBACK_MODE` | bool | `0` | Force fallback to real FS |
| `VRIFT_DEBUG` | int | `0` | Debug verbosity (0-3) |
| `VRIFT_STAGING_DIR` | path | `.vrift/staging` | Custom staging directory |
| `VRIFT_SOCKET_TIMEOUT_MS` | int | `5000` | UDS connect timeout |

**Auto-Detection Logic**:
When `VRIFT_PROJECT_ROOT` is not set:
1. Search upward from CWD for markers: `Cargo.toml`, `package.json`, `.git/`
2. Use first found directory as project root
3. Fall back to CWD if no marker found

### 1.2 Server-Side (vdir_d)

| Variable | Type | Default | Description |
|----------|------|---------|-------------|
| `VRIFT_VDIR_SIZE_MB` | int | `1024` | Max VDir mmap size |
| `VRIFT_CAS_PATH` | path | `~/.vrift/cas` | CAS storage location |
| `VRIFT_DB_PATH` | path | `~/.vrift/db` | LMDB database location |
| `VRIFT_SOCKET_PATH` | path | `~/.vrift/sockets` | UDS socket directory |
| `VRIFT_IDLE_TIMEOUT_SEC` | int | `3600` | Auto-shutdown after idle |
| `VRIFT_SYNC_INTERVAL_MS` | int | `100` | LMDB sync interval |
| `VRIFT_DURABILITY` | enum | `wal` | Durability mode |

**Durability Modes**:
- `sync`: Every commit fsyncs to LMDB (~1ms latency)
- `wal`: Write-ahead log + async LMDB (~200µs latency)
- `async`: No fsync, periodic sync (~150µs latency)

### 1.3 Coordinator (vriftd)

| Variable | Type | Default | Description |
|----------|------|---------|-------------|
| `VRIFT_MAX_PROJECTS` | int | `32` | Max concurrent projects |
| `VRIFT_GLOBAL_CAS_SIZE_GB` | int | `10` | Global CAS pool limit |
| `VRIFT_LOG_LEVEL` | enum | `info` | Log level |

---

## 2. Configuration File

**Location**: `~/.vrift/config.toml`

**Format**:
```toml
# Global settings
[global]
log_level = "info"            # debug, info, warn, error
max_projects = 32
cas_path = "~/.vrift/cas"
db_path = "~/.vrift/db"

# Per-project overrides (by project root hash prefix)
[projects."a8f9c1d2"]
durability = "sync"           # Override for this project
staging_dir = "/tmp/vrift-staging-a8f9"

# Performance tuning
[performance]
vdir_size_mb = 1024
sync_interval_ms = 100
idle_timeout_sec = 3600
reflink_enabled = true        # Auto-detected if not set

# Limits
[limits]
max_file_size_mb = 1024       # Skip files larger than this
max_files_per_project = 10000000
staging_quota_gb = 10
```

---

## 3. Directory Structure

```
~/.vrift/
├── config.toml               # Global configuration
├── cas/                      # Content-Addressable Storage
│   ├── metadata.db           # CAS index (LMDB)
│   └── blobs/
│       ├── ab/
│       │   └── cd12...       # Sharded by hash prefix
│       └── ...
├── db/                       # Per-project LMDB databases
│   ├── a8f9c1d2.../
│   │   ├── data.mdb
│   │   └── lock.mdb
│   └── ...
├── sockets/                  # UDS sockets
│   ├── a8f9c1d2.sock
│   └── ...
├── wal/                      # Write-ahead logs
│   ├── a8f9c1d2.wal
│   └── ...
└── logs/                     # Daemon logs
    ├── vriftd.log
    └── vdir_d.a8f9c1d2.log

<project>/.vrift/             # Per-project local directory
├── staging/                  # Staging files
│   ├── pid_1234/
│   │   └── fd_5_123456.tmp
│   └── ...
└── manifest.json             # Project metadata cache
```

---

## 4. Precedence Rules

Configuration is resolved in order (later overrides earlier):

1. **Compiled defaults** (lowest priority)
2. **Config file** (`~/.vrift/config.toml`)
3. **Environment variables**
4. **Command-line flags** (highest priority)

---

## 5. Runtime Queries

### Query Current Config (CLI)

```bash
# Show effective configuration
vrift config show

# Show specific value
vrift config get performance.sync_interval_ms

# Show project-specific config
vrift config show --project /path/to/project
```

### Query via IPC

Extend MSG_CONNECT_ACK to include config:

```c
struct ConnectAckPayload {
    // ... existing fields ...
    uint32_t sync_interval_ms;
    uint32_t durability_mode;
    uint32_t capabilities;
};
```

---

## 6. Validation Rules

| Parameter | Min | Max | Notes |
|-----------|-----|-----|-------|
| `vdir_size_mb` | 64 | 8192 | Power of 2 recommended |
| `sync_interval_ms` | 10 | 10000 | Lower = safer, slower |
| `idle_timeout_sec` | 60 | 86400 | 0 = never timeout |
| `max_file_size_mb` | 1 | 10240 | Large files bypass CAS |

---

## 7. Feature Flags

```toml
[features]
# Experimental features (default: disabled)
experimental_mmap_pool = false
experimental_batch_commit = false
experimental_prefetch = false

# Debugging features
trace_syscalls = false        # Log all intercepted syscalls
dump_vdir_on_crash = true     # Save VDir state on crash
```

---

[End of Document]
