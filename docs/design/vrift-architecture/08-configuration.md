# vrift Configuration Specification

This document defines all configurable parameters for vrift components, matching the implementation in `vrift-config`.

---

## 1. Configuration Loading Order

Configuration is resolved in order (later overrides earlier):

1. **Compiled defaults** (lowest priority)
2. **Global config**: `~/.vrift/config.toml`
3. **Project config**: `.vrift/config.toml` (overrides global)
4. **Environment variables** (highest priority)

---

## 2. Configuration Structure

```toml
# ~/.vrift/config.toml

[storage]
the_source = "~/.vrift/the_source"  # CAS root directory
default_mode = "solid"               # solid | phantom

[ingest]
threads = null                       # null = auto-detect CPU count
default_tier = "tier2"               # tier1 | tier2 | auto

[tiers]
tier1_patterns = [                   # Immutable (dependencies)
    "node_modules/",
    ".cargo/registry/",
    ".rustup/",
    "/toolchains/",
    ".venv/lib/",
    "site-packages/",
    "/usr/lib/",
    "/usr/share/",
]
tier2_patterns = [                   # Mutable (build outputs)
    "target/",
    "target/debug/",
    "target/release/",
    "dist/",
    "build/",
    ".next/",
    "__pycache__/",
    ".pytest_cache/",
    ".cache/",
    "out/",
]

[security]
enabled = true
exclude_patterns = [                 # Sensitive files (never ingest)
    ".env",
    ".env.*",
    "*.key",
    "*.pem",
    "*.p12",
    "*.pfx",
    "id_rsa",
    "id_rsa.*",
    "id_ed25519",
    "id_ed25519.*",
    "*.keystore",
    "credentials.json",
    "secrets.yaml",
    "secrets.yml",
]

[daemon]
socket = "/run/vrift/daemon.sock"    # UDS socket path
enabled = false                      # Enable daemon mode
```

---

## 3. Configuration Sections

### [storage] - CAS Storage Settings

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `the_source` | path | `~/.vrift/the_source` | TheSource™ CAS root directory |
| `default_mode` | string | `solid` | Default projection mode: `solid` or `phantom` |

### [ingest] - Ingestion Settings

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `threads` | int? | null (auto) | Parallel ingestion threads |
| `default_tier` | string | `tier2` | Default tier classification |

### [tiers] - Tier Classification

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `tier1_patterns` | string[] | (see above) | Immutable dependency patterns |
| `tier2_patterns` | string[] | (see above) | Mutable build output patterns |

**Tier Definitions**:
- **Tier 1** (Immutable): Dependencies that rarely change. Read-optimized, aggressive caching.
- **Tier 2** (Mutable): Build outputs that change frequently. Write-optimized, quick invalidation.

### [security] - Security Filter

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `enabled` | bool | `true` | Enable security filter |
| `exclude_patterns` | string[] | (see above) | Patterns to exclude from VFS |

### [daemon] - Daemon Settings

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `socket` | path | `/run/vrift/daemon.sock` | UDS socket path |
| `enabled` | bool | `false` | Enable daemon mode |

---

## 4. Environment Variables

| Variable | Config Override | Description |
|----------|-----------------|-------------|
| `VR_THE_SOURCE` | `storage.the_source` | TheSource™ CAS root directory |
| `VRIFT_THREADS` | `ingest.threads` | Parallel thread count |
| `VRIFT_PROJECT_ROOT` | - | Override project root discovery |
| `VRIFT_MANIFEST` | - | Direct manifest path (shim/daemon) |
| `VRIFT_VFS_PREFIX` | - | VFS mount point prefix (shim) |
| `VRIFT_DEBUG` | - | Enable debug logging (shim) |

**Example**:
```bash
export VR_THE_SOURCE="/fast-ssd/vrift-cas"
export VRIFT_THREADS=16
```

> [!NOTE]
> **Migration**: Legacy code may use `VRIFT_CAS_ROOT`. This is deprecated.
> All components should read `VR_THE_SOURCE` as the canonical variable.
> For backward compatibility, components MAY fall back to `VRIFT_CAS_ROOT` if `VR_THE_SOURCE` is unset.

---

## 5. Project-Local Override

Projects can override global settings with `.vrift/config.toml`:

```toml
# /path/to/project/.vrift/config.toml

[tiers]
tier1_patterns = ["vendor/"]  # Project-specific patterns

[security]
exclude_patterns = [".env.local"]  # Additional exclusions
```

**Merge Behavior**:
- Non-empty arrays **replace** (not merge) global values
- Empty arrays preserve global values

---

## 6. Directory Structure

```
~/.vrift/
├── config.toml              # Global configuration
├── the_source/              # TheSource™ CAS
│   ├── objects/             # Content blobs
│   │   ├── ab/
│   │   │   └── cd1234...    # Sharded by hash prefix
│   │   └── ...
│   └── refs/                # Reference tracking
└── sockets/                 # Future: per-project sockets

/tmp/
├── vrift.sock               # Current: single daemon socket
└── vrift-manifest.mmap      # Shared memory manifest

<project>/
└── .vrift/
    ├── config.toml          # Project-local config
    └── staging/             # Future: staging area
```

