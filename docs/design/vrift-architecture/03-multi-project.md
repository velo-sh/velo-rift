# vrift Multi-Project Architecture

## Project Identification (Single-User, Multi-Project)

### Simplified Project ID

**Scope**: Single machine, single user, multiple independent projects

**Project ID = hash(absolute_project_root)**

```c
char* generate_project_id(const char *project_root) {
    // 1. Resolve to absolute canonical path
    char abs_path[PATH_MAX];
    realpath(project_root, abs_path);
    
    // 2. Hash to fixed length (BLAKE3 for consistency)
    uint8_t hash[32];
    blake3(abs_path, strlen(abs_path), hash);
    
    // 3. Encode first 16 bytes to hex (32 chars)
    char *project_id = malloc(33);
    for (int i = 0; i < 16; i++) {
        sprintf(project_id + i*2, "%02x", hash[i]);
    }
    project_id[32] = '\0';
    
    return project_id;
}
```

**Examples**:
```
Project: /home/user/my-rust-app
  → realpath: /home/user/my-rust-app
  → hash: blake3("/home/user/my-rust-app")
  → project_id: "a8f9c1d2e3f4a5b6c7d8e9f0a1b2c3d4"
  
Project: /home/user/my-c-app  
  → project_id: "fedcba0987654321fedcba0987654321"
  
Same project (always same ID):
  → Deterministic, reproducible
```

**Key properties**:
- **Deterministic**: Same path always generates same ID
- **Unique**: Different paths generate different IDs (collision probability: 2^-128)
- **Simple**: No git, no branch, just path

---

## VDir File Naming

### Shared Memory Files

```
/dev/shm/
  ├─ vrift_vdir_a8f9c1d2e3f4a5b6  (/home/user/my-rust-app)
  ├─ vrift_vdir_fedcba0987654321  (/home/user/my-c-app)
  ├─ vrift_vdir_1234567890abcdef  (/home/user/my-go-service)
  └─ vrift_cas/                   (shared CAS pool, all projects)
      ├─ metadata.db
      └─ blobs/
          ├─ abc123...
          └─ def456...
```

**Naming scheme**: `vrift_vdir_<first_16_hex_of_project_id>`

**Properties**:
- **Isolation**: Each project directory has its own VDir file
- **Sharing**: All projects use same CAS pool (deduplication!)
- **Single-user**: No permission isolation needed (same uid)

---

## Server Management

### Server State

```c
struct VriftServer {
    // Map: project_id → VDir instance
    HashMap<String, VDirHandle*> vdirs;
    
    // Shared CAS pool (all projects)
    CasPool *cas_pool;
    
    // Mutex for vdirs map updates
    pthread_mutex_t vdir_map_lock;
};

struct VDirHandle {
    char *project_id;
    char *project_root;
    char *mmap_path;           // "/dev/shm/vrift_vdir_..."
    void *mmap_ptr;
    size_t mmap_size;
    
    uint64_t generation;
    uint64_t last_access_ns;   // For cleanup
    uint32_t active_clients;   // Reference count
};
```

### VDir Lifecycle

#### 1. VDir Creation (On-Demand)

```c
VDirHandle* server_get_or_create_vdir(const char *project_root) {
    char *project_id = generate_project_id(project_root);
    
    // 1. Check if already exists
    pthread_mutex_lock(&server.vdir_map_lock);
    VDirHandle *vdir = hashmap_get(&server.vdirs, project_id);
    if (vdir) {
        vdir->active_clients++;
        pthread_mutex_unlock(&server.vdir_map_lock);
        return vdir;
    }
    
    // 2. Create new VDir
    vdir = malloc(sizeof(VDirHandle));
    vdir->project_id = project_id;
    vdir->project_root = strdup(project_root);
    
    // 3. Create mmap file
    char mmap_path[256];
    snprintf(mmap_path, sizeof(mmap_path), 
             "/dev/shm/vrift_vdir_%.16s", project_id);
    
    // 4. Initialize VDir (scan project files)
    vdir_init(vdir, mmap_path, project_root);
    
    // 5. Register in map
    hashmap_set(&server.vdirs, project_id, vdir);
    vdir->active_clients = 1;
    pthread_mutex_unlock(&server.vdir_map_lock);
    
    printf("[Server] Created VDir for project %s\n", project_root);
    return vdir;
}
```

