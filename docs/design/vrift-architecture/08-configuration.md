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
| `VR_THE_SOURCE` | `storage.the_source` | CAS root directory |
| `VRIFT_THREADS` | `ingest.threads` | Parallel thread count |

**Example**:
```bash
export VR_THE_SOURCE="/fast-ssd/vrift-cas"
export VRIFT_THREADS=16
```

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

[End of Document]