---

## 7. Programmatic Access

```rust
use vrift_config::{config, reload, Config};

// Read current config (global singleton)
let cfg = config();
println!("CAS path: {:?}", cfg.storage.the_source);
println!("Default tier: {}", cfg.ingest.default_tier);

// Reload from disk
reload()?;

// Generate default TOML
let toml_str = Config::default_toml();
```

---

## 8. Validation

The configuration module does **not** validate:
- Path existence (created on demand)
- Pattern syntax (glob patterns accepted as-is)
- Thread count limits (OS enforced)

Invalid TOML syntax will cause load failure with `ConfigError::Toml`.

---

## 9. Future Configuration (v3 Architecture)

The following settings are planned for the Staging Area architecture:

```toml
# Future additions (not yet implemented)

[performance]
vdir_size_mb = 1024
sync_interval_ms = 100
idle_timeout_sec = 3600

[limits]
max_file_size_mb = 1024
max_files_per_project = 10000000
staging_quota_gb = 10

[features]
experimental_staging_area = false
experimental_dirty_bit = false
```

---

## 10. Path Resolution Rules

All paths in vrift follow consistent resolution rules:

### 10.1 Canonical Path Format

| Context | Format | Example |
|---------|--------|---------|
| Manifest storage | Leading `/`, relative to project root | `/src/main.rs` |
| Syscall input | Any (shim normalizes) | `./src/main.rs`, `src/main.rs` |
| IPC communication | Absolute or manifest-relative | `/src/main.rs` |
| Config files | Absolute or `~` expanded | `~/.vrift/the_source` |

### 10.2 Path Normalization

All components MUST normalize paths before use:

```rust
fn normalize_path(path: &str, project_root: &Path) -> Result<String, PathError> {
    // 1. Expand ~ to $HOME
    // 2. Resolve relative paths against project_root
    // 3. Collapse //, /./, /../
    // 4. Strip trailing /
    // 5. Ensure leading / for manifest keys
}
```

### 10.3 Project Root Discovery

Project root is discovered in order:

1. `VRIFT_PROJECT_ROOT` environment variable
2. Nearest ancestor containing `.vrift/` directory
3. Current working directory (fallback)

### 10.4 Derived Paths

From project root, all other paths are derived:

| Path | Derivation |
|------|------------|
| Manifest | `{project_root}/.vrift/manifest.lmdb` |
| Mmap cache | `{project_root}/.vrift/manifest.mmap` |
| Local config | `{project_root}/.vrift/config.toml` |
| Staging | `{project_root}/.vrift/staging/` |

---

## 11. Error Messages

vrift provides clear, actionable error messages:

### 11.1 Configuration Errors

| Error Code | Message | Solution |
|------------|---------|----------|
| `E001` | `VFS prefix not set` | Set `VRIFT_VFS_PREFIX` or use `vrift init` |
| `E002` | `Manifest not found` | Run `vrift ingest` or check `VRIFT_MANIFEST` |
| `E003` | `CAS root not accessible` | Check `VR_THE_SOURCE` path permissions |
| `E004` | `Project root outside VFS prefix` | Verify path alignment |

### 11.2 Error Format

```
⚠️  vrift Error [E002]: Manifest not found

The manifest file could not be loaded:
  Path: /path/to/project/.vrift/manifest.lmdb
  
This usually means:
  1. The project has not been ingested yet
  2. The VRIFT_MANIFEST path is incorrect
  
To fix, run:
  vrift ingest /path/to/project

Or set the correct path:
  export VRIFT_MANIFEST=/path/to/.vrift/manifest.lmdb
```

---

## 12. Troubleshooting

### 12.1 Diagnostic Command

```bash
vrift doctor
```

Output:
```
vrift Doctor - Configuration Diagnostics

✅ Global config:     ~/.vrift/config.toml
✅ CAS root:          ~/.vrift/the_source (42.3 GB, 15,234 blobs)
✅ Daemon socket:     /tmp/vrift.sock (connected)
⚠️  Project config:   Not found (using global)
❌ Manifest:          /work/proj/.vrift/manifest.lmdb (MISSING)

Recommendations:
  1. Run `vrift ingest .` to create manifest
```

### 12.2 Common Issues

#### "Files: 0" in VFS benchmark
**Cause**: Path format mismatch between shim queries and manifest storage.
**Fix**: Ensure paths are normalized with leading `/` before manifest lookup.

#### "Failed to open mmap file"
**Cause**: Daemon didn't export mmap, or path mismatch.
**Fix**: 
1. Check daemon logs for mmap export message
2. Verify `VRIFT_MANIFEST` points to correct location
3. Run warm-up command to trigger workspace registration

#### "Permission denied" on CAS files
**Cause**: macOS immutable flag (`uchg`) on CAS blobs.
**Fix**: `chflags -R nouchg ~/.vrift/the_source`

### 12.3 Debug Environment Variables

| Variable | Purpose |
|----------|---------|
| `VRIFT_DEBUG=1` | Enable verbose shim logging |
| `VRIFT_NO_MMAP=1` | Disable mmap, use IPC only |
| `VRIFT_TRACE=1` | Full syscall trace (very verbose) |

---

[End of Document]