**Time**: First access ~100ms (scan files), subsequent instant (cached)

#### 2. VDir Update (File Changes)

```c
void server_handle_write_complete(const char *project_root, 
                                   const char *path,
                                   uint8_t cas_hash[32]) {
    VDirHandle *vdir = server_get_vdir(project_root);
    if (!vdir) return;
    
    // Update this project's VDir only
    server_update_vdir(vdir, path, cas_hash, ...);
    
    // Other projects' VDirs unaffected
}
```

#### 3. VDir Cleanup (Idle Timeout)

```c
// Background thread
void server_cleanup_idle_vdirs() {
    uint64_t now = get_time_ns();
    
    pthread_mutex_lock(&server.vdir_map_lock);
    
    for (VDirHandle *vdir : server.vdirs.values()) {
        // Conditions for cleanup:
        // 1. No active clients
        // 2. Idle for > 1 hour
        if (vdir->active_clients == 0 && 
            (now - vdir->last_access_ns) > 3600e9) {
            
            printf("[Server] Cleaning up VDir %s\n", vdir->project_id);
            
            // Unmap and delete file
            munmap(vdir->mmap_ptr, vdir->mmap_size);
            unlink(vdir->mmap_path);
            
            // Remove from map
            hashmap_remove(&server.vdirs, vdir->project_id);
            free(vdir);
        }
    }
    
    pthread_mutex_unlock(&server.vdir_map_lock);
}
```

**Frequency**: Every 10 minutes

---

## Client Discovery

### How Client Finds VDir

**Method 1: Environment Variable (Explicit)**

```bash
# Set by build system or user
export VRIFT_PROJECT_ROOT=/home/user/my-rust-app
export VRIFT_PROJECT_BRANCH=main

# Client reads env vars
rustc src/main.rs
  ↓
  libvrift.dylib constructor:
    root = getenv("VRIFT_PROJECT_ROOT")
    branch = getenv("VRIFT_PROJECT_BRANCH")
    project_id = generate_project_id(root, branch)
    vdir_path = "/dev/shm/vrift_vdir_<project_id>"
    mmap(vdir_path)
```

**Method 2: Auto-Detection (Implicit)**

```c
// Client constructor
void vrift_client_init() {
    // 1. Get current working directory
    char cwd[PATH_MAX];
    getcwd(cwd, sizeof(cwd));
    
    // 2. Search upward for project markers
    char project_root[PATH_MAX];
    if (find_project_root(cwd, project_root)) {
        // Found Cargo.toml, package.json, etc.
        project_id = generate_project_id(project_root);
    } else {
        // Fallback: use cwd
        project_id = generate_project_id(cwd);
    }
    
    // 3. Build VDir path
    snprintf(vdir_path, sizeof(vdir_path),
             "/dev/shm/vrift_vdir_%.16s", project_id);
    
    // 4. Check if exists, if not request server to create
    if (access(vdir_path, R_OK) != 0) {
        ipc_request_vdir_creation(project_root);
        // Wait for server to create (async)
    }
    
    // 5. mmap VDir
    client->vdir_mmap = mmap_vdir(vdir_path);
}
```

### Client-Server Handshake

```c
// Client → Server IPC
struct VDirRequest {
    char project_root[PATH_MAX];
    pid_t client_pid;
};

// Server → Client response
struct VDirResponse {
    char vdir_path[256];     // "/dev/shm/vrift_vdir_..."
    uint64_t generation;
    bool ready;
};
```

---

## Multi-Project Scenario

### Example Setup

```
Machine state:
  ├─ Project A: /home/user/backend (rustc, cargo)
  │   └─ VDir: /dev/shm/vrift_vdir_aaaa...
  │
  ├─ Project B: /home/user/frontend (npm, webpack)
  │   └─ VDir: /dev/shm/vrift_vdir_bbbb...
  │
  └─ Project C: /home/user/backend (branch: feature-x)
      └─ VDir: /dev/shm/vrift_vdir_cccc...
```

