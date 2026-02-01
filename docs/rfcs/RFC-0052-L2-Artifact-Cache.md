# RFC-0052: L2 Compilation Artifact Cache

## Vision
> **Change one line of code, build completes instantly. Share compilation results across the team, eliminate all redundant CPU work.**

## The 10x Promise
| Layer | What it caches | Speedup |
| :--- | :--- | :---: |
| L1 (Current) | Source code (node_modules, .cargo) | 1-2x |
| **L2 (This RFC)** | **Compilation artifacts (.o, .rlib, binary)** | **10x-100x** |

---

## Core Mechanism

### Content-Addressed Build Cache
```
Artifact Key = SHA256(
    source_files_content +
    Cargo.toml +
    RUSTFLAGS +
    rustc_version +
    target_triple
)

Artifact Value = {
    .rlib, .rmeta, .o, .d files
}
```

### Flow
```
┌────────────────────────────────────────────────────────┐
│  Developer A: cargo build (first time)                 │
│    └── Compile crate foo → Store in L2 CAS             │
│                                                        │
│  Developer B: cargo build (same crate, same inputs)    │
│    └── L2 HIT → Download .rlib → Skip compilation      │
│    └── Build time: 120s → 3s                           │
│                                                        │
│  CI: cargo build (same branch)                         │
│    └── L2 HIT → 0 compilation → Deploy in seconds      │
└────────────────────────────────────────────────────────┘
```

---

## Architecture

### Local + Remote Hybrid
```
┌─────────────────────────────────────────────────────────────┐
│  vrift-shim (intercepts rustc/gcc)                          │
│       │                                                     │
│       ▼                                                     │
│  L2 Local Cache (~/.vrift/l2/)                              │
│       │ miss                                                │
│       ▼                                                     │
│  L2 Remote Cache (S3 / LAN Server / Team NAS)               │
│       │ miss                                                │
│       ▼                                                     │
│  Actually compile → Push result to L2                       │
└─────────────────────────────────────────────────────────────┘
```

### Configuration
```toml
# velo.toml
[l2_cache]
enabled = true
local_path = "~/.vrift/l2"
max_local_size = "50GB"

# Team sharing
remote_url = "s3://company-build-cache/velo-l2"
# or
remote_url = "http://build-cache.internal:8080"

# What to cache
cache_targets = ["debug", "release"]
cache_extensions = [".rlib", ".rmeta", ".o", ".a"]
```

---

## The `--live` Synergy

`--live` mode + L2 Cache = **Ultimate Speed**

| Without L2 | With L2 |
| :--- | :--- |
| Change 1 file → Recompile crate + all dependents | Change 1 file → Recompile **only that crate** |
| Incremental compile: 30s | Incremental compile: **<1s** |
| CI cold start: 5min | CI cold start: **30s** (all from cache) |

---

## Performance Projections

### Single Developer
| Scenario | Before | After | Speedup |
| :--- | :---: | :---: | :---: |
| First build | 120s | 120s | 1x |
| Rebuild same commit | 120s | 5s | **24x** |
| Rollback to old commit | 120s | 0s | **∞** |

### Team (10 developers, shared L2)
| Scenario | Before | After | Speedup |
| :--- | :---: | :---: | :---: |
| 10 devs build same branch | 10 × 120s | 1 × 120s + 9 × 5s | **13x** |
| Daily CI (50 runs) | 50 × 120s | 1 × 120s + 49 × 5s | **17x** |

### Enterprise (100+ developers, high-speed LAN)
| Scenario | Before | After | Speedup |
| :--- | :---: | :---: | :---: |
| Monorepo full build | 30min | 2min | **15x** |
| Feature branch build | 30min | 30s | **60x** |

---

## Implementation Phases

### Phase 1: Local L2 (MVP)
- [ ] Intercept compiler output
- [ ] Calculate artifact hash
- [ ] Store/retrieve from local cache
- **Timeline**: 2 weeks

### Phase 2: Remote L2
- [ ] S3-compatible backend
- [ ] LAN discovery (mDNS)
- [ ] Concurrent download/upload
- **Timeline**: 2 weeks

### Phase 3: Smart Warming
- [ ] Predict next-needed artifacts
- [ ] Background pre-fetch from remote
- [ ] Priority queue for hot crates
- **Timeline**: 1 week

---

## The Ultimate Promise

> **Change one file → Press Enter → Build complete**

This is the true meaning of `--live` mode:
- L1: Sources in memory → No IO wait
- L2: Artifacts cached → No CPU wait
- Result: **Build time ≈ Network latency** (LAN: 1-5ms)