### Concurrent Builds

```
Timeline:
T0: User starts "cargo build" in Project A
  ├─ rustc processes spawn
  ├─ libvrift detects project_id=aaaa...
  ├─ mmap /dev/shm/vrift_vdir_aaaa...
  └─ All rustc processes share VDir A

T1: User starts "npm run build" in Project B
  ├─ webpack workers spawn
  ├─ libvrift detects project_id=bbbb...
  ├─ mmap /dev/shm/vrift_vdir_bbbb...
  └─ All webpack processes share VDir B

T2: User starts "cargo build" in Project C
  ├─ rustc processes spawn
  ├─ libvrift detects project_id=cccc...
  ├─ mmap /dev/shm/vrift_vdir_cccc...
  └─ All rustc processes share VDir C

Server state:
  vdirs map:
    aaaa... → VDirHandle (active_clients=8, rustc processes)
    bbbb... → VDirHandle (active_clients=4, webpack workers)
    cccc... → VDirHandle (active_clients=8, rustc processes)
    
  cas_pool:
    Shared by all projects!
    ├─ stdlib.a → used by both Project A and C (deduplicated!)
    └─ common.o → used by all projects (1 physical copy)
```

### Memory Usage

**Without vrift**:
- Project A: 2GB (artifacts in RAM)
- Project B: 1GB
- Project C: 2GB
- **Total: 5GB**

**With vrift**:
- VDir A: 60MB (metadata only)
- VDir B: 40MB
- VDir C: 60MB
- CAS pool: 2.5GB (shared, deduplicated)
- **Total: 2.66GB** (53% savings!)

---

## Resource Management

### VDir Size Limits

```c
// Per-project VDir constraints
#define MAX_VDIR_SIZE (1 * 1024 * 1024 * 1024)  // 1GB
#define MAX_FILES_PER_PROJECT (10 * 1000 * 1000)  // 10M files

// Global constraints
#define MAX_ACTIVE_VDIRS 32
#define TOTAL_VDIR_MEMORY_LIMIT (8 * 1024 * 1024 * 1024)  // 8GB
```

### Eviction Policy

When global limit reached:
```c
void server_evict_lru_vdir() {
    // Sort VDirs by last_access_ns
    // Evict oldest with active_clients == 0
    // Keep at least most recent N VDirs
}
```

---

## Cross-Project Deduplication

### CAS Pool Sharing

**Key insight**: Same file content across projects → single blob in CAS

```
Project A compiles:
  src/utils.rs → utils.o (hash: abc123...)
  → CAS: write abc123... (5MB)

Project B compiles:
  src/utils.rs → utils.o (SAME CONTENT!)
  → CAS: dedup check → abc123... exists → skip write!
  → VDir B: points to abc123...

Physical memory:
  CAS pool: 5MB (1 copy)
  VDir A + VDir B: point to same blob

Result: 2 projects, 1 physical copy of utils.o
```

**Dedup rate**: 30-50% for typical monorepos with shared dependencies

---

## Implementation Checklist

Server side:
- [ ] HashMap for project_id → VDirHandle
- [ ] On-demand VDir creation
- [ ] Idle VDir cleanup thread
- [ ] Global memory limit enforcement

Client side:
- [ ] Project root auto-detection (git, Cargo.toml, etc.)
- [ ] Project ID generation (deterministic hash)
- [ ] VDir path resolution
- [ ] Fallback to server IPC if VDir missing

---

## Summary

**Multi-project isolation**:
- Each project+branch → unique VDir
- Deterministic project_id generation
- Files: `/dev/shm/vrift_vdir_<project_id>`

**Resource sharing**:
- All projects share single CAS pool
- Automatic deduplication (30-50% savings)
- Physical memory = VDirs (metadata) + CAS (unique blobs)

**Lifecycle**:
- VDir created on-demand (first client access)
- Persists while clients active
- Cleaned up after 1 hour idle

**Scalability**: O(projects) VDirs, O(unique_files) CAS blobs
